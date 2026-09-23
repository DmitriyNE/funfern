# Engineering log

Low-effort working notes: what changed, what was checked, TODOs, open issues, and
next steps. Short bullets are enough; no entry is required for every tiny edit.
Keep current actions near the top and dated entries newest first. Durable decisions
belong in [architecture.md](architecture.md) and milestone scope in [plan.md](plan.md).

## 2026-09-23 — The solver was allowed to set the frame time

Reported: selecting a non-stationary material takes the frame rate from 120 to
65. The reply that this is the tighter trajectory step asking for more steps a
frame was the mechanism but not the defect - frame rate should not be a function
of solver throughput at all. At 13.85 steps a frame there is always a new
configuration to draw, so nothing justifies the display waiting.

- The measurement, from the reported numbers at 8827 dofs: steps a frame are
  `750/120 = 6.25` and `900/65 = 13.85`, and fitting `frame = R + steps x S` to
  both points gives one per-step cost, `S = 0.93 ms`, `R = 2.5 ms`. So a driven
  step costs what a fixed one costs; the whole difference is how many of them a
  frame asks for.
- `steps_for_frame` capped by accumulated simulated time and by
  `MAX_STEPS_PER_FRAME`, and `steps_with_gpu_backpressure` by outstanding lead.
  Nothing capped by what the batch would cost, and the compute shares the
  frame's queue with drawing, so the batch set the frame time. `pacing.rs`
  already stated the contract - preserve display service, report the shortfall -
  without enforcing it.
- `frame_step_budget` is multiplicative backoff and additive recovery on the
  batch ceiling, which needs no estimate of what a step costs and converges on
  the largest batch that still ships frames at the cadence.
- The target is measured, not assumed: 60, 120 and 144 Hz want different
  budgets. `hold_cadence` holds the best frame lately and relaxes upward, the
  mirror of `hold_rate`.
- The subtle constant is the backoff. It is also the only thing that ever
  produces a frame faster than the last, so it is how the cadence estimate
  learns what the display can do. Backing off just enough to stop overrunning
  leaves a saturated solver defining its own slowness as normal: at `0.9` the
  estimate stalls at 9.02 ms and holds 111 fps, where `0.75` finds 8.33 ms and
  holds 116. The tolerance is the band the controller settles on the edge of, so
  it is frame rate given away directly - `1.25` would hold 96.
- Simulated from the reported operating point: 65 fps at 0.56x speed becomes
  116 fps at 0.44x. That is the trade asked for, and the shortfall is already
  measured and reported.
- The backlog cap moved from `MAX_STEPS_PER_FRAME` to the budget. Holding a full
  ceiling of backlog while the budget is small would saturate every following
  frame catching up, which is the opposite of the contract.
- Why a step costs 0.93 ms at all is a separate question, written up in
  [the step throughput spike](spikes/funfern-step-throughput-spike.md): 141
  dispatches a step, almost all of them the outgoing trace sweep, which is
  launch-latency bound rather than work bound. Two levers costed there, neither
  taken yet.

## 2026-09-23 — A dense inverse nobody read, built one column at a time

Reported: after a material change the status sits at "Ready for GPU upload" for
a long time, and it scales with the mesh. That phase covers packing and the
device upload, so the pack was timed at two mesh sizes, in release.

- Every millisecond of it was one call. Packing the plan is 15-19 ms and the
  transfer machinery is 1-2 ms; constructing the state was 165 ms at 206 trace
  nodes and 613 ms at 288.
- `prepare_static` inverted the dense trace system by calling `solve_dense` once
  per column, each call cloning and re-factorizing the whole matrix. That is
  `O(trace^4)`. The 206 to 288 ratio predicts `(288/206)^4 = 3.8x`; the measured
  ratio was 3.7. Trace count grows as the square root of the mesh, so this was
  quadratic in the degrees of freedom - the scaling that was reported.
- `invert_dense` eliminates once and carries the identity along: 613 ms to 33 ms.
- Then the second half. The export a backend consumes is mass-free - diagonal,
  modal corrections, sweep count, eliminated modes - and never the inverse. A
  state built only to be compiled into a device plan was paying a dense
  inversion that was then discarded. `for_backend` skips it, and the three pack
  sites and `compile_temporal` use it: 33 ms to 0.4 ms.
- At 41k dofs, end to end: state construction 613 ms to 0.4 ms, a linear pack
  600 ms to 22 ms, a driven pack 1273 ms to 118 ms.
- `a_backend_state_exports_what_a_stepping_state_exports` is the licence for the
  second half: if the export could tell the two constructions apart, skipping
  the inversion would be wrong. It also steps both and compares.
- Left standing: a driven pack still rebuilds the whole source plan (about 95 ms
  of `attach_temporal_bulk`) to read back drive signatures and material ids that
  the mapping consumes in 1 ms. That is the next lever, and it is a plumbing
  change rather than an algorithmic one.
- Also found, unrelated to speed: `update_temporal_canonical_point_probes`,
  `_vector_overlay`, `_curve_probes` and `_area_probes` are never called. The
  viewport always takes the fixed variants, so probes and the vector overlay on
  a driven medium reconstruct from authored rather than instantaneous
  coefficients. Wrong numbers, not slow ones; not yet addressed.

## 2026-09-23 — The refusal named itself and the bug fell out in one run

Finishing the sweep left `WaveError::InvalidCoefficients` with exactly one
production site: the reference P1 `WaveOperator::assemble`, the one check the
shared sentence actually describes, and one nothing the application prepares can
reach. Everything else says which check it was.

- The last six were the thin-gap orientation, two grid-filter strengths, the
  operator's eigenvalue bound, the two grid-scale filter strengths, and three
  combined guards in `canonical_temporal` that were split so each clause carries
  its own words instead of five conditions sharing one.
- That immediately answered the reported failure. Packing an upload rebuilds the
  *source* plan to read its drives back, and does it at the **candidate's** time
  step. A driven generation runs at the tighter step its coefficient trajectory
  demands, so going pump to linear the candidate's step is the larger of the
  two and the pump will not hold it: `CanonicalTemporalWaveState::zero` refuses
  with "the time step exceeds what the time-driven trajectory holds stable" and
  the whole upload dies. Nothing is wrong with either generation or with the
  handoff maps - they just do not share a step, and only the source has to.
- It is direction-asymmetric, which matches the report: enabling a drive tightens
  the step, so the source always holds it; disabling one loosens it.
- The existing both-directions test hid this by packing every direction at the
  pump's step, the one step that always works. `undriving_packs_at_the_step_the
  _host_actually_uses` packs at the step the host computes, and fails.
- Fixed by packing the source plan at the *source's* own step with its own
  clock. Nothing read back from that plan depends on a step - node and sample
  counts, record counts, drive signatures, material identities - so the step
  there is free, and the source's own is the one it is guaranteed to hold.
  Measured on the reported document: linear candidate `2.5355e-3` against a pump
  ceiling of `2.5198e-3`, overrunning it by 0.6%.
- Swept for the sibling shape. `solver_time_step`'s fallback named the base
  operator's step, which for a driven generation is not the step the solver
  runs; it now reads the generation's own. Reachable only before the first
  upload, but it is the same mistake and the third time this family has bitten.
- The both-directions test now packs each direction at the candidate's step,
  which is what the host computes, instead of at one generation's step for all
  of them. A single shared step is exactly the case that always works.
- Three rounds of guessing, then one run once the message was specific. The cost
  of a shared error variant is paid entirely by whoever is holding the report.

## 2026-09-23 — One sentence for forty different refusals

A reported preparation failure - "wave coefficients must be finite with positive
mass and stiffness and nonnegative damping" - could not be reproduced across
three attempts, because that sentence is `WaveError::InvalidCoefficients`, a
unit variant that around forty unrelated checks return. A source whose weights
miss their support reads as a bad material.

- Reproducing the reported sequence exactly - load the inert document, apply the
  pump, return to linear, driving the real `request_runtime` at the reported
  mesh edge - passes at every step. So the document and the request path are not
  it, and guessing which check fired was never going to converge.
- `WaveError::Unsupported(&'static str)` names a refusal where it is found.
  Every one reachable while a generation is prepared now carries its own words:
  a source signal or rate anchor that is not a valid waveform, a source driving
  a node outside its support, prescribed data that does not match the
  generation, a volume source that does not, the wrong number of quadrature
  samples, a driven medium's instantaneous mass or loss rate, an element naming
  a region the scene does not hold, a region naming a missing material, and one
  material holding two runtime records.
- With the three named earlier - the law a fixed assembly cannot execute, the
  axis ratio, and the assembled nodal mass - the message a user sees now
  identifies the check rather than a category.
- The lesson is about diagnosis rather than physics. Three rounds went into
  guessing which of forty checks a user had hit, when one mechanical change made
  the question answer itself. A shared unit error variant is a diagnostic debt
  that is only paid by whoever is holding the report.

## 2026-09-23 — What the solver is told to do now survives a restart

Reported while trying to run an experiment: adaptation cannot be turned off for
a session, because its settings are not kept. The control exists, but every
launch resets it, so "start without adaptation" was not expressible - which is
also what blocked reproducing the report it was needed for.

- The adaptation controls lived on the UI struct alone. They move into the
  document's presentation, beside `simulation_speed`, which already worked that
  way: `mesh_edge`, the `adaptation` block - enabled, accuracy, elements per
  wavelength, minimum and maximum edge - and `grid_scale_filter`.
- Moved rather than mirrored. A working copy synchronised against the document
  would be two truths to keep in step, so the call sites read the document and
  the duplicate fields are gone.
- Every stored key carries a serde default taken from the real default, so a
  file written before today opens with what the application used to start from
  rather than being refused. The existing older-file test covers all seven.
- `the_solver_settings_survive_a_round_trip` says what it is for: a session left
  with adaptation off must open with it off.
- Two judgement calls. `mesh_edge` and `grid_scale_filter` were not asked for,
  and are included because they are the other settings that change what the
  solver does rather than what is drawn - a reopened document meshing itself at
  a different resolution is the same surprise as a changed coefficient. And
  presentation travels with a share link, so these now travel with one too;
  that seems right for a shared scene, but it does hand over the sender's
  adaptation settings.
- Verification: `cargo fmt --all`, `cargo clippy --workspace --all-targets
  --locked -- -D warnings`, `cargo test --workspace --locked`.

## 2026-09-23 — The device held the assumption the host had just dropped

Reported: loading the autosave and switching to a parametric pump on epsilon
fails with "layout or transfer consistency check failed" - a device status code,
so the shader rejected the transfer at runtime.

- The transfer shader carried the same assumption lifted from the host an entry
  ago: it refused unless *both* generations held a runtime bank, testing
  `material_header.y == 0 || material_header.z == 0 ||
  old_control.runtime_slots.w == 0 || new_control.runtime_slots.w == 0` - the
  two record counts and each side's temporal-enabled flag. Enabling a drive
  produces `(0, N)`, which is exactly what the host now builds.
- A side is now only required to be a temporal generation when it claims
  records. With none at the source every target record is written from its own
  authored anchors; with none at the target there is nothing to write.
- The lesson is the one the session keeps repeating from the other direction:
  the host and the device hold the same contract in two places, and relaxing it
  on one side without grepping the other leaves a rejection that only a real run
  finds. This is the second time in a day.
- **No device gate covers a handoff that changes drivenness.** It has broken
  twice and a user found it both times, because `canonical_gpu_driven_document`
  builds a single generation and never hands off. Extending it to go inert to
  driven and back is the missing check.

## 2026-09-23 — Copy never reached the clipboard

- `bevy_egui` is declared `default-features = false` with `render` and
  `default_fonts`, and `manage_clipboard` - the feature that pulls in `arboard`
  and hands egui's copied text to the operating system - is in its default set
  but was not enabled. `Context::copy_text` filled a buffer nothing read, so
  every Copy button was inert on native.
- The share-link copy already carried a `web_sys` fallback, which is why the
  browser path worked and the native one never did; that fallback is the
  evidence the gap had been half-noticed before.
- Both of the feature's dependencies are target-gated upstream - `arboard`
  excludes wasm, `smithay-clipboard` is Linux only - so the browser build is
  unaffected, which `cargo check --target wasm32-unknown-unknown` confirms
  rather than assumes.

## 2026-09-23 — A refusal that could not be told from a bad number

Reported: switching a driven material to Linear failed with "wave coefficients
must be finite with positive mass and stiffness and nonnegative damping".

- Not reproducible from what the document holds. Both saved states - the driven
  one and the one after the switch - prepare cleanly, and the switch itself
  applied correctly: the accepted scene is linear and the preset's parameters
  were retired. The adapted preparation path is not implicated either; it
  refuses a changed document and takes the ordinary path instead.
- What is wrong regardless is the message. `linear_material_sample` returns
  `WaveError::InvalidCoefficients` both for a coefficient that is genuinely not
  positive and finite *and* for a material carrying an authored law the fixed
  assembly cannot execute. Those read identically in the application, so a
  perfectly good pump reports as a bad number and the report cannot say which
  material or which row.
- The law refusal now returns `MaterialEvaluation`, naming the material, the row
  and the sample point, and saying that a driven generation assembles from the
  stripped model instead. The same ambiguity cost a diagnosis earlier today,
  when a probe stencil mapped a law refusal onto its own "cannot use the current
  mesh".
- So the next occurrence identifies itself. Until then the cause is unknown
  rather than fixed, and this entry should not be read as closing it.
- Verification: `cargo fmt --all`, `cargo clippy --workspace --all-targets
  --locked -- -D warnings`, `cargo test --workspace --locked`.

## 2026-09-23 — The estimator learns the boundaries the stepper already runs

Reported: adaptation fails at load on the autosaved scene. Reproduced against
that autosave - the estimate returns `InvalidCoefficients` - and the cause is a
capability gap rather than a defect.

- `canonical_temporal_indicator_supplement` refused unless
  `conservative_bulk_supported()`, which is
  `!open && ungapped && undamped_boundary && !has_loss && undriven_boundary`.
  The reported scene has four second-order outgoing sides, so `open` blocked it;
  nothing else did. The guard was correct when written, and said so in its own
  doc, but the temporal path executed only the conservative bulk then. This
  session closed all six boundary compositions, so the stepper runs open
  boundaries while the estimate never learned them - and a document with an
  outgoing wall is the default one.
- The supplement now carries the thin-gap and outgoing defect terms, which are
  the fixed path's own with the mass and force in force at the instant standing
  in for the authored ones. `diagnostic_derivative` gained a mass-taking variant
  for the same reason the kick did: the trace admittance and the modal couplings
  divide by the mass at the instant being measured.
- The guard narrowed to `indicator_supplement_supported`. Loss, a damped
  boundary and prescribed boundary data are still refused, because each puts a
  term in the evolution that the defect would otherwise charge to the mesh and
  none of those has been derived here.
- `open_boundary_supplement_matches_the_fixed_one_when_inert` is the check that
  was missing: an undriven medium behind an outgoing wall must produce exactly
  the fixed supplement's outgoing contribution and per-element boundary
  residual. It asserts the fixture exercises the term before comparing, so it
  cannot pass on a scene without a wall. The existing parity fixture is a
  reflecting box, which is why nothing caught this.
- End to end on the reported autosave the estimate goes from
  `InvalidCoefficients` to succeeding.
- Verification: `cargo fmt --all`, `cargo clippy --workspace --all-targets
  --locked -- -D warnings`, `cargo test --workspace --locked`, the device AMR
  gate, and `temporal_amr_calibration` - whose efficiency indices stay bounded,
  the widest row spanning `1.12x` over `15x` the unknowns.

## 2026-09-23 — A field survives its medium starting to move

Reported, and the last of the four: enabling a non-stationary material threw the
field away. A change of drivenness skipped the transfer entirely, on the reading
that the two generations share no layout and a fresh start is more honest than
inventing the missing half.

- Nothing was missing. `Q` and `b` exist in both and map as they always do. What
  the driven side has and the inert side does not is the material runtime bank,
  and the transfer shader already writes a target record with no source from the
  target's *own* authored anchors - which is exactly where a drive that was just
  switched on should begin. The capability was there and gated off.
- Both gates were host-side. `with_temporal_material_runtime` required a
  temporal manifest on both plans and now treats a missing one as zero records,
  so an inert source maps every target material to `NO_INDEX` and an inert
  target has nothing to write. `compile_gpu_upload` returned no transfer
  whenever drivenness changed; that early return is gone, and one
  `compile_generation_plan` helper builds the source plan for either kind.
- The handoff's layout check needed nothing: it compares each side's record
  count against its own plan, so `(0, 1)` and `(1, 0)` both pass. `state_count`
  excludes the runtime words, which is why driven-to-driven already worked.
- Tests assert `(0, 1)` enabling a drive and `(1, 0)` disabling one, on
  generations prepared the way the application prepares them.
- One consequence worth watching rather than asserting: the field now carries
  into a medium that starts moving under it, in one step. That is a temporal
  interface by hand, which suits a toy about them, but it is a discontinuity
  rather than a clean start. If it reads badly in motion the answer is a ramp,
  not a return to discarding the field.
- Verification: `cargo fmt --all`, `cargo clippy --workspace --all-targets
  --locked -- -D warnings`, `cargo test --workspace --locked`, and the device
  gates that exercise a handoff.

## 2026-09-23 — The host was preparing the same generation every frame

Reported with a driven medium running: the phase label churned every frame and
was unreadable, adaptation never ran, handoffs piled up and the frame rate fell.
The report's own guess - that the host was constantly doing something with the
device - was right, and three of the four symptoms are one comparison.

- `retime_for_speed` decides whether the running generation is at the timestep
  the speed control wants. It compared against `active.operator`'s recommended
  step - the base operator's, built from the stripped model - while the upload
  uses the generation's own, which on a driven medium is tighter because the
  coefficient trajectory tightens the CFL bound. The gap on the reported scene
  is `8.78e-3` against `1.007e-2`, `14.7%` against a `10%` hysteresis, so every
  frame decided the step was wrong, cleared the requested revision and prepared
  the whole generation again: mesh, assembly, upload. The label churned because
  a new preparation started every frame, adaptation never saw a settled
  generation, and the frame rate went with the work.
- It now compares against `PreparedTopology::recommended_time_step`, which is
  what the upload chooses.
- `pacing_does_not_re_request_a_generation_running_at_its_own_step` asserts the
  two bounds differ by more than the hysteresis *before* checking the behaviour.
  Without that it would pass on a medium whose bounds happen to agree and prove
  nothing; with the old comparison it fails with the revision cleared.
- Verification: `cargo fmt --all`, `cargo clippy --workspace --all-targets
  --locked -- -D warnings`, `cargo test --workspace --locked`.
- Still open, and its own slice: **enabling a drive resets the field**. A change
  of drivenness skips the transfer. `Q` and `b` would map either way and
  authored anchors are the right initial runtime for a drive just switched on,
  but `with_temporal_material_runtime` requires both plans to be temporal and
  the handoff compares each side's record count against its own plan, so the
  asymmetric case needs a "target has records, source has none, leave the
  target's authored values alone" mode in both the transfer plan and the
  transfer shader.
- Also worth measuring if handoffs still feel long: the previous entry made
  every driven handoff rebuild the source plan. It is worker-thread packing
  rather than frame work, but it is real and was not there before.

## 2026-09-22 — The driven handoff, and an error that could not be read

Reported: changing a material fails with "canonical GPU handoff layouts do not
match", and before it fails the phase label churns every frame so nothing can be
followed.

- `source_material_runtime_count` and `target_material_runtime_count` on a
  transfer are set only by `with_temporal_material_runtime`, which had no
  production caller. A driven handoff therefore described no runtime records
  while both plans held one each, and `begin_handoff` refused the layout however
  well the state mapped. The previous entry's fix is what made this reachable:
  until then every preparation after the first came out inert, so a driven
  handoff never ran.
- `compile_gpu_upload` now rebuilds the source plan from the active generation's
  temporal operator and installs the mapping. The plan is rebuilt rather than
  kept because the request holds device buffers, not the plan they came from;
  only its layout and its drives' identities are read, and both follow from the
  operator and the forcing, so rebuilding recovers them exactly. It is packing
  work on a worker thread.
- This also closes the gap noted in the previous entry from the other side: a
  pump's carrier phase and a Switch mid-ramp now survive an edit instead of
  restarting from their authored anchors, and the lane-following correction made
  earlier today is on a live path rather than only in its test.
- `material_runtime_counts` exposes what the handoff compares, so the test can
  make the same comparison. Without the mapping it reads `(0, 0)` against the
  `(1, 1)` the plans hold.
- **A failed preparation was retried every frame, forever.** Nothing recorded
  that a revision had already failed, so the next frame ran the whole
  preparation again - assembly included - and replaced the error message before
  it could be read. That is why the report could not say what was happening. The
  request guard now treats "already failed for this revision" as it treats
  "already in flight" and "already accepted"; the next edit, a different mesh
  edge or an explicit remesh moves on, and Reset publishes onto the accepted
  generation without coming through that path.
- Worth naming: this is the third defect in a row reachable only on a path the
  previous fix opened. Authoring a drive was the first thing in the application
  that could flip `driven()` against a running generation, and every stage of
  the handoff it touches had been written but never executed.
- Verification: `cargo fmt --all`, `cargo clippy --workspace --all-targets
  --locked -- -D warnings`, `cargo test --workspace --locked`.

## 2026-09-22 — A driven medium stopped driving itself

Reported after the previous fix: applying a material reset the field, and a
preset ran the driven medium for a second or two before reverting to a
stationary one. Both are one defect.

- A preparation that reuses its canonical operator was not reusing the temporal
  operator built over it. That operator is only constructed on the assembly
  path, so the first preparation after an edit was driven and every one after it
  - the application prepares repeatedly while it runs - came out inert. The
  medium stopped driving itself a moment after it started.
- The field reset followed from that rather than being separate. Once drivenness
  had changed, `compile_gpu_upload` returns no transfer, because the two
  generations do not share a state layout, so the candidate was installed and
  the field started from zero. Reverting to stationary and losing the field were
  the same event.
- The fix is to carry the previous temporal operator when the operator is
  reused. That is sound because `operator_scene_eq` compares whole materials:
  a reused operator means the laws are exactly the ones it was compiled
  against. `a_driven_generation_stays_driven_across_repeated_preparations`
  prepares the same unchanged driven document four times and requires every one
  of them to stay driven; before the fix, round one was driven and rounds two
  onward were not.
- Verification: `cargo fmt --all`, `cargo clippy --workspace --all-targets
  --locked -- -D warnings`, `cargo test --workspace --locked`.
- Two gaps found while diagnosing this, neither fixed here.
  **`with_temporal_material_runtime` has no production caller**: the function
  that maps carrier phases and Switch trajectories across a handoff is reached
  only from its tests, so every driven handoff restarts the material runtime
  from authored anchors - a pump's phase jumps and a Switch mid-ramp is lost
  whenever anything is edited. The lane-following correction made earlier today
  is therefore exercised only by its test.
  **Turning a drive on still resets the field**, because a change of drivenness
  skips the transfer entirely. Nothing appears to be genuinely missing: `Q` and
  `b` transfer either way, and authored anchors are the correct initial runtime
  for a drive that was just switched on rather than an invention - which is also
  what a driven handoff already lands on today, given the gap above. Whether the
  rest of the transfer tolerates the differing state-word counts needs checking
  rather than assuming.

## 2026-09-22 — Two defects the authoring panel exposed

Reported from the running application: after a parametric pump applied, the next
preset never committed, the runtime sat at "Ready for GPU upload" and Reset did
nothing. Reproducing it against the autosave found two unrelated faults, neither
of them in the preset code.

- **The upload waited for a generation that never arrives.** `ui/runtime.rs`
  chose which generation to wait for from `candidate.fresh`, while the path
  taken is chosen by whether a transfer exists. `install` publishes its
  generation synchronously and `begin_handoff` publishes on completion, so the
  `+1` belongs to a handoff and not to an install. Those agreed only while a
  fresh candidate meant an install - and `compile_gpu_upload` returns no
  transfer whenever `active.driven() != candidate.driven()`, because the two
  generations share no state to map. So an edit that starts or stops a drive
  installs while not being fresh, and the upload waited for a generation one
  past the one that arrived. Reset is gated behind `uploading.is_none()`, so it
  could not clear it either: one cause, both symptoms. The expectation now
  follows the path actually taken.
- This is latent in the drivenness rule rather than new. Nothing in the
  application could author a drive before today, so no edit had ever flipped
  `driven()` against a running generation.
- **A preset did not take its parameters with it.** Applying one over another
  left the old preset's parameters behind: a pump then Linear left `depth`,
  `pump_hz` and `pump_phase` with no law referring to them, which is exactly
  what the reported autosave held, and a pump then a time crystal reached seven
  of the eight a material can hold - after which the next choice was refused and
  the selector appeared to do nothing. `apply_law_preset` now retires the
  outgoing preset's parameters, keeping any the user pointed at from another
  slot, and a test chains the whole catalogue three times over without
  accumulating.
- The reported symptom named the second application, but the fault was the
  transition *out* of driven - the user had switched back to Linear before
  reporting, which is the same flip in the other direction.
- Verification: `cargo fmt --all`, `cargo clippy --workspace --all-targets
  --locked -- -D warnings`, `cargo test --workspace --locked`.

## 2026-09-22 — A drive can be authored

- The material editor grows a **Response** selector: the catalogue by
  phenomenon, with the mathematical form in hover text, showing the matched
  preset or Custom. Under it, the matched preset's variables as named
  parameters with its own ranges; a Switch ramp whenever either row carries an
  alternate, since that is one number on the material rather than a slot on a
  row; and the effective-law text in the named register. Applying a preset
  stages like any other material edit and commits on Apply, which is the one
  undo step section 11 asks for.
- That closes the loop: pick a pump, turn its depth up, Apply, and the document
  compiles to a temporal plan and runs. Until now the only driven scenes in this
  project were built in test and example code.
- **One label changed on the strength of a measurement.** `wave_coefficients`
  reciprocates the complementary slot for TM and TE but not for Mechanical,
  while the solver divides the complementary coefficient by its law factor in
  every skin. Working through all six (skin, row) pairs, five multiply the
  coefficient stored beside them and mechanical stiffness does not: a law there
  multiplies `s0 = 1/k0`. `a_pump_on_the_stiffness_row_lowers_the_mechanical_stiffness`
  measures it - a crest of `1 + 0.5` divides the stiffness by exactly 1.5 - so
  that row is named **Reciprocal stiffness s0** wherever a law touches it. A
  depth slider beside a control called stiffness would have read backwards.
- This also sizes the advanced view's honesty problem, which looked bigger than
  it is. Only that one field differs between the two presentations; everywhere
  else advanced and linear name the same quantity, so "advanced has to be
  honest" costs one relabelled field rather than a second set of numbers.
- **The panel did not work on the default document, and the end-to-end test is
  what caught it.** Applying a pump and preparing the result failed in the
  Assembling phase with "Point source placement is invalid: probe cannot use the
  current mesh" - a message about the mesh, for a material law. Every probe
  stencil, the far-field stencil and the volume-source compiler read
  `bundle.model()`, the *authored* model, and `Material::evaluate` refuses a
  law-carrying material on purpose; the probe then mapped that refusal onto its
  own `InvalidMesh`. Five sites, all missed when the assemblies were taught to
  build from the stripped model. They now take the stripped model too, which is
  the same base coefficients both assemblies use and what a stencil's placement
  belongs to - the drive is applied at stage time by the temporal path.
- Worth naming how this was isolated: the same sequence with a colour-only edit
  prepared cleanly, which separated "my test is wrong" from "a drive breaks
  preparation" in one run. Without that the obvious reading was fixture
  friction.
- So a driven document previously prepared only when it had no enabled probe and
  no volume source. `canonical_gpu_driven_document` has neither, which is why
  the device gates never saw this.
- The first arrangement read as two editors stacked, which the user called out.
  The real duplication was that a preset's parameter had two controls - one
  labelled in the response section, one raw in the Parameters list - and the
  selector sat below the coefficients as though it were an extra feature rather
  than the thing that decides what a material is. Now: name, Response, the base
  coefficients, the preset's own values, the effective-law text, and a
  Parameters list holding only what the user made.
- The base coefficients stay a separate segment on purpose, and the reason is
  not layout. They keep their Constant/Formula editor because a spatially graded
  density is a real capability a preset must not take away, and they survive a
  change of preset where a preset's own values are replaced by it. The separator
  is that distinction rather than chrome.
- Recorded so it is not assumed later: a **drive parameter cannot be spatially
  varying**, and making one so is not a UI relaxation. `TimeDrive::evaluate`
  resolves depth, frequency and phase through `evaluate_constant` into one
  runtime record per material, and the device packs one record per material
  rather than per node. The travelling modulation's `q·x` is the sanctioned
  spatial dependence, which is why it is a drive variant instead of a spatial
  depth. Base coefficients and field-law slots are spatial; drive parameters
  would need per-sample drive data on both sides.
- Uniformity between the two segments stays available and additive: the drive
  slots are already `ScalarField`s, constrained to bare parameter references
  only so the matcher can recognise them. Relaxing that to any expression over
  parameters would give both segments the same widget, at the cost of widening
  what structural match has to accept.
- Verification: `cargo fmt --all`, `cargo clippy --workspace --all-targets
  --locked -- -D warnings`, `cargo test --workspace --locked`, plus a native
  launch against an isolated `HOME`.
- Next: the Advanced toggle in the Library header with the raw slot editors and
  `s0` editable there, then moving the AMR estimate onto the decoded runtime
  bank - which has to land before a Switch button exists - and then the Switch.

## 2026-09-22 — The preset catalogue, applied and recovered

- `law_presets` in core: the named points in the law catalogue a user can put
  on a material. Ten entries - Linear, and the four time-driven laws on each
  constitutive row, plus the impedance-preserving pair that needs both.
- A preset is a factory, not a binding. It creates named parameters and wires
  the slots to formulas referring to them, and after that the material stands on
  its own; the editor recovers which preset produced it by matching structure.
  Editing a slot to a bare constant, or moving a drive to the other row, simply
  stops it matching and the material reads as Custom rather than being refitted.
- `apply_law_preset` and `identify_law_preset` are both written against one
  private constructor, so a preset cannot be applied in a shape its own matcher
  would not recognise. The round-trip test covers every entry, which is what
  catches that class of disagreement.
- Presets are named for the row rather than the physical quantity, because the
  quantity depends on the skin: the mass row is the density in Mechanical and
  the permittivity in the electromagnetic skins, and those are not the same
  thing - the density pairs with the permeability. The editor takes the label
  from the skin, so a pump stays a pump across a physics change while its label
  follows the coefficient.
- Only laws that run are listed, which section 11 asks for directly: a preset
  behind an open design gate is unavailable rather than offered and refused.
  A test asserts no preset writes a field law, a restoring law or a loss
  channel, so the catalogue cannot drift ahead of the solver by accident.
- Two behaviours worth naming. A parameter the user authored is never taken: a
  preset wanting a name already in use gets the next free suffix, and the user's
  value survives. Re-applying the preset a material already carries keeps the
  values it has been tuned to, so the selector is not a reset button and does
  not grow a second copy of every parameter.
- `every_preset_survives_a_skin_change` ties this to the previous entry: every
  catalogue preset converts Mechanical to TM and back to exactly itself, so the
  preset view keeps working across a skin change instead of reporting a material
  the user cannot see into.
- A material carrying a restoring law or a loss channel reads as Custom even
  when its constitutive rows are untouched. No preset reaches those slots, so
  calling it Linear would name the medium after the half of it the selector
  happens to inspect.
- A material with no room for a preset's parameters is refused before anything
  is written, under `MaterialError::ParameterLimit` rather than the misleading
  unsupported-law error, so a failed application leaves the material alone.
- Verification: `cargo fmt --all`, `cargo clippy --workspace --all-targets
  --locked -- -D warnings`, `cargo test --workspace --locked`.
- Next: the editor frame - the Advanced toggle in the Library header as
  presentation state, the preset selector as an authored edit, and the
  effective-law text above the controls in both non-linear views.

## 2026-09-22 — A driven medium can change its physics skin

- A skin change swaps the two stored constitutive slots, because the mechanical
  adapter is `s0 = eps = 1/k0` and `mu = rho` and neither slot holds the same
  physical quantity in both skins. `convert_material` refused outright whenever
  a material had any law, so one pumped material blocked the whole scene's
  change - and `set_physics` is transactional across all materials, correctly,
  so it blocked every other material with it.
- A time drive and a Switch alternate do convert. They are multiplicative
  factors on a coefficient, so the row that reciprocates carries its law
  reciprocated, and `inverted` expresses exactly that: it divides by the whole
  multiplier, which is the reciprocal of `drive * switch`. Nothing else moves -
  depth, frequency, phase and wavevector are all in the material frame.
  `CoefficientLaw::reciprocated` is the one-line law-level counterpart to the
  `reciprocal()` already applied to the base expression.
- A field law still does not convert, and now says so by name. Section 5.3:
  a reciprocal nonlinear coefficient is not an inverse nonlinear constitutive
  map, and the physical field the law reads changes with the skin. That is gate
  C. Restoring laws have no counterpart to move to, and loss channels stay on
  their own physical field with a rate conversion of their own, so both are
  refused rather than guessed at. `UnconvertibleMaterialLaw` carries which one
  blocked it, so the editor says which slot to clear instead of reporting an
  unsupported material.
- This is reachable now rather than after gate C because section 11 filters a
  preset with an open design gate out of the selector, so a material authored in
  the preset view can only carry the convertible laws.
- The round trip is exact: two reciprocations cancel, and a law left carrying
  only `inverted` normalizes away, so toggling skins does not accumulate flags
  on a medium that never changed. The measurement is physical rather than
  structural - the converted permittivity must equal one over the original
  stiffness at twelve instants through a drive cycle with a Switch mid-ramp.
- **Enabling it made a latent defect reachable.** Drive lanes in the material
  runtime bank are ordered mass row then stiffness row, and the transfer
  preserved a carrier phase only when the source and target lane *indices*
  matched. A skin change moves a drive between lanes, so both carriers would
  have silently reset to their authored anchor and the modulation would jump
  phase at the switch - on a path section 4.4 says preserves its compiled maps.
  The host now matches lanes by drive signature and names the source lane per
  target lane in the mapping word; the shader reads the phase and frequency from
  that lane. Unambiguous, because two lanes sharing a signature cannot be told
  apart by phase either, and it reduces to the old test whenever nothing moved.
- Both tests were checked against their own failure: dropping the reciprocation
  breaks the physical equality, and restoring lane-for-lane matching breaks the
  carrier mapping.
- Verification: `cargo fmt --all`, `cargo clippy --workspace --all-targets
  --locked -- -D warnings`, `cargo test --workspace --locked`.
- Next: the preset table and structural match. Presets are named by phenomenon
  and row rather than by slot, because the mass slot is `rho` in Mechanical and
  `eps` in EM and those are not the same quantity - `rho` pairs with `mu`. The
  row label comes from the skin, so a pump stays a pump across a change while
  its label moves from density to permeability.

## 2026-09-22 — The law catalogue, written down and read back

- The catalogue of material laws existed only in conversation. It is now
  [docs/material-law-catalogue.md](material-law-catalogue.md), by slot and ID,
  with what each law does to a wave, how it is stored, and whether it runs.
  Linked from architecture and from section 5.1 of the material-laws plan,
  whose slot table is the schema it is drawn against.
- Writing it out turned up that **slot K's time-driven half is not deferred**.
  The catalogue had it as future work; `canonical_temporal.rs` evaluates
  `stiffness_law.drive` and `temporal_amr_calibration` crosses a driven mass row
  against a driven stiffness row against both. K-T1 to K-T4 run. That makes the
  reflectionless time interface - modulate `m` and `K` so `Z = sqrt(mK)` holds
  still - available now, expressible as one drive per row with `inverted` on
  one, and falsifiable against M-T1's reflecting interface.
- Also: the variant named `TimeCrystal` is M-T4, the smoothed square, while band
  gaps are M-T2's phenomenon. Controls must carry the catalogue's phenomenon
  names, not the Rust variant names, or the selector points at the wrong law.
- What runs today: the whole time-driven row of M and K, plus M-T1's Switch.
  Everything field-driven sits behind gate C, the restoring row and van der Pol
  behind gate O, and a driven loss channel behind a composition the timed
  evaluator refuses outright. The refusals are in `linear_material_sample` and
  `evaluate_timed_directional_material_library_at`; the catalogue names both.
- First piece of the authoring UI: `law_summary` in core, the effective-law
  text. One line per coefficient a law modifies and nothing for one left alone,
  in two registers - authored names for the preset view, evaluated numbers for
  the advanced one. It lives in core because it has to stay true to what the
  solver evaluates: every match is exhaustive, so a new catalogue entry cannot
  be added without saying what it reads as.
- Section 11 of the plan specifies this UI in more detail than I had planned it,
  and corrected two things: preset identity is recovered by structural match
  rather than stored ("presets are snapshots, not hidden live bindings"), and a
  preset whose law is not implemented is filtered out of the selector rather
  than offered and refused. So every document a user can author assembles.
- The `s0` question was already settled, and I first recorded it as an open
  divergence in error. There is no storage conflict: the two stored slots are
  skin-relative and `convert_material` is the ground truth for what they mean -
  crossing to EM sets `mass_density = 1/stiffness` and `stiffness = mass_density`,
  so mechanical `s0 = 1/k0` is epsilon and mechanical `rho` is mu, exactly as
  section 5.1 says, and both editor labels are right.
- What the decision actually covers is Kerr. Orthodox Kerr is a law on the
  *direct* coefficient, `D = eps (1 + chi |E|^2) E`. In EM that is the stored
  mass slot, so it is ordinary; in Mechanical the same medium's epsilon is
  `s0 = 1/k0` while the stored slot is `k0`, and writing `1 + chi u^2` on `k0`
  gives `s = s0/(1 + chi u^2)`, a different medium. So: author on the direct
  coefficient, keep `k0` as a derived editing view, and where a reciprocal law
  is genuinely wanted express it with explicit `inverted` semantics as its own
  map. Section 5.4 validates the two separately, which is the proof they differ
  - the direct polynomial's tangent is `1 + 2 chi1 u + 3 chi2 u^2` and the
  reciprocal's numerator is `1 - chi2 u^2`, and direct Saturable admits
  `a > -8/9` where reciprocal Saturable admits `a < 8` or `a > -1`.
- What remains is only the advanced mechanical *presentation* of `s0`, not a
  storage change. `ScalarField::reciprocal` goes through `simplify`, whose
  `simplify_once` carries a `1/(1/x) -> x` rule, so opening and closing that
  view already round-trips a spatial `k0` without expression growth.
- Loss channels are not in the summary yet. Their slot is authored but no path
  executes a driven one, and naming the two channels in the mechanical skin is a
  question for the advanced view rather than for this function.
- Verification: `cargo fmt --all`, `cargo clippy --workspace --all-targets
  --locked -- -D warnings`, `cargo test --workspace --locked`.
- Next: the three-way editor mode - Linear, presets, Advanced - with the
  summary above the controls in the latter two.

## 2026-09-22 — A driven medium can stand behind a second-order wall

- Implements the previous entry. `prepare` no longer takes a nodal mass: it
  builds `diag(D + Gamma)` plus `a_k - 1` per mode and the pole eliminations,
  and `solve` receives the stage's mass. `DenseLu` is deleted; nothing in the
  trace path factorizes any more.
- The solve is a diagonal-preconditioned sweep. The pass count comes from the
  factor's own `max_k |a_k - 1|`, refused above `OUTGOING_TRACE_SWEEP_LIMIT`, so
  the gate is a computed statement about a generation rather than a rule about
  driven media. `one_outgoing_preparation_solves_every_nodal_mass` holds it
  against the dense generator at three masses - authored, a uniform `2.7x` pump,
  an arbitrary non-uniform wobble - and three step fractions, and separately
  requires each pass to decay at least as fast as the recorded bound.
- A prescribed trace row is now held at its own right-hand side through the
  sweep instead of rebuilding and re-inverting a constrained matrix.
  `export_with_prescribed` is down to recording the pattern.
- **CPU cost.** The driven second-order wall went from `27147` to `6373 us/step`
  at `h=0.1`, `35x` the fixed path down to `8x` - now below the `13x` bulk floor
  rather than far above it.
- The first version of this regressed the *fixed* CPU path from `777` to
  `1772 us/step`, because ten sweeps cannot match one triangular solve. A
  generation whose mass cannot move now also inverts once, through
  `prepare_static`, and picks that lane when the mass presented is the one it
  inverted. The fixed column came back to `776 us/step`, unchanged. Both lanes
  are held against the same dense oracle by the same test.
- **Device.** The boundary buffer loses its dense `trace^2` inverse and carries
  `trace + modes` scalars instead. The sweep's two halves are two kernels; the
  barrier a pass needs between them is the one between dispatches.
- The first version ran the whole solve in one workgroup so a pass could use a
  real barrier, and read `6975 us/step` against `520` without the wall - `13x`.
  One workgroup is one core with 128 threads and nothing to hide storage
  latency behind. Spreading each half back over the boundary took it to
  `1041 us/step`, `6.7x` better, against `833 us/step` for the same fixture
  before any of this. So a second-order outgoing generation costs `1.26x`, and
  driven and fixed read the same `~1045`: per step the drive is still free.
- **The refusal is gone**, along with `outgoing_trace_mass_is_static` and the
  test that existed only for it. `DRIVEN_WALLS=outgoing` on
  `canonical_gpu_driven_document` now runs instead of asserting a refusal, and
  `canonical_gpu_temporal_timing --outgoing` is where the wall's device cost is
  read.
- **A second defect surfaced on the way.** That run first read `1.263e-3`
  against the bulk case's `2.46e-7`. The device was pinning a driven kick, and
  reading its mass, half a step in - the fixed path's convention, which the CPU
  reference had already moved off for exactly this reason. Correcting it took
  the outgoing run to `3.31e-7`, and independently took
  `canonical_gpu_temporal_forced` from `5.98e-6` to `2.42e-7`. Two examples
  improving from one change is what makes this a cause rather than a coincidence.
- Verification: `cargo fmt --all`, `cargo clippy --workspace --all-targets
  --locked -- -D warnings`, `cargo test --workspace --locked` (752 passing, 1
  known ignored reproducer), `cargo build --release -p funfern-app --locked`.
  Device gates on an M1 Max / Metal: driven document `2.457e-7` and, behind an
  outgoing wall, `3.13e-7`; forced `2.415e-7`; temporal work within `2.0e-7`;
  AMR within its documented bounds; filter boundary `4.14e-7` / `1.09e-6`.
- Next: authoring UI (drives, Switch, presets), which is also what forces the
  runtime-bank decode against the live epoch origin.

## 2026-09-22 — The outgoing trace system hides a mass-free operator

- Paper work on `prepare`, before any implementation, to decide whether the
  outgoing refusal can be lifted. Two questions: does the Schur elimination of
  the three-pole blocks preserve a `K M^-1` shape, and does the assembly's
  existing modal basis diagonalize the mass-scaled result.
- The first answer is yes, exactly. The matrix the midpoint solve factors is
  `S = I + (h/2) K M^-1`, where `K = D + sum_k a_k t_k t_k^T` collects the
  first-order impedance diagonal and the modal outer products. Mass enters
  `prepare` in exactly two places, both of them the `/ mass[node]` on the right.
  The pole blocks never see it: their 3x3 inverse, `aqz`, `azq` and the resulting
  `schur_coefficient` are built from a mode's decay and the step alone, so
  eliminating them rescales `a_k` and leaves the shape alone.
- `the_outgoing_trace_system_hides_a_mass_free_operator` is the falsifier. It
  recovers `K` from the factored `DenseLu` at three masses - authored, a uniform
  2.7x pump, and an arbitrary non-uniform wobble - and requires them to agree.
  They do, to `1e-9` of scale, and `K` is symmetric to `4e-17`. Dropping the mass
  factor from the recovery makes it fail by `8.4`, so the test is measuring what
  it claims.
- The second answer is no. The existing eigensolve diagonalizes
  `Gamma^-1/2 K Gamma^-1/2` with `Gamma` the second-order trace impedance; the
  driven path would need `M^-1/2 K M^-1/2`. Those differ because `Gamma` is a
  line integral and `M` is the lumped volume mass: on the square fixture their
  ratio spreads 2:1 across the trace, and the existing basis leaves **25.6%**
  off-diagonal. Reusing it is not an option; a second Jacobi of the same cubic
  cost would be.
- That second eigensolve turns out to be unnecessary, which is the useful
  finding. Because the eigenvectors are orthonormal,
  `sum_k 1 * t_k t_k^T = Gamma` exactly, so
  `K = (D + Gamma) + sum_k (a_k - 1) t_k t_k^T` - a **diagonal** matrix plus a
  correction carrying only the pole modes' deviation from one. The three-pole
  DtN residues `6/7, -8/7, 2/7` sum to zero, which makes
  `a_k - 1 = (1/7)(h * decay_k)^2 + O((h decay)^3)`. Measured: `1.37e-4` at
  `0.08` of the CFL step and `5.28e-3` at half of it, a ratio of 39 against the
  predicted 39.1.
- So the solve can be a diagonal-preconditioned Jacobi iteration.
  `Delta(t) = I + (h/2)(D + Gamma) M(t)^-1` is diagonal and follows an arbitrary
  mass for free; the correction `C = sum_k (a_k - 1) t_k t_k^T` is mass-free and
  uploads once. Contraction per sweep, measured: `4.7e-6` at 0.08 CFL,
  `9.5e-4` at half, `4.7e-3` at 0.9, `6.2e-3` at the full CFL bound. Eight sweeps
  reproduce the direct solve to `2.2e-16` at every step size.
- The contraction has a closed form that is free of the mass. With `D = 0`,
  `Gamma^-1/2 C Gamma^-1/2 = V (A - I) V^T`, so the iteration matrix's spectral
  radius tends to `max_k |a_k - 1|` as the mass shrinks and to zero as it grows,
  and `D >= 0` only lowers it. Checked against 400 random masses spanning `1e4`
  in scale with arbitrary non-uniformity: the bound held at both half and full
  CFL, and it is tight to 3%.
- This is strictly better than the eigendecomposition route on every axis. No
  second eigensolve, so the outgoing assembly cost recorded two entries ago does
  not double. No uniformity restriction, so the refusal lifts for a travelling
  modulation and not only a pump. And the sweep count is a plan-time number
  derived from `a_k`, which `prepare` already computes, so the gate becomes a
  computed bound rather than a blanket rule.
- Measured on the square fixture at 8 trace nodes. The `a_k` bound is structural,
  but the constant is mesh and material dependent, so a real implementation
  should compute it per plan rather than hardcode a sweep count.
- Verification: `cargo fmt --all`, `cargo clippy --workspace --all-targets
  --locked -- -D warnings`, `cargo test --workspace --locked` (753 passing, 1
  known ignored reproducer).
- Next: implement it - split `prepare` into the mass-free `K` plus the diagonal,
  carry the contraction bound, replace the cached `DenseLu` with the iteration on
  both the CPU reference and the device export, and delete
  `outgoing_trace_mass_is_static`.

## 2026-09-22 — The outgoing refusal is about the trace, not the generation

- The refusal landed in the previous entry was broader than the defect. The
  stale factorization touches only the boundary trace: trace nodes are advanced
  by the dense solve and skipped by the local kick, so no interior node sees
  that matrix. An error confined to a few hundred trace nodes out of 3811 is
  exactly what an `8.5e-3` whole-field norm looks like.
- That does not make it safe to run - the boundary is where energy leaves, so a
  wrong wall reaches the interior within a few crossing times - but it does
  mean the test belongs on the trace. Nothing else in that system can move: the
  diagonal is `damping / mass`, the couplings are `trace . trace / mass`, and
  the damping and trace are assembled constants. A medium driven where the
  boundary does not reach keeps a wall that is exactly what it was assembled
  as.
- `outgoing_trace_mass_is_static` is that test, and `compile_temporal` now asks
  it instead of asking whether the generation is driven at all. A drive
  confined to an interior inclusion compiles and runs.
- The test for it needed a real two-material fixture, and the first version did
  not have one: `Scene::initial()` carries a single material, so the case that
  actually discriminates never ran and the test passed while proving less than
  it claimed. It now builds a background and an inclusion and checks all four
  combinations, including the one that matters - driven inclusion, inert
  background, wall still valid.
- **What reassembly would cost, measured rather than presumed.** At 8938 DOFs
  the trace is 276 nodes and 825 auxiliary scalars, the factorization takes
  `22.5 ms`, and a whole KDK step takes `1.5 ms`. Two refactorizations per step
  is roughly `30x`, which agrees with the driven second-order row of the cost
  table from the other direction. It also degrades with refinement: the trace
  dimension grows like `O(sqrt(N))`, so a dense factorization is `O(N^1.5)`
  against a step's `O(N)`.
- **A third option worth deriving before choosing between the first two.** The
  mass enters as a diagonal scaling of an otherwise fixed operator - the trace
  system is `I + h K M^-1` with `K` fixed, equivalently `M (M + h K)^-1`. A
  changing diagonal has no cheap update in general, but a drive that is
  spatially uniform over the trace gives `M = m(t) M0`, and one eigendecomposition
  of the symmetric `M0^-1/2 K M0^-1/2` turns each stage into a diagonal inverse
  between two fixed matrices: `O(d^2)` rather than `O(d^3)`. A travelling
  modulation that varies along the trace breaks that and would need the real
  refactorization. This is traced by eye through `prepare`, not derived - the
  auxiliary elimination folds terms into the trace block and that step has to
  be checked before the structure can be relied on.
- **And the machinery that option needs already exists.** Measured on the same
  mesh, an outgoing boundary costs `280.88 ms` to compile against `4.99 ms`
  without one - fifty-six times, with the boundary accounting for about
  ninety-eight per cent of it. That is not the factorization, which is a
  separate `22.5 ms`: the assembly builds the tangential operator and takes a
  symmetric eigendecomposition of it over the trace, carrying each mode as
  `eigenvector * sqrt(damping)`. So the expensive part of the modal route is
  already paid, once, and the open question narrows usefully. If the existing
  basis also diagonalizes the mass-scaled system, a stage costs a diagonal
  inverse in coordinates that already exist. If a second, differently weighted
  decomposition is needed, that is another assembly-time cost paid on remesh or
  material edit - not per stage. Either branch beats refactorizing, which is
  `30x` per step.
- What survives either way: this only helps when the mass's spatial *pattern*
  at the trace is fixed and only its amplitude moves, which a pump or a time
  crystal satisfies. A travelling modulation changes the pattern's shape along
  the trace, so no fixed basis diagonalizes it.
- Checks: `cargo fmt --all`, `cargo clippy --workspace --all-targets --locked
  -- -D warnings`, `cargo test --workspace --locked` (752 passed, 1 known
  ignored reproducer), and the end-to-end gate both ways - `2.46e-7` running,
  and the refusal when the drive reaches the trace.

## 2026-09-22 — The end-to-end failure is the outgoing wall, and it is now refused rather than run

- Bisected by configuration rather than by stage, because the intermediate GPU
  state is not readable and guessing at the shader had already cost one wrong
  suspect.
- **First cut: modulation depth.** Keeping every piece of the temporal path
  switched on but setting the depth to zero reads `4.47e-8`; at `0.05` it is
  `2.04e-3` and at `0.24` it is `8.45e-3`, roughly linear. So the temporal path
  is sound and a *moving* mass is what is mishandled. That one measurement
  eliminated the whole assembly and plan pipeline.
- **Second cut: the boundary.** The application's default document carries
  second-order outgoing walls on all four sides, which no other driven fixture
  had. Swapping them for reflecting ones, with everything else identical:

  | walls | one step | forty-eight |
  | --- | --- | --- |
  | reflecting | `4.19e-8` | `2.46e-7` |
  | second-order outgoing | `8.45e-3` | `5.49e-2` |

- The mechanism was already on record from the CPU composition. A second-order
  wall's trace factorization is built from the nodal mass, which is why the
  reference rebuilds its Schur complement at every stage and why that showed up
  as the one asymptotic cost in the timing table. The device plan compiles that
  factorization once, from the authored mass, and has no mechanism to refresh
  it. The two paths were never going to agree.
- **Refused rather than approximated.** `compile_temporal` now rejects a driven
  generation behind a second-order outgoing boundary, with the reason in the
  message. Running it anyway is a five per cent error that grows with depth, and
  the specification's rule against executing an authored law as something it is
  not applies just as much to executing it against a wall that cannot follow it.
- The end-to-end gate now runs reflecting walls and reads `2.46e-7`, and
  `DRIVEN_WALLS=outgoing` asks for the refusal instead, so both halves are
  checked. `DRIVEN_DEPTH` and `DRIVEN_STEPS` stay for the next investigation.
- The product limitation this creates is real and worth naming: the default
  document has outgoing walls, so a drive authored on a fresh document will be
  refused until the factorization can follow the mass. Lifting it is a device
  design change - rebuilding a dense trace factorization per stage, or
  parameterizing it by the mass the way the CPU maps now are - and it belongs
  before the authoring UI rather than after.
- Checks: `cargo fmt --all`, `cargo clippy --workspace --all-targets --locked
  -- -D warnings`, `cargo test --workspace --locked` (751 passed, 1 known
  ignored reproducer), `cargo build --release -p funfern-app --locked`, four
  device gates passing.

## 2026-09-22 — The end-to-end run fails, and every piece of it passes separately

- The check I said would be worth doing before the authoring UI, because it is
  where a surprise would hide. `canonical_gpu_driven_document` takes an
  authored document with a material drive through the application's own
  preparation - meshing, both assemblies from the stripped model, the temporal
  operator over the shared base - compiles a temporal plan from the result and
  steps it on the device against an f64 oracle.
- **It fails.** `Q` misses by `8.45e-3` after one step and `5.49e-2` after
  forty-eight. Committed failing on purpose, per the review's instruction to
  retain expected-failing artifacts rather than erase them.
- What makes it worth stating rather than fixing in the same breath: every
  solver-level gate passes on the same physics. The forced gate runs a pumped
  medium with a source and an absorbing wall at `5.978e-6`, the AMR gate at
  `4.6e-7`, the work gate at `1e-8`. The difference is the path into the
  solver, not the solver: this is the only fixture assembled by the
  application's topology jobs rather than by `assemble_scene`.
- Localized so far. The error is per step, not accumulated - one step already
  shows `8.45e-3`, and `DRIVEN_STEPS` is there to bisect it. The complementary
  lane reports a relative difference below f32 resolution, which is
  uninformative rather than clean: `b` starts large and the drift increment is
  small against it.
- One suspect raised and eliminated, which is worth recording so it is not
  raised again. The drift's force-cache update calls `stiffness_force`, which
  converted `Q` to a field with the authored inverse mass - wrong under a mass
  drive, and its sibling at the other call site already had the instantaneous
  branch. Fixed, and it changed the measurement by nothing at all, because this
  fixture runs with the force cache off. The fix is kept as a latent
  correctness repair with that limitation stated: it is not exercised here and
  is therefore unverified by measurement.
- The three passing device gates were re-run after that change and are
  unmoved, so it is a repair rather than a regression.
- Next step on this is bisection rather than more suspects: the per-step error
  with one step is small enough to compare stage by stage against the oracle.
- Checks: `cargo fmt --all`, `cargo clippy --workspace --all-targets --locked
  -- -D warnings`, `cargo test --workspace --locked` (751 passed, 1 known
  ignored reproducer), `cargo build --release -p funfern-app --locked`, three
  device gates passing and one failing as recorded.

## 2026-09-22 — A driven document runs

- The plumbing the previous entry listed, all of it. Both plan sites - the
  background upload and the reset - compile a temporal plan when the generation
  is driven and the fixed one otherwise. The timestep comes from
  `PreparedTopology::recommended_time_step`, which reads the temporal ceiling
  when there is one, so every caller gets the trajectory bound rather than the
  fixed operator's; that is the `1.23x` the GPU timing measured, and it is now
  where the drive's cost actually lands.
- `CanonicalTemporalWaveOperator::recommended_time_step` carries the fixed
  path's safety margin onto the trajectory bound rather than inventing a second
  one, so the two differ only by the ratio of their ceilings.
- **A drive added or removed forces a fresh start rather than a transfer.** The
  two generations do not share a state layout, so there is nothing to carry
  across; a transfer would have to invent the half that is missing. Refusing is
  the honest outcome and it is stated where the decision is made.
- AMR now takes the temporal supplement and the instantaneous materials on a
  driven generation, which is the whole point of that arc: the fixed
  supplement's residual, energy and recovery read authored coefficients, and on
  a driven medium that charges the estimator for the medium's own modulation.
  The size rule's `resolved_frequency_hz` and `coefficient_wavelength` were
  already wired from the earlier work, and the `1.88` calibration lives in the
  core, so one accuracy target still means one true accuracy.
- One limitation stated rather than discovered: the estimate uses the
  operator's authored runtime. That is correct while nothing has stamped a
  Switch or re-anchored a carrier, which nothing in the application can do yet.
  When drive authoring lands it has to become the bank decoded from the
  accepted state against the live epoch origin - the hazard reported after the
  AMR device gate, now with a named place where it will bite.
- Every document that existed before drives is untouched: no temporal operator,
  the fixed plan, the fixed timestep, the fixed supplement. The whole driven
  path is behind one `Option` being `Some`.
- Checks: `cargo fmt --all`, `cargo clippy --workspace --all-targets --locked
  -- -D warnings`, `cargo test --workspace --locked` (751 passed, 1 known
  ignored reproducer), `cargo build --release -p funfern-app --locked`.

## 2026-09-22 — The application can hold a driven document

- The assembly restructuring the previous entry identified. Both of the
  application's assemblies - the scalar quadratic one and the canonical one -
  are now given the law-stripped model, built once per preparation and kept,
  and the temporal operator is built over the base the canonical assembly
  produced rather than compiling a second one.
- `OwnedTopologyWaveModel::without_temporal_laws` is the stripping, in core
  where the gate it answers to lives. The temporal operator's base moved behind
  an `Arc`, so the application holds one assembly rather than two copies of a
  large operator; the test asserts pointer identity rather than taking that on
  trust.
- `PreparedTopology` carries `canonical_temporal_operator`, and `None` means
  inert. Every document that existed before drives answers `None` and takes the
  fixed path unchanged, which the same test pins down by preparing an inert
  document first and requiring the absence.
- The evidence is an application-level test that would have failed before this
  for a reason worth restating: not that a drive was unsupported, but that the
  document could not be *loaded*. It now prepares, carries its temporal
  operator, shares one base, and reports the tighter trajectory timestep.
- The non-resumable temporal compile is charged to the preparation's own
  assembly timing rather than hidden, so if it ever costs a frame the existing
  readout shows it. On the fixtures here it does not register against the
  assembly it reuses, which is what the shape predicted: per-sample law
  evaluation with no linear algebra.
- Still to do before a driven document actually runs: the two plan sites switch
  to `compile_temporal`, the timestep comes from the temporal ceiling, the
  runtime bank decodes against the live epoch origin, a drive being added or
  removed forces a fresh rebuild rather than a transfer, and AMR either takes
  the temporal supplement or holds the mesh with a stated reason.
- Checks: `cargo fmt --all`, `cargo clippy --workspace --all-targets --locked
  -- -D warnings`, `cargo test --workspace --locked` (751 passed, 1 known
  ignored reproducer), `cargo build --release -p funfern-app --locked`.

## 2026-09-22 — A law-carrying document cannot compile at all, which reshapes enablement

- Started wiring the application to run a driven scene and found the premise
  wrong. The plan assumed enablement meant "also compile a temporal operator
  beside the canonical one". It does not. **The fixed compiler refuses a
  law-carrying model outright** - that is the same gate that stops a law being
  executed as a static medium, enforced at `Material::evaluate` - so the
  application's canonical assembly would fail before anything temporal was
  reached. A document with a pump does not run today; it does not load.
- So the application's assembly has to be fed the *stripped* model, and the
  temporal operator built from the authored one over that same base. Which
  raised the second problem: `CanonicalTemporalWaveOperator::compile` builds
  its own base internally, so an application that already assembled one through
  its resumable job would compile the whole thing twice, synchronously, in a
  pipeline whose entire design is to stay responsive.
- `from_base` is the split that fixes it. Everything it does is per-sample law
  evaluation with no linear algebra, over a base the caller supplies. `compile`
  is now that base plus this, so there is one definition rather than two, and a
  test pins the two routes to the same trajectory bound, the same admission
  predicates and the same instantaneous coefficients.
- Both findings are recorded as a test rather than as prose:
  `a_law_carrying_scene_needs_a_stripped_base_the_temporal_operator_can_reuse`
  asserts that the fixed compiler refuses the scene, that the temporal one
  accepts it, and that a reused base agrees with a freshly compiled one.
- What this means for the remaining enablement work, which is now better
  understood than when it was planned: the application's canonical assembly job
  needs the stripped model, `PreparedTopology` needs to carry the temporal
  operator, the timestep must come from the temporal ceiling, and the two plan
  sites switch to `compile_temporal`. The restructuring of the assembly is the
  part the plan did not anticipate and is where the risk now sits.
- Checks: `cargo fmt --all`, `cargo clippy --workspace --all-targets --locked
  -- -D warnings`, `cargo test --workspace --locked` (750 passed, 1 known
  ignored reproducer), `cargo build --release -p funfern-app --locked`.

## 2026-09-22 — On the core anyone actually runs, the drive is free per step

- The CPU oracle's driven path costs about thirteen times its fixed path, and
  the obvious inference from that is that time-driven media are expensive. The
  inference is wrong, and the previous entry said the deciding measurement had
  not been made. It has now.
- `canonical_gpu_temporal_timing` runs the f32 GPU core with and without a
  drive on an identical fixture - same mesh, same operator, same initial state.
  On an M1 Max at 15270 degrees of freedom:

  | | us/step | simulated s per wall s |
  | --- | --- | --- |
  | driven | 516.9 | 1.7 |
  | fixed | 516.5 | 2.1 |

- **Per step the drive is free**, at a ratio of `1.001`. The shader reads a
  stage's coefficients once where the reference recomputes them in every helper
  that wants them, and that difference is the whole thirteen times.
- What a drive does cost is the timestep. The driven ceiling is `8.93e-4`
  against the fixed `1.097e-3`, because the CFL bound is taken over the whole
  coefficient trajectory rather than one set of coefficients. So the honest
  figure for the product is about `1.23x` wall clock per simulated second, and
  it comes from stability rather than arithmetic - which also means it shrinks
  with modulation depth instead of being a fixed tax.
- One caveat worth stating rather than leaving to be discovered: at this size a
  step may be dominated by dispatch and synchronization rather than by shader
  arithmetic. The equality therefore establishes that the drive's extra work is
  not the bottleneck, which is the operative question, rather than proving that
  work to be free in isolation.
- This changes what the CPU oracle's thirteen times means. It is a reference
  implementation's redundancy, it slows the test suite and the calibration
  examples, and it is worth fixing on those grounds - but it is not a statement
  about the feature's cost, and the earlier entry should be read with this one.
- Checks: `cargo fmt --all`, `cargo clippy --workspace --all-targets --locked
  -- -D warnings`, `cargo test --workspace --locked` (749 passed, 1 known
  ignored reproducer), and the timing pair on an M1 Max / Metal with `HOME`
  isolated from the live autosave.

## 2026-09-22 — A third of the driven cost was a walk taken for gaps that were not there

- The thin-gap composition reconstructed the midpoint primary field on every
  step to drift the gap displacement. That reconstruction is a walk over every
  node with a transcendental at each, and it ran whether or not the generation
  had any gaps - which is the ordinary case. Guarded, the driven bulk goes from
  `18.7x` the fixed bulk to `13.2x`, and the second-order wall from `37.5x` to
  `34.2x`.
- It was mine, introduced when gaps landed, and the cost measurement is what
  exposed it. Worth stating plainly rather than folding into the optimization
  work: taking the measurement immediately after the compositions found a
  regression that a later, more general optimization pass would have absorbed
  silently into its own improvement.
- The remaining ratio is still redundancy rather than physics, and still in the
  f64 CPU oracle rather than the production path. What has not been measured is
  the one that decides whether any of it matters to a user: the f32 GPU core's
  driven-versus-fixed cost. The shader reads its coefficients once per stage, so
  there is reason to expect a much smaller factor there, but that is an
  expectation and not a number.
- Checks: `cargo fmt --all`, `cargo clippy --workspace --all-targets --locked
  -- -D warnings`, `cargo test --workspace --locked` (749 passed, 1 known
  ignored reproducer).

## 2026-09-22 — The incremental cost is nineteen times, and most of it is redundant work

- Stage 7's exit criterion asks for the actual-core incremental cost of a
  time-driven medium to be recorded. `canonical_temporal_timing` records it per
  boundary composition, with the same mesh, operator, forcing and timestep on
  both sides so the only difference measured is the coefficient work.

  | Composition | Fixed us/step | Driven us/step | Ratio |
  | --- | --- | --- | --- |
  | bulk | 304 | 5679 | 18.7x |
  | bulk + source | 316 | 5937 | 18.8x |
  | thin gap | 310 | 5725 | 18.5x |
  | first-order wall | 333 | 6294 | 18.9x |
  | second-order wall | 942 | 35317 | 37.5x |

- The prediction was half right and the half that was wrong is the one that
  matters. The second-order wall does cost about twice the others' ratio, which
  is the per-stage Schur refactorization the previous entry called out, and that
  part is asymptotic rather than a constant. But the **floor is nineteen
  times**, before any boundary capability is involved, and that was not
  anticipated - the expectation recorded when the compositions landed was "a
  constant factor" in the sense of something small.
- The cause is not that evaluating a drive is expensive. It is that the same
  instantaneous factor is recomputed from scratch by every helper that wants
  it. One step calls `energy`, `complementary_energy_and_rate` twice,
  `force_at` twice, `primary_mass_at` at least three times,
  `primary_energy_and_rate`, `drift_at` and `energy_at`: roughly a dozen full
  walks over every primary contribution and every complementary sample, each
  evaluating a transcendental per entry, where the fixed path reads a table.
  Nothing about the physics requires that; the stage times repeat.
- **Reported, not fixed.** Caching a stage's factors once and passing them down
  changes the signature of most of the temporal operator's evaluation surface,
  and it is a performance change that wants its own before-and-after on this
  harness rather than being folded into the measurement that motivated it.
  The number to beat is on record now, which is the point of taking the
  measurement before the optimization rather than after.
- Worth keeping in proportion: this is the f64 CPU reference, which is an
  oracle rather than the production path. The app runs the f32 GPU core, where
  the shader reads its coefficients once per stage already. The cost that
  matters for interactivity is the GPU one, and that is a separate measurement
  this does not make.
- Checks: `cargo fmt --all`, `cargo clippy --workspace --all-targets --locked
  -- -D warnings`, `cargo test --workspace --locked` (749 passed, 1 known
  ignored reproducer), `cargo build --release -p funfern-app --locked`.

## 2026-09-22 — The GPU carries the forced composition, and the gate caught the kick dividing by the wrong mass

- The GPU work was not the port it looked like. `canonical_wave.wgsl` is one
  program that has carried sources, prescribed data, loss stages, thin gaps and
  both outgoing orders since the fixed path, and it already knows how to read
  an instantaneous coefficient. `compile_temporal_bulk` simply called the full
  static compiler with empty forcing and gated on the conservative bulk. So
  what was needed was an admission, not an implementation: `compile_temporal`
  takes a `CanonicalForcing` and gates on `forced_composition_supported`.
- That widening is also what made the defect reachable. Until it landed, a
  driven medium and a source, a wall or a gap could not be asked for together,
  so no stage had ever run with a moving nodal mass. A stage dividing by the
  authored mass is correct on every fixture that existed and wrong the moment
  the two meet.
- **The device gate found one on its first run.** `canonical_gpu_temporal_forced`
  puts a pumped medium, a volume source and an absorbing wall on the device and
  compares both state lanes against the f64 reference. First result: `Q` off by
  `1.439e-2`, `b` exact. Percent-level is the signature of a mass mismatch at a
  modulation depth of `0.24`, and `b` being exact localized it to the primary
  update rather than the drift.
- It was `kick_node`, reading `nodes[node].mass_loss.y` - the authored inverse
  mass - for the absorbing wall's admittance, the prescribed pin and both
  energy lanes. Those are the same three places the CPU composition had to make
  instantaneous, which is a useful corroboration: the same derivation, found
  independently on the two sides. Under `temporal_enabled()` the kick now takes
  the mass at the stage's own instant. The damping itself stays as assembled,
  which is the frozen reference impedance and a documented approximation rather
  than an oversight.
- After the fix, `Q` reads `5.978e-6` against the same oracle - a factor of
  2400 - and `b` is still exact. The change is guarded by `temporal_enabled()`,
  so the fixed path is untouched, and the two existing temporal device gates
  are unchanged: the work gate still reports `2e-7` on its Switch anchors and
  `1e-8` on its energies, and the AMR gate still reports a `4.6e-7` global
  indicator and `2.7e-7` state lanes.
- Histories start unexcited on a compiled generation, and that is refused
  explicitly rather than silently dropped: a nonzero thin-gap or pole-current
  history needs an initializer on this path.
- Checks: `cargo fmt --all`, `cargo clippy --workspace --all-targets --locked
  -- -D warnings`, `cargo test --workspace --locked` (749 passed, 1 known
  ignored reproducer), and three device gates on an M1 Max / Metal with `HOME`
  isolated from the live autosave.

## 2026-09-22 — The second-order outgoing boundary composes, and Stage 7's boundary work closes

- The last refused capability. A driven medium can now carry a second-order
  absorbing wall, with its pole currents as state and its energy in the total.
  Every boundary capability Stage 7 named - prescribed data, volume sources,
  loss, a first-order wall, thin gaps, a second-order wall - composes.
- Taken by extraction rather than by porting. The force-coupled outgoing kick
  is now a free function over explicit state - the flux, the pole currents, the
  factorization, the mass - and the fixed path's method is a thin wrapper
  around it. With the five map functions parameterized by mass in the previous
  commit, that means one implementation of roughly 250 lines of dense linear
  algebra serves both paths. Their agreement is structural rather than
  something to keep testing for, and the whole suite passing unchanged through
  both refactors is the evidence that neither changed behaviour.
- **The cost the shape predicted is real.** A fixed generation factorizes its
  Schur complement once, at construction, because the mass it is built from
  never moves. A driven one rebuilds it at every stage - twice per step -
  because the trace admittance, the modal couplings and the Schur complement
  all scale with the nodal mass. That is the first place in this work where the
  time-driven path is asymptotically more expensive than the fixed one rather
  than a constant factor over it, and it belongs in the incremental-cost
  measurement.
- One combination stays refused, and named rather than implied: prescribed data
  sitting on an outgoing trace. It is its own composition, it has had no tests,
  and the fixed path's handling of it involves a second cached factorization
  keyed by the prescribed pattern. Refusing it keeps the claim honest.
- Evidence. Inert parity is the strong one and it now covers the only state on
  this path that is neither a field nor a local spring: with nothing driven, a
  second-order wall's flux, complementary flux, pole currents and boundary-loss
  lane all match `CanonicalWaveState::step_with_forcing` to `1e-12` over a run
  that actually radiates. And a pumped medium keeps its second-order energy
  balance across two halvings with the wall carrying energy out, which is what
  says the per-stage refactorization is correct and not merely expensive.
- `conservative_bulk_supported` has now narrowed to exactly what its name says.
  It began as a gate refusing everything and ends as a statement about the
  absence of each exchange lane, with `forced_composition_supported` carrying
  what the stepper can actually run.
- Checks: `cargo fmt --all`, `cargo clippy --workspace --all-targets --locked
  -- -D warnings`, `cargo test --workspace --locked` (749 passed, 1 known
  ignored reproducer), `cargo build --release -p funfern-app --locked`.

## 2026-09-22 — The outgoing maps take their nodal mass as a parameter

- Groundwork for the last refused capability, landed on its own because it
  changes no behaviour and the whole suite says so.
- The second-order outgoing boundary's maps - the dense generator, its
  matrix-free application, the Schur-complement midpoint factorization and its
  solve - read `operator.primary_mass` in eight places. A time-driven
  generation's mass moves, so the trace admittance, the modal couplings and the
  Schur complement all move with it. Those five functions now take the mass as
  a parameter instead.
- The point is to avoid a second implementation. Porting roughly 250 lines of
  dense linear algebra to the temporal path would have left two copies of the
  same derivation to keep in step, and the agreement between them would have
  been something to test for. Parameterized, the fixed path passes
  `operator.primary_mass()` and a driven one will pass the mass in force at the
  stage, and their agreement is structural.
- No behaviour change, and the full suite is the evidence: 747 tests pass
  unchanged.
- What remains for the capability itself: the force-coupled outgoing kick is
  still a method on the fixed state, reading its own flux, auxiliaries and
  cached factor. It gets the same treatment next, and then the temporal state
  needs its own pole currents. One cost is already visible from the shape - the
  fixed path prepares its factorization once at construction, and a driven one
  cannot, because the mass it is built from changes every stage.
- Checks: `cargo fmt --all`, `cargo clippy --workspace --all-targets --locked
  -- -D warnings`, `cargo test --workspace --locked` (747 passed, 1 known
  ignored reproducer).

## 2026-09-22 — Thin gaps compose, and the gap's own store joins the energy

- The fourth boundary capability, and the smallest: a thin gap is a spring
  across a trace with a displacement of its own. One scalar per sample, a force
  that pushes the two sides apart in proportion to it, and a drift that
  integrates the field jump. The specification calls this a cheap exact local
  split and admits it to the structure claim once verified; this is that
  verification on the time-driven path.
- Two things had to be got right and both are checked rather than assumed. The
  gap stores `stiffness * jump^2 / 2`, which the bulk fields cannot account
  for, so it belongs to the state's total energy - leave it out and the balance
  charges a real store to the splitting remainder. And its drift belongs to the
  *same* subflow as the complementary flux's, over the same interval and on the
  same midpoint field; splitting them would cost the exactness the local split
  is admitted for. With both, a pumped medium with an open baffle keeps its
  second-order balance across two halvings.
- A new generation's gaps start closed, which is the unexcited physical history
  the specification asks for. A nonzero one needs an explicit initializer
  rather than being implied by zero bulk fields, and that is stated where the
  state is built.
- Inert parity is the guard again, and here it is the strongest form yet:
  primary flux, complementary flux *and* total energy all agree with
  `CanonicalWaveState::step_with_forcing` to `1e-12` over a run with the gap
  genuinely open. That covers the gap force, the gap drift and the gap energy
  in one statement.
- **Stage 7's boundary compositions are now done except one.** Prescribed data,
  volume sources, loss, a first-order absorbing wall and thin gaps all compose
  with a driven medium. Only the second-order outgoing boundary remains
  refused, and for a concrete reason: it carries pole currents that have no
  state on this path. `conservative_bulk_supported` has narrowed step by step
  as each capability landed, and now means what it says - the freely evolving
  Poisson system, with every exchange lane absent.
- Checks: `cargo fmt --all`, `cargo clippy --workspace --all-targets --locked
  -- -D warnings`, `cargo test --workspace --locked` (747 passed, 1 known
  ignored reproducer), `cargo build --release -p funfern-app --locked`.

## 2026-09-22 — A first-order absorbing wall composes, and what freezing its impedance costs

- Split the outgoing capability rather than taking it whole, after finding that
  the expensive part is not the part that carries the physics question. The
  172-line force-coupled outgoing kick belongs to the *second*-order boundary
  and its pole currents. A **first-order** wall is a local damping term in the
  same kick every other node already goes through - six lines - and it carries
  the identical frozen-reference-impedance question. So first-order outgoing
  composes now, and second-order stays refused until auxiliary state exists on
  this path.
- `boundary_loss` joins the accounting as its own lane. The admission predicate
  narrowed again: `conservative_bulk_supported` now additionally requires an
  undamped boundary, while `forced_composition_supported` allows one. The
  admission test is renamed to what it has become - the bulk claim narrows as
  each capability composes - and asserts both halves: a first-order wall steps,
  a second-order one still has no state.
- **The frozen impedance, measured.** A wall is assembled from the medium it
  was built against and stays there, so a medium that has since moved leaves it
  mistuned by that ratio. A Switch is the instrument, because it moves the mass
  to a new constant value and the mismatch is steady rather than smeared over a
  drive's cycle.
- Two confounds had to be removed before any signal appeared, and both are
  worth recording because either one alone produces a confident wrong answer.
  Cumulative escaped energy says nothing at all: reflected energy simply leaves
  on its next encounter, so over a long run a mistuned wall absorbs as much as
  a matched one - measured `0.945` against `0.942` across a twofold mismatch.
  And a heavier medium is slower by `sqrt(f)`, so comparing at a fixed clock
  scores a wave that has not yet reached the wall as reflected; that artefact
  alone produced an apparent fourfold effect at `f = 2.2`.
- With both removed, and comparing residual energy after one encounter at equal
  propagation distance:

  | Mass factor | Residual | Excess over matched | `R^2` predicted |
  | --- | --- | --- | --- |
  | 1.0 | 0.108 | - | - |
  | 2.2 | 0.112 | 0.003 | 0.038 |
  | 6.0 | 0.193 | 0.085 | 0.177 |

- So the approximation is **cheaper than the continuous normal-incidence
  coefficient predicts**, by about an order of magnitude at a realistic
  mismatch, and only approaches it at a sixfold one. That is the reassuring
  direction, but the number is not calibrated and the test does not assert it.
  A blob radiating into a square box is not a normal-incidence experiment, and
  the matched wall's own residual - the first-order condition's angular
  imperfection, `0.108` here - swamps a small mismatch. **Open**: a calibrated
  curve needs packet tracking, which is what `wave_boundary_reflection` already
  does for the fixed path.
- Inert parity again as the guard: with nothing driven, the absorbing wall damps
  exactly as `CanonicalWaveState::step_with_forcing` does, in the boundary lane
  and in the final state, to `1e-12`.
- A small finding on the way past: `QuadraticWaveOperator::assemble_regions`
  takes `outer_coefficients`, validates it, and then never passes it to
  `assemble_with_provider`. It is a dead parameter. That is also why the
  frozen-impedance cost could not be measured the cheap way, by assembling a
  wall tuned for one medium around another - there is no such knob, because the
  boundary damping is derived from the same node coefficients as the interior.
- Checks: `cargo fmt --all`, `cargo clippy --workspace --all-targets --locked
  -- -D warnings`, `cargo test --workspace --locked` (745 passed, 1 known
  ignored reproducer), `cargo build --release -p funfern-app --locked`.

## 2026-09-22 — Loss composes with a driven medium, and keeps second order doing it

- The second boundary capability. A dissipating generation now has a state and
  a step: the Strang dissipation map is composed around the conservative core
  with instantaneous rates, and `primary_loss` and `complementary_loss` join
  the accounting as their own lanes.
- **The specification's concession turns out not to be needed.** It accepts
  first-order accuracy for a varying loss rate, keeping second order only for
  the lossless step, on the grounds that the exponential is exact only when the
  rate is constant over the substep. That is true of the *rate*, but the
  accuracy of the composition is a separate question and it was worth
  measuring rather than inheriting: each half map sits at a step endpoint, so
  the temporal-work quadrature between those endpoints is untouched, but it
  stands for evolution over its own half interval, so its rate is read at that
  interval's midpoint rather than at the endpoint. With a pumped mass and a
  pumped loss rate the composed step's energy balance converges at **2.11 and
  2.03** across two halvings. Reading the rate at the endpoint instead would
  have been first order, and would have matched the concession.
- The state's admission moved with the stepper's. A state now exists wherever
  `forced_composition_supported` holds, which is anywhere without thin gaps,
  boundary damping or an open boundary; loss no longer prevents one from
  existing, it only costs the conservative-bulk claim.
  `bulk_symplectic_state_rejects_loss_and_open_boundaries` is renamed and
  narrowed to say that: it asserts the lossy generation now steps, and that the
  open one still does not.
- Passivity is checked per step rather than in aggregate - each lane must be
  nonnegative every step - and a negative removal is an error rather than a
  small number clamped to zero, because a passive channel adding energy is a
  defect in the rate and not a rounding artifact.
- Inert parity again, as the guard that the two paths mean the same thing: a
  constant-rate loss on an inert medium decays exactly as
  `CanonicalWaveState::step_with_forcing` does, in both lanes and in the final
  state, to `1e-12`.
- Still refused, each its own Gate B: thin gaps, first- and second-order
  outgoing boundaries. The frozen reference impedance under modulation belongs
  to the outgoing one and is next.
- Checks: `cargo fmt --all`, `cargo clippy --workspace --all-targets --locked
  -- -D warnings`, `cargo test --workspace --locked` (743 passed, 1 known
  ignored reproducer), `cargo build --release -p funfern-app --locked`.

## 2026-09-22 — A driven medium can now be driven: prescribed data and sources compose

- Until this, `CanonicalTemporalWaveState` had no forcing and no pulse. A scene
  could author a pump, a time crystal, a travelling modulation and a Switch,
  and then had no way to put any energy into the medium: the stepper refused
  anything but a closed conservative bulk. `step_with_forcing` composes
  prescribed data and volume sources, which is the first of the boundary
  capabilities Gate B lists and the one that makes a driven scene do anything
  at all.
- Admission is split in two, because what the stepper can compose and what the
  conservative-bulk claim covers are different questions. `forced_composition_supported`
  allows prescribed data and sources; `conservative_bulk_supported` additionally
  requires that nothing drives the boundary, and keeps its current meaning for
  the energy claim and the AMR supplement. Loss, thin gaps, boundary damping and
  open boundaries stay refused by both until each closes its own gate.
- **The stage equations, which is what this slice is really about.** A
  prescribed node pins `Q = M(t) g(t)`. Under modulation that means a constant
  `g` still moves flux, because the mass it is pinned against breathes. Holding
  `u` fixed while `M` varies gives the drive `-M' g^2 / 2` at fixed `Q` and the
  boundary `+M' g^2`, so the two lanes are both active and do not cancel; the
  measured ratio is `-2.00` as derived. The accounting therefore carries
  `temporal_work`, `source_work` and `prescribed_exchange` separately, and
  `splitting_residual` is what the three of them fail to explain.
- The falsifier earned its place again. Hold the whole field at one constant
  value through prescribed nodes everywhere, pump the mass, and the truth is
  known without a solver: the field is that constant, nothing radiates, and the
  energy is `0.5 M(t) g^2`. The first composition left `6.3e-6` per step
  unaccounted against an energy of `2.3e-1`. That could have been the
  second-order splitting remainder the field is named for, so the test measures
  the order rather than guessing a tolerance - and it came out at **0.99**,
  first order, which is a defect and not a remainder.
- The cause was a stage-instant mismatch. The fixed path pins a prescribed node
  half a step into the first kick, which is free to choose when the mass is
  constant. It is not free here: the temporal-work quadrature integrates
  between the step's endpoints, so a pin at an instant it does not know about
  leaves a first-order hole. Pinning at the stage instants - the same two the
  autonomous extension stages at - takes the order to **2.01** across two
  halvings.
- That change has a consequence worth stating rather than hiding: a prescribed
  node's initial flux must already satisfy `Q = M(t0) g(t0)`. The fixed path
  conceals a violation by pinning before its first kick and charging the
  correction to prescribed exchange. This one pins at the stage, so an
  inconsistent start shows up as a first step that does not balance.
  `CanonicalTemporalWaveState::pinned` builds a consistent state, and the
  requirement is in the doc comment rather than left to be discovered.
- Gate B evidence, kept to what the gate names rather than gold-plated. Stage
  equations and initialization are documented above. Passivity is the existing
  bulk suite, which now runs through the composed path with an empty forcing and
  is unchanged. Transient and fixed-CFL refinement are the order sweep. And the
  strongest one is cheap: an inert generation with a prescribed wall *and* a
  volume source reproduces `CanonicalWaveState::step_with_forcing` to `1e-12`,
  in both accounting lanes and in the final state, so the two paths mean the
  same thing by the same scene.
- Still refused, each its own gate: loss, thin gaps, first- and second-order
  outgoing boundaries. The frozen reference impedance under modulation belongs
  to the outgoing one, where the plan already allows it only as a documented,
  tested approximation. This is the f64 CPU reference only; the GPU path and its
  device gate follow.
- Checks: `cargo fmt --all`, `cargo clippy --workspace --all-targets --locked
  -- -D warnings`, `cargo test --workspace --locked` (741 passed, 1 known
  ignored reproducer), `cargo build --release -p funfern-app --locked`.

## 2026-09-22 — The transfer was never wrong; the oracle watching it was, twice

- **Correcting the entry two below.** It reported that a refinement transfer
  moves the state by `1.9e-4` against a `2.7e-7` input and called that a
  roughly seven-hundredfold amplification belonging to the transfer path. That
  is wrong. The transfer is sound. Both discrepancies were defects in the
  host-side oracle the gate compared it against, and the gate now measures the
  transfer directly rather than inferring it.
- The decisive experiment was to apply the same maps on the host to the
  device's *own* pre-transfer state, with no stepping either side, so nothing
  but the transfer itself is between the two answers. That separates the
  transfer from the trajectory in one number, which the earlier comparison
  could not.
- **First defect: the wrong transfer.** With identical input, host and device
  differ by `1.841e-4` on `Q` and exactly zero on `b`. The complementary
  transfer being bit-exact rules out precision as the explanation. The primary
  transfer can be asked to preserve each isolated component's total - the
  physical statement that interpolating a field onto another mesh must not
  create or destroy any of it - and the device does. The host oracle passed
  `None` and got the free transfer. Asked to conserve, the two agree to
  `5.505e-8`, and the component total is carried across exactly, `1.971303e-1`
  on both sides.
- **Second defect: a stale epoch origin.** With the transfer corrected the
  state agreed to `2.5e-7`, but the estimate's rate-sensitive terms were still
  percent-level apart while the flux terms agreed to `4e-8`. That split is a
  fingerprint: the fixture drives the mass row only, so a coefficient error
  lands in the energy and the rate and nowhere in the flux. The instantaneous
  mass differed by `4.077e-2`. A handoff rebases the clock - epoch 0 to 1,
  origin `0` to `4.648e-2` - and the gate was decoding the material runtime
  with the origin it had captured before the handoff. At `0.9 Hz` that stale
  origin displaces the carrier phase by `0.26 rad`, which at depth `0.22` is
  the four percent. Decoding against the live origin closes it to `7.2e-10`.
- With both fixed, the estimate is as good after a genuine refinement transfer
  as before one, and the gate now applies one set of bounds to both
  generations rather than carving the post-transfer terms out:

  | Term | Before refinement | After |
  | --- | --- | --- |
  | interior jump | `1.2e-7` | `3.7e-8` |
  | recovery | `1.1e-7` | `7.1e-8` |
  | total energy | `3.0e-7` | `3.4e-7` |
  | global indicator | `4.6e-7` | `4.0e-7` |
  | cell residual | `1.7e-5` | `8.4e-6` |
  | instantaneous mass | `9.5e-10` | `7.2e-10` |

- The gate keeps both measurements as checks rather than as comments. It
  requires the device to be performing the conserving primary transfer, by
  measuring both candidate transfers and requiring the conserving one to win.
  And it decodes the runtime against `display.clock`'s own origin, so the
  staleness cannot come back.
- **An API hazard worth a decision, not fixed here.**
  `CanonicalGpuDisplay::material_runtime` takes `epoch_origin_seconds` as a
  parameter while the same `CanonicalGpuDisplay` already holds the
  authoritative value in `self.clock`. A caller that stores the origin once -
  which is the natural thing to do, and what `canonical_gpu_temporal_work`
  does - is silently wrong the moment a handoff rebases the clock. That gate
  performs no handoff so it is correct today, but the shape invites the bug.
  Defaulting to the display's own clock would remove it. That is a public
  signature change, so it is reported rather than folded in.
- The lesson for the log: an oracle is code too. Two of the three surprises in
  this AMR work turned out to be the measurement rather than the thing
  measured, and in both cases the tell was a term that agreed far better than
  its neighbours - the complementary transfer being exact, and the flux terms
  holding while the mass-dependent ones drifted.
- Checks: `cargo fmt --all`, `cargo clippy --workspace --all-targets --locked
  -- -D warnings`, `cargo test --workspace --locked` (739 passed, 1 known
  ignored reproducer), `cargo build --release -p funfern-app --locked`, and
  `canonical_gpu_temporal_amr` on an M1 Max / Metal.

## 2026-09-22 — One accuracy target, one true accuracy, on both estimator paths

- The driven estimate is multiplied by `DRIVEN_INDICATOR_CALIBRATION = 1.88`
  wherever the gradient terms have moved onto the solver's flux, so a single
  accuracy number means the same true error on the static and the driven path.
  The alternative was carrying two targets that deliver the same accuracy at
  different numbers, which puts the burden on whoever reads the slider.
- Measured the production static estimator on the same fixture before setting
  the constant, rather than reusing the pre-substitution inert row as a stand-in
  for it. The calibration example now runs `canonical_indicator_supplement` with
  no runtime as its first row - which is exactly what the application runs
  today - and it reads 1.538, 1.262, 1.360, matching the pre-substitution inert
  row as the existing parity test says it must. Two indices only compare if one
  study produced both.
- The constant is the ratio of geometric means: 1.3820 static over 0.7339
  driven, the latter over seven media crossing driven row, spatial pattern and
  modulation wavenumber.
- What it buys, per row, after scaling:

  | Row | Index | Spread |
  | --- | --- | --- |
  | static path, inert | 1.54, 1.26, 1.36 | 1.22x |
  | inert | 1.51, 1.28, 1.36 | 1.18x |
  | mass pumped | 1.62, 1.37, 1.46 | 1.18x |
  | mass travelling, `k=0.75` | 1.55, 1.36, 1.43 | 1.14x |
  | mass travelling, `k=3` | 1.41, 1.24, 1.26 | 1.14x |
  | stiffness pumped | 1.54, 1.30, 1.39 | 1.18x |
  | stiffness travelling, `k=3` | 1.47, 1.32, 1.38 | 1.12x |
  | both travelling, `k=3` | 1.35, 1.21, 1.25 | 1.12x |

- On an inert medium the substituted estimate now reproduces the static one to
  within 2 percent, which is the check that matters most: the two estimators
  differ in which field they differentiate, and on a medium where that should
  not matter they agree. Across all seven driven media the scaled index runs
  1.21 to 1.62, against the static estimator's own 1.26 to 1.54 across three
  meshes on one medium. The substituted estimate is no more scattered than the
  one it has to agree with.
- Called a calibration rather than a correction, deliberately. Neither index is
  one. The static 6 percent target was set by watching a production run settle
  and delivers about 4.4 percent true error; the claim here is only that the
  driven path now delivers the same, not that either estimate is unbiased.
- Guarded by a test that recomputes the factor from the report's own residual
  and energy, so the constant cannot drift away from the number its
  documentation cites without a failure.
- The application still runs the static path, so nothing user-facing moves
  today. That is also why this was worth doing now rather than at enablement:
  an optimistic error estimate sitting in the core is a footgun for whoever
  wires the driven path up, and the measurement is fresh.
- Re-ran the device gate after the change: unchanged relative errors, with the
  reported indicator now `1.689e-1` where it was `8.98e-2`, exactly the
  constant.
- Checks: `cargo fmt --all`, `cargo clippy --workspace --all-targets --locked
  -- -D warnings`, `cargo test --workspace --locked` (739 passed, 1 known
  ignored reproducer), `cargo build --release -p funfern-app --locked`, and
  `canonical_gpu_temporal_amr` on an M1 Max / Metal.

## 2026-09-22 — The estimate holds on a real device, and a refinement transfer moves the state more than expected

- `crates/funfern-app/examples/canonical_gpu_temporal_amr.rs` runs the error
  estimate against the f32 state an M1 Max actually produces, on a travelling
  mass modulation - the medium that used to take the efficiency index from 1.4
  to 17.4 before the estimate moved onto the solver's own flux. The
  instantaneous coefficients come from the material runtime decoded out of the
  same buffer copy as the state, which is what the runtime bank travels in the
  snapshot for.
- On the accepted generation the estimate is device-proof, and the margins fall
  exactly where the previous entry predicted they would:

  | Term | f32 against the f64 oracle |
  | --- | --- |
  | interior jump (flux) | `1.2e-7` |
  | recovery (flux) | `1.1e-7` |
  | total energy | `3.0e-7` |
  | global indicator | `4.6e-7` |
  | boundary residual | `1.4e-6` |
  | worst element indicator | `5.3e-4` |
  | **cell residual** | **`1.7e-5`** |

- The cell residual is the worst term by two orders, and it is the one term
  that still differentiates the nodal primary quotient `Q/M` - twice, through a
  Hessian. The previous entry named it as the remaining exposure before this
  was measured. `canonical_primary_rate` has carried the same warning about f32
  cancellation all along. The flux terms, which come straight from stored
  complementary state, are the best-behaved things in the table.
- **The transfer, not the estimator, is what a genuine refinement exposes.**
  The gate then refines where the estimate asks - 605 elements to 1604, driven
  by the estimator's own size field - hands the generation over on the device,
  and steps the new one. The flux terms still agree with the f64 oracle to
  `2e-6`. The rate-sensitive terms do not, and the reason is upstream of the
  estimate: the device's transferred state differs from a host application of
  the same transfer maps by `1.9e-4` on `Q` and `1.7e-5` on `b`, against
  `2.7e-7` on the same lanes before the transfer. The transfer amplifies its
  input difference by roughly seven hundredfold.
- Ruled out before concluding that. The device clock agrees with the assumed
  time to `1.8e-7` and the accepted step count is exactly `48 + 8`, so the
  instantaneous coefficients are not being evaluated at the wrong instant. The
  maps are literally the same objects the GPU plan was compiled from.
- `canonical_gpu_temporal_handoff` could not have found this: it transfers
  through an identity map on one mesh, so there is nothing to amplify. Whether
  `1.9e-4` is the honest cost of an f32 interpolation with redistribution or a
  defect in the transfer is open, and it belongs to the transfer path rather
  than to this slice.
- The gate therefore bounds, across the transfer, only the flux terms and the
  transferred state itself, and says in the source why. Bounding the
  rate-sensitive terms there would be asserting the transfer's precision under
  the estimator's name. It does assert that refining where the estimate asked
  lowers the estimate, `8.98e-2` to `6.83e-2`, because a size field that did
  not would mean the indicator and the error term disagree about where the
  error is.
- One incidental defect the gate surfaced: the estimator's size bounds and the
  adapter's have to agree, or the adapter rejects the size field outright
  (`InvalidTarget` at a target of `0.037` against a floor of `0.045`). They now
  read the same two constants in the fixture. Production carries the same
  hazard wherever those two option sets are filled in independently.
- `docs/checks.md` documented four application flags that do not exist -
  `--mesh-edit-benchmark`, `--wave-gpu-check`, `--wave-transfer-check` and
  `--amr-check`. The unified-topology cutover removed them and the doc was never
  updated, so anyone following it ran four commands that cannot work. Corrected,
  along with the `temporal_amr_calibration` description, which still described
  four sweeps and the superseded conclusion.
- Checks: `cargo fmt --all`, `cargo clippy --workspace --all-targets --locked
  -- -D warnings`, `cargo test --workspace --locked` (739 passed, 1 known
  ignored reproducer), `cargo build --release -p funfern-app --locked`, and the
  new gate on an M1 Max / Metal with `HOME` isolated from the live autosave.

## 2026-09-22 — The estimate reads the solver's own flux, and is calibrated on every medium tried

- Under a material runtime the gradient-based error terms no longer
  differentiate the reconstructed scalar field. The interior flux jump and the
  displacement recovery step aside for the complementary flux jump and the
  complementary recovery, both taken from the state the solver actually stores.
  Without a runtime nothing changes, so the static path keeps its behaviour and
  its `6%` target untouched.
- Built the falsifier first and it did its job twice. The first version
  measured the jump in `(S b) . n` and read `1e-32` against a scalar jump of
  `1e-5` - structural zero, not a small error. The direct state keeps its
  complementary variable in a quarter-turn-rotated frame, the reference map
  being `rotate_tensor(stiffness)`, and `b` evolves from the curl of the
  primary field, so `(S b) . n` across a face is a tangential derivative of a
  single-valued edge trace and is identical from both sides by construction.
  The informative component is the tangential one, which is the scalar
  `[[A grad(u) . n]]` carried through that rotation.
- With the component fixed, the decision rule set in advance was met. The
  candidate converges at `h^4` on every medium, including the two where the
  scalar jump stalls:

  | Medium | Scalar jump | Flux jump |
  | --- | --- | --- |
  | inert | `h^4.26` | `h^4.19` |
  | mass travelling, `k=3` | `h^1.19` | `h^4.00` |
  | both travelling, `k=3` | `h^1.46` | `h^3.96` |

- After the substitution the efficiency index is near enough medium-independent
  over 15x the unknowns, which it has never been before:

  | Medium | Index | Spread |
  | --- | --- | --- |
  | inert | 0.80, 0.68, 0.73 | 1.18x |
  | mass pumped | 0.86, 0.73, 0.78 | 1.18x |
  | mass travelling, `k=0.75` | 0.82, 0.72, 0.76 | 1.14x |
  | mass travelling, `k=3` | 0.75, 0.66, 0.67 | 1.14x |
  | stiffness pumped | 0.82, 0.69, 0.74 | 1.18x |
  | stiffness travelling, `k=3` | 0.78, 0.70, 0.74 | 1.12x |
  | both travelling, `k=3` | 0.72, 0.64, 0.67 | 1.12x |

- Every spread is now tighter than the inert control's own 1.22x was before,
  and the drift with refinement is gone. That drift was the real damage: a
  fixed target used to map to a different true error at every mesh, so a
  controller would have refined at the modulation pattern and never settled.
- **The level moved and the constant is no longer near one.** The index sits
  around 0.7 rather than around 1.4, so the estimate now reads about 1.4x
  optimistic where it used to read pessimistic. That is a consequence of
  dropping the scalar displacement recovery, which carried real magnitude on
  the inert control too. It is a calibration constant, not a drift: it is the
  same 0.64 to 0.86 across seven media that differ in driven row, spatial
  pattern and wavenumber. A driven accuracy target has to be set against this
  number rather than inheriting the static `6%`, and until that target is
  chosen the driven estimate is optimistic, which is the opposite of the
  direction the previous entry could claim.
- Chose this over the two alternatives for a reason that outlives the defect.
  `S b` at an element's own six samples is exactly the form a Stage 8 nonlinear
  constitutive inverse can produce; `A grad(u)` at an interpolated point is not,
  which is why the consumers were realigned in the first place. Inverting the
  mass consistently would have made the estimator measure a field the solver
  does not have, and reporting only the canonical terms on a patterned mass row
  would have made one number mean different things on different scenes.
- The substitution is gated on the supplement declaring it carries a flux jump,
  not merely on a runtime being present. `element_complementary_jump` is an
  `Option`, `None` on the fixed path. A runtime paired with a fixed supplement
  therefore keeps the scalar terms rather than silently replacing them with
  zeros, and a supplement carrying the term without a runtime does not get it
  counted on top of the scalar jump. Both directions are tested.
- The scalar displacement recovery is still computed and still reported in the
  breakdown while excluded from the total, because that column is what shows
  the substitution actually happened.
- Still open, and unchanged by this: the strong cell residual continues to
  differentiate the nodal quotient `Q/M`. It is small on these fixtures - the
  drift column runs `1e-16` against a jump of `1e-7` - so it does not bind
  here, but it is the same construction that broke the other two terms and it
  would bind on a medium that excites it.
- Checks: `cargo fmt --all`, `cargo clippy --workspace --all-targets --locked
  -- -D warnings`, `cargo test --workspace --locked` (739 passed, 1 known
  ignored reproducer), `cargo build --release -p funfern-app --locked`. The
  temporal path is still dormant in the app, so no production estimate changes
  with this.

## 2026-09-22 — The estimator samples the instant, and what breaks it is narrower than reported

- Landed the approved fix: the scalar estimator's material samples are now
  evaluated at the instant the snapshot belongs to, through
  `SolutionIndicatorJob::with_instantaneous_materials`. Every error term reads
  them - vertex stiffness for the recovery, the six interior samples, the
  stiffness divergence, the edge samples the interior jump compares across a
  face, and the boundary residual.
- Per sample point, not per element. The cheaper per-element factor was the
  other option offered and it would have been wrong for the case in question:
  the divergence term differentiates the stiffness across an element and the
  jump term compares two elements at a shared edge point, so a factor held
  constant over an element erases exactly the modulation gradient those terms
  exist to see.
- It costs no extra material evaluation. One lookup yields the authored
  coefficients and the instant's, because the instantaneous form is the
  authored one with each solver row scaled: the primary row multiplies the
  mass, the complementary row is the reciprocal of the stiffness tensor so its
  factor divides that tensor. Which authored law owns which row is the physics
  skin's business, and `coefficient_for` already answered it, so this holds for
  Mechanical, TM and TE without a case of its own. The added work is two factor
  evaluations per sample point per estimate.
- `Material::evaluate` refuses a law-carrying material on purpose, so that
  nothing executes an authored law as a static medium by accident. That gate is
  why the scalar path has always been handed a law-stripped scene. Rather than
  weaken it, there is now an explicit `evaluate_base` that a consumer applying
  the law itself declares it wants, and the instantaneous evaluator refuses a
  medium whose law reaches past the two constitutive rows - a loss channel or a
  restoring law - which is the same line the temporal supplement draws.
- The wavelength limit deliberately keeps the authored wave speed. A limit that
  breathed with the drive would retarget the same element every cycle; a
  drive's reach belongs to `resolved_frequency_hz` and
  `coefficient_wavelength`, which the caller already supplies for it. A
  trajectory-minimum speed for that limit is a separate question, left open.
- Locked down by two tests. A uniform pump is exactly a scene whose coefficient
  was authored at the pumped value, and every error term now agrees with that
  scene's to `1e-12`, which makes the instantaneous samples a reconstruction of
  the medium rather than a correction to it. And an inert medium's report is
  unchanged by supplying a runtime at all.

- **The previous entry's mechanism was wrong, and so was its conclusion about
  what breaks.** It attributed the inflation to the estimator's static material
  samples and named the spatial pattern as the cause. Both fail on measurement.
  The two driven sweeps it compared differed in the driven row *and* in the
  spatial pattern, so neither could be attributed. Crossing them says:

  | Medium | Efficiency index | Spread |
  | --- | --- | --- |
  | inert | 1.54, 1.26, 1.36 | 1.22x |
  | mass pumped | 2.08, 1.71, 1.86 | 1.21x |
  | mass travelling, `k=0.75` | 2.53, 3.28, 5.10 | 2.01x |
  | mass travelling, `k=3` | 7.77, 10.24, 17.92 | 2.31x |
  | stiffness pumped | 1.62, 1.38, 1.46 | 1.17x |
  | stiffness travelling, `k=3` | 1.85, 1.66, 1.74 | 1.11x |
  | both travelling, `k=3` | 8.05, 10.94, 17.21 | 2.14x |

- A spatial pattern is not the cause: a travelling drive on the stiffness row
  is the best-behaved sweep in the table. The mass row is not the cause either:
  pumped uniformly it matches the inert control. It is the two together, and
  the size of it scales with the pattern's own wavenumber - at `h=0.1` the jump
  term sits at `5.4e-7` inert, `6.5e-6` at `k=0.75` and `1.4e-4` at `k=3`,
  about `k^2` across a fourfold change, which is the signature of a term
  carrying `|grad m|^2`.
- Making the samples instantaneous did not fix it, and could not have: in the
  mass-travelling sweep the stiffness row is not driven, so the tensor the jump
  term reads was already correct. What the change did fix is the energy
  denominator, which had been dividing by an authored mass while the field was
  driven.
- Where it does come from, on the evidence. The scalar field handed to the
  estimator is the solver's nodal quotient `u = Q / M(t)`, and `M` is the
  lumped instantaneous primary mass, assembled by row sum. A spatially uniform
  factor cancels in that quotient exactly. A patterned one does not: the lumped
  inverse is not the consistent one, and the discrepancy is patterned at the
  modulation wavenumber. It is nearly invisible in the field's own norm - true
  error still converges at about `h^1.8` - and very visible to anything
  differentiating that field across a face. On the same `k=3` run the scalar
  interior jump converges at `h^1.2` and the scalar displacement recovery at
  `h^1.5`, while the canonical complementary recovery, which never passes
  through the nodal quotient and inverts at each element's own six samples with
  the instantaneous map, converges at `h^3.9`. Four independent legs agree, so
  this is the best-supported reading rather than a proven one.
- Still not calibrated for a patterned mass row, and the practical damage is
  the drifting index rather than the inflation: a fixed target maps to a
  different true error at every mesh, so a controller would refine at the
  modulation pattern and never settle. The estimate stays conservative, never
  optimistic, and the size rule's pattern limit is what protects the case
  today. Reported rather than fixed, because every remedy is a design change
  to which field the estimator differentiates - taking the gradient terms from
  the canonical complementary flux where a runtime is present, inverting the
  mass consistently for the estimator's own reconstruction, or reporting only
  the canonical terms on a patterned mass row - and that is not a measurement
  commit's decision to make.
- The equivalence test now runs on both constitutive rows, which it did not at
  first. Pumping only the mass row left the complementary row's direction
  untested, and the two scale opposite ways: the primary row multiplies the
  mass, while the complementary row is authored on the reciprocal stiffness, so
  its factor divides the stiffness tensor. Both rows now agree with a
  hand-authored equivalent scene to `1e-12`, deriving each factor from the
  operator's own two evaluations of that row rather than from the path under
  test. That also settles the roles for the work ahead: the stored `b` is the
  gradient-like variable and `S b` is the flux, so the analogue of the scalar
  interior jump is a jump in `S b`, not in `b`.
- Checks: `cargo fmt --all`, `cargo clippy --workspace --all-targets --locked
  -- -D warnings`, `cargo test --workspace --locked` (738 passed, 1 known
  ignored reproducer), `cargo build --release -p funfern-app --locked`.
  `crates/funfern-core/examples/temporal_amr_calibration.rs` carries the
  crossed design and now prints why an estimate was refused instead of leaving
  a blank column.

## 2026-09-22 — The error estimator is not calibrated for a patterned medium

- Measured the efficiency index, estimator over true error, across a
  refinement sequence, which the material-law review requires before the
  static `6%` target is quoted for driven media. The static number had itself
  only ever been checked by watching a production run settle, so the inert
  sweep is the control rather than the assumption.
- The study is a bare rectangle with a reflecting-box mode released from rest,
  so the initial state satisfies the walls exactly and the solution stays
  smooth. A first attempt on the starter scene converged at `h^0.3` and gave a
  meaningless index; a curved hole and data that fights the boundary limit
  convergence by geometry, not by the scheme. Every mesh starts from the same
  analytic data and runs at the finest mesh's timestep, so what differs is
  space, and is compared on a lattice belonging to no mesh.
- Isolating which kind of drive breaks it settles the cause. Efficiency index
  over a 15x range of unknowns:

  | Medium | Index | Spread |
  | --- | --- | --- |
  | inert | 1.54, 1.26, 1.36 | 1.22x |
  | stiffness pumped, uniform in space | 1.65, 1.41, 1.49 | 1.17x |
  | mass travelling, patterned in space | 7.53, 9.92, 17.36 | 2.31x |
  | both | 7.62, 11.30, 17.62 | 2.31x |

- So it is the spatial pattern, not the time dependence. A drive that varies
  only in time leaves the estimator as calibrated as the inert one. A
  travelling drive inflates it about thirtyfold and makes the index climb with
  refinement, which is worse than the inflation: a fixed target maps to a
  different true error at every mesh, so the controller would chase it and
  never settle.
- The dominant term is the interior flux jump, which converges at about
  `h^1.3` when patterned against about `h^4` inert. The displacement recovery
  behaves the same way. The mechanism follows: the estimator's per-element
  material samples are the static ones, so the jump is the discontinuity of
  `A_static . grad(u_driven)`, whose mismatch against the flux that actually
  produced the field is itself patterned at the modulation wavenumber. That
  mismatch shrinks only as the mesh resolves the pattern.
- Separated two roles that one option had been doing, which is how this became
  visible. `forcing_frequency_hz` is the spectral scale that converts the
  complementary recovery channel and weights the energy denominator, and stays
  the driving frequency. The new `resolved_frequency_hz` is what the size rule
  must resolve and takes the modulation reach. Feeding the reach into the
  former inflated the energy denominator by its square and partly masked the
  jump inflation; the inert sweep is unchanged by the separation, so it exposed
  the defect rather than causing it. This supersedes the previous entry's
  wiring, where the demand went into `forcing_frequency_hz`.
- Not fixed. Making the scalar estimator's material samples instantaneous is a
  change to how the resumable job builds them, with a cost decision in it, so
  it is reported rather than folded in here. Until then the estimator is
  conservative on a patterned medium, never optimistic, and the size rule's
  pattern limit is what actually protects that case.

## 2026-09-22 — Modulation-aware mesh sizing and estimator inputs

- Started modulation-aware AMR from the two places a driven medium breaks the
  existing estimator: what the mesh must resolve, and what the residual is
  measured against. The refinement and coarsening controller, its hysteresis
  and its resumable scheduling are untouched, which is what the review asked.
- A source frequency alone cannot size a mesh in a driven medium. Mixing puts
  energy at `f_source +/- n f_drive`, so the resolved frequency now adds the
  drive's reach: sideband order from the modulation depth against a stated
  one-per-cent amplitude floor, multiplied by the drive's own harmonic order.
  A cosine pump has one harmonic; a smoothed square carries odd harmonics on
  a scale set by its sharpness, so a sharpened square reaches further than a
  cosine of the same depth. Both are bounded at eight orders, which is a
  guard against a pathological authored depth rather than a physical limit.
- A travelling modulation additionally writes a spatial pattern into the
  coefficients, period `2*pi/q`. That binds the mesh through the same
  elements-per-wavelength rule whether or not a wave is present, because a
  mesh too coarse for the pattern is assembling the wrong operator and no
  error estimate on the field would say so. It is the more rigorous of the
  two rules and needed no threshold.
- The demand is computed from authored materials as well as from a compiled
  operator, so the application's size rule is correct the moment drives
  become authorable rather than needing a second change then. The controller
  already passes it.
- Added the variable-coefficient supplement. Endpoint fields divide by the
  mass in force at their own endpoint, the drift the residual is measured
  against uses those, and the energy and recovery norms use the instantaneous
  constitutive inverse. Reusing the authored coefficients would charge the
  estimator for the medium's own modulation and refine against it. Only the
  conservative bulk is covered, which is what the temporal path executes; an
  operator carrying loss, gaps, open boundaries or prescribed data is refused
  rather than reported with those terms missing.
- Two fixtures hold it: on an inert medium the temporal supplement reproduces
  the fixed one to `1e-12` relative in energy, drift and recovery, so the
  temporal path cannot report different errors for the same physics; on a
  driven one it departs by more than `1e-3`, so the fixture cannot pass
  without exercising the new evaluation.
- The midpoint proxy is unchanged and still approximates the half-kicked
  field by averaging the two endpoint fields, an `O(dt^2)` mismatch the fixed
  path has too. What changed is that the two endpoints no longer share a mass.
- An end-to-end fixture drives the indicator on a travelling modulation of
  wavenumber 24 over a mesh at 0.25, with the field at `1e-9` so no accuracy
  estimate has anything to say. Every element target comes back at or below
  the pattern limit, refinement is asked for, and not one of those requests
  is an accuracy decision. That is the case a field-based estimator cannot
  see at all.
- Not claimed: the calibrated relative-error percentage has not been
  re-validated for driven media, as the review requires before that number is
  quoted for them. The remaining AMR work is that calibration and a
  real-device gate over an adaptive run on a modulated scene.

## 2026-09-22 — The accepted runtime travels with the state

- Closed the temporal-work accounting gate. The accepted material runtime
  bank is now published into the state buffer after the metadata word, by the
  same commit that writes the fields around it, so a full snapshot carries
  the anchors and Switch trajectories that explain those fields. A separately
  arriving runtime copy would have been status rather than snapshot identity,
  which is why it is not a second readback.
- The host decodes the bank into a runtime state by adopting the solver's
  anchors and Switch onto the operator's own material set: the IDs and names
  come from compilation, only the runtime values move. A Switch origin is
  stamped at a GPU commit boundary and a carrier is re-anchored at one, so
  neither is reconstructible from the clock.
- The canonical state layout version therefore moves to 4. The bank is three
  words per material for the accepted slot only, and every existing
  publication site picks it up because the copy lives inside
  `publish_snapshot_metadata` rather than beside its callers.
- A new hidden gate stamps a Switch mid-run, evolves past it, then compares
  the decoded runtime and the energy breakdown against the f64 oracle. On
  Apple M1 Max / Metal over 1,934 Q and 3,630 b: Switch start time `2.004e-7`,
  duration `2.649e-8`, primary energy `8.249e-9`, complementary energy
  `4.549e-9` and pump power `1.428e-8`, all relative.
- Checked that it has teeth by suppressing the publication: the Switch
  becomes invisible, its blend reads 0 to 0 instead of 0 to 1, and the
  reported pump power is 41% wrong with the energy split 1.2% wrong. That is
  the error the readback removes.
- `target` is a reserved word in WGSL and the naga suite caught it before the
  device did, which is the second time that test has paid for itself.
- Re-ran every hidden GPU gate after the layout change, since all of them
  read the state buffer. All six pass on Apple M1 Max / Metal with their
  recorded figures: the bulk 96-step run at `3.882e-7` Q and `4.172e-5` b,
  handoff at `2.986e-7` and `2.090e-6`, rollback at `2.645e-7` and
  `2.426e-6`, the four consumers unchanged, temporal work as above, and the
  filter boundary at `4.140e-7` field and `1.094e-6` rate.
- Formatting, strict workspace Clippy, the workspace suite and a native
  release build pass. The Stage 7 diagnostic gate is now closed. What remains
  in Stage 7 is modulation-aware AMR, supported boundary compositions under
  modulation, and the incremental-cost measurement.

## 2026-09-22 — Recorders step past a filter commit

- The point recorder and the far field now take a sample one step later when
  its due step is a resident grid-filter commit, which is what the adaptation
  controller already did at the same boundary. The other consumers read only
  the accepted lane and keep their own cadence.
- The deferred sample keeps its ring slot, because `step / stride` does not
  change over the extra step when `step` is a multiple of `stride`. With a
  stride of one there is no later step to move to and the boundary sample is
  dropped instead. A host test covers the cadence arithmetic, including that
  every sample is still taken and that the slot is unchanged.
- Added a real-device regression on the worst case rather than a rare one:
  `canonical_gpu_filter_boundary` runs a 120 Hz point probe at the step
  `paced_time_step` gives for `0.0625x`, so the stride is exactly the
  sixteen-step cadence and before the fix every sample collided. On Apple M1
  Max / Metal the sample now matches the f64 oracle to `4.140e-7` in field and
  `1.094e-6` in rate. The fixture prints what differencing at the commit would
  have given: `-3.436e-3` against a true `-4.608`, so 0.07% of the truth.
- Checked that the regression has teeth by reverting the dispatch change: it
  fails, and reports that the last sample sat on the commit rather than one
  step past it.
- This is a static-path behaviour change. Rates sampled on a filter commit
  previously read near zero and now read correctly, one step later than
  before; nothing else about the traces moves.
- Formatting, strict workspace Clippy, the workspace suite and a native
  release build pass.

## 2026-09-22 — Temporal work, and a rate defect at filter boundaries

- Added the f64 temporal energy breakdown: bulk energy split into primary and
  complementary storage, with the power the authored material trajectory is
  pumping into it. The power is the analytic partial time derivative at fixed
  canonical state, so one snapshot suffices and no history is kept; a test
  pins it against a central difference of the energy with the state held
  still, and a composed system carrying loss or an open boundary is refused
  rather than summarised with its other exchanges missing.
- This deliberately stops short of the GPU. The accounting gate needs the
  accepted material runtime bank read back in the same command stream as the
  state, because a GPU-stamped Switch origin is not something the host can
  reconstruct, and a separately arriving runtime copy is status rather than
  snapshot identity. The f64 contract above is what that readback will be
  compared against. The accounting gate is not closed.

Measured while preparing the event-boundary work, and fixed in the entry
above:

- [x] A probe rate sampled exactly on a resident grid-filter boundary is
  wrong by about its own magnitude. The spare state lane holds the pre-filter
  value at the *same* instant, not the endpoint one `dt` earlier, so the
  difference the recorder forms is a filter correction divided by `dt`. On a
  driven h=0.2 fixture at the cadence boundary the true rate was `4.008e-1`,
  a recorder reading that lane would report `6.016e-3`, and the neighbouring
  ordinary step was `4.478e-1`. The reported value collapses to about 1% of
  the truth.
  Only quantities that difference the two lanes are affected: the point
  probe's rate and the far field's `rate`, which feeds its Kirchhoff
  integrand. Field, energy, flow and complementary magnitude read the accepted
  lane alone and are exact. The vector overlay already handles this boundary
  deliberately, carrying the pre-filter complementary field for its
  presentation high-pass.
  This is pre-existing on the static path, not something the temporal work
  introduced.

  How often it bites is set by the speed control, which is worse than a first
  look suggests. `paced_time_step` makes the step `speed/120` below about
  `0.3x`, so the recorder stride tracks the slider directly and can share a
  factor with the sixteen-step cadence. A sample is corrupted every
  `16/gcd(stride, 16)` readings:

  | Speed | Point probe | Far field |
  | --- | --- | --- |
  | 0.3x and above | 1 in 16 | 1 in 16 |
  | 0.25x | 1 in 4 | 1 in 2 |
  | 0.125x | 1 in 2 | every sample |
  | 0.0625x | every sample | every sample |
  | 0.05x | 1 in 4 | 1 in 2 |

  So slow motion, which is when a probe trace is most likely being read
  closely, is where it is worst, and at two speeds inside the `0.02..=2.0`
  slider every single reading is affected. Measured magnitudes over eight
  probe points and four boundaries: the reported rate was between 0.03% and
  18% of the truth, median about 0.05%. It is always a collapse toward zero,
  never an overshoot, so it reads as a dropout rather than a glitch, and the
  field trace beside it stays correct.

  Fixed the same day; see the entry above.

## 2026-09-22 — Far-field exterior must be time-invariant

- Far-field compilation now rejects an exterior medium whose response varies
  in time, by name, alongside the existing non-uniform, anisotropic, lossy and
  driven refusals. The retarded Kirchhoff projection integrates over a
  homogeneous linear time-invariant exterior; a modulated one has no such
  Green's function, and feeding instantaneous coefficients into the static
  formula would have produced a plausible wrong pattern rather than a failure.
- The check is one predicate on the authored material, covering a field law, a
  drive or a Switch alternate on either constitutive row and a drive on either
  loss channel. That answers Stage 8's nonlinear case with the same rule, so
  the exterior policy does not need revisiting when field-dependent laws
  arrive.
- A spatially varying, anisotropic or lossy exterior is still judged by its own
  existing rules, which the predicate deliberately does not touch. Driven
  material *inside* the contour remains a different and supported case, but it
  cannot be exercised end to end yet: scalar assembly still refuses a driven
  material anywhere in the scene, so the interior case is pinned on the
  predicate directly until the far field compiles against a temporal
  generation.
- No shader changed. The contour reconstruction reads the static inverse mass,
  which stays exact while the exterior is time-invariant, so the policy is
  what keeps that shader correct rather than an accident.

## 2026-09-22 — Temporal vector overlay

- The arrow lattice now installs against a time-driven generation through the
  same shared reconstruction the point and line recorders use. The overlay
  shader already had the temporal branch and its skip guard; what was missing
  was a host installer that addresses the law records, so a modulated element
  drew arrows from fixed coefficients.
- The pre-filter complementary field the presentation high-pass reads is
  evaluated with the current law. That is correct at the only boundary the
  host uses it, a zero-duration filter commit, where the spare lane holds the
  pre-filter state at the same instant rather than the previous endpoint.
- The hidden gate now carries four consumers on one fixture. On Apple M1 Max /
  Metal the arrows matched the f64 point contract to `1.119e-6` in
  complementary magnitude and `1.354e-6` in flow across five lattice samples,
  with the point, line and area figures unchanged from the previous run.
- Formatting, strict workspace Clippy, the workspace suite and a native release
  build pass. Remaining in the diagnostic gate: the far-field exterior policy,
  temporal-work accounting and the event-boundary deferral fixtures.

## 2026-09-22 — Temporal area probes

- Area probes now compile against a time-driven generation. Each contribution
  addresses its own law records, so the assembled nodal map, each node's share
  of it and the samples' constitutive inverses are all evaluated at the
  sampled instant rather than from the authored coefficients.
- The area contribution stores each node's time-independent mass contribution
  instead of its finished share, and the shader applies the node's inverse mass
  itself. That is the same arithmetic on a static generation and stays correct
  when a driven material moves the mass, so there is one code path rather than
  two.
- Splitting the recorder's two passes onto separate bind-group layouts made
  room for the law tables. The shader declares nine buffers, one more than a
  portable stage may bind, but the element pass uses seven and the reduction
  five over the same numbering. The declared budgets are asserted against the
  shader text, including that their union exceeds the limit, so the split
  cannot be quietly undone.
- Replaced the quadratic law-record lookup while there. Resolving an element's
  node-major record addresses by scanning the contribution list is tolerable
  for sixteen point probes and quadratic for an area probe covering a mesh; the
  runs and ranks are now built once per recorder upload.
- A core fixture pins the driven area probe against the operator's own
  instantaneous energy at full coverage, and requires the driven answer to
  differ from the inert one so the fixture cannot pass without exercising the
  new evaluation. On Apple M1 Max / Metal the hidden gate now carries all three
  consumers: area total energy `2.339e-7` and complementary RMS `1.094e-7` at
  100% coverage, the five-sample line at `3.946e-7` primary, `1.108e-6`
  complementary, `7.227e-7` energy and `1.855e-6` normal flow, and the point at
  `8.836e-8`, `9.016e-6` rate, `6.812e-8`, `6.750e-8` flow and `1.273e-7`
  energy.
- Formatting, strict workspace Clippy, the workspace suite and a native release
  build pass. Remaining in the diagnostic gate: temporal vector overlays,
  far-field exterior policy, temporal-work accounting and the event-boundary
  deferral fixtures.

## 2026-09-21 — Temporal line probes

- Line probes now compile against a time-driven generation. Each sample
  reconstructs through the same shader block the point recorder uses, so a
  travelling drive is resolved at the element's own samples on a line exactly
  as at a point, and a sample without temporal addresses reports a gap instead
  of being read with fixed coefficients.
- The point and line recorders no longer carry separate installers for the
  fixed and temporal operators. One stencil source builds either record, which
  removed a duplicated point installer and made the line variant a few lines
  rather than a copy of sixty.
- Extended the hidden consumer gate with a five-sample slanted interior line,
  its cadence aligned with the point recorder's so both land their last sample
  on the compared state. On Apple M1 Max / Metal over 2,214 Q, 4,182 b and 20
  steps, the worst relative f64-oracle errors across the line were `3.946e-7`
  primary, `1.108e-6` complementary magnitude, `7.227e-7` energy density and
  `1.855e-6` normal flow. The point probe in the same run was `8.836e-8`,
  `9.016e-6` rate, `6.812e-8`, `6.750e-8` flow and `1.273e-7` energy.
- Formatting, strict workspace Clippy, the workspace suite and a native release
  build pass. Next: temporal area probes.

## 2026-09-21 — Consumers invert where the solver owns an inverse

- Fixed the evaluation order every diagnostic consumer uses, before the rest of
  the Stage 7 diagnostic gate is built on it. A constitutive inverse is now
  applied only at the assembled nodal map and at an element's own six
  complementary samples; the recovered physical fields are interpolated to
  wherever a consumer reports them, and densities and flow use forward
  coefficients there. Point, line, area and arrow consumers previously
  interpolated the flux and inverted once at the report point.
- The old order is identical for a uniform linear element and cheaper, so this
  is not a bug fix. It is a prerequisite: it invents a constitutive evaluation
  site the solver does not own, which under a travelling drive already differs
  across one element, and which a Stage 8 nonlinear law could only serve with an
  extra uncached bracketed solve per reported point. A new core test builds a
  travelling complementary drive whose six per-sample factors span more than
  0.05 and pins the consumer to the sample-first answer.
- Area probes now report the solver's own discrete energy over their covered
  elements: each node's lumped energy apportioned by that element's share of the
  node's mass, plus the element's sample energies, scaled by the piece's covered
  fraction. A probe covering every face equals the diagnostics panel's bulk
  energy to `1e-9` relative, which an integral of a pointwise density cannot do
  because the conserved primary energy is mass-lumped. Field means and RMS keep
  the smooth twelve-point rule; a partly covered element contributes in
  proportion to its area, which shows only at a disk's clipped rim. The area
  quadrature record consequently carries no material law at all, which removed
  one packed vector per quadrature point.
- The point and line shaders now share one reconstruction block held
  byte-identical by a test, and the line recorder gained the law-table binding
  it needs for temporal samples. The canonical area recorder is at the portable
  eight-binding limit, so it carries per-sample constitutive data in its
  contributions instead; both budgets are now asserted against the shaders. Base
  constitutive tensors are immutable for a generation and no event rewrites
  them, so copying them into a stencil cannot go stale the way a copied law
  value would.
- Re-ran the temporal point gate on Apple M1 Max / Metal over 2,214 Q and 4,182
  b. At the original 16 steps the relative f64-oracle errors are `1.256e-8`
  primary, `2.866e-6` rate, `1.142e-7` complementary, `1.427e-10` flow and
  `5.778e-8` energy, against `1.26e-8`, `2.87e-6`, `1.14e-7`, `1.31e-7` and
  `5.78e-8` before. Do not read the flow figure as an improvement: at 20 steps
  the same build gives `6.750e-8`, so that one value was a near-cancellation in
  that fixture rather than a systematic gain. Agreement is unchanged in
  magnitude, which is the expected result for a reordering that is exact on a
  uniform linear element.
- Formatting, strict workspace Clippy, the 727-pass workspace suite with the one
  historical ignored reproducer, and a native release build pass. Next in the
  diagnostic gate: temporal line probes, then temporal area probes.

## 2026-09-21 — Temporal point diagnostics use accepted endpoint maps

- Added the first modulation-aware diagnostic subgate: the CPU point contract
  reconstructs current `Q/M(t)`, previous `Q/M(t-dt)`, the current physical
  complementary field, energy density and Poynting flow from one synchronized
  accepted endpoint. Travelling modulation is evaluated at the actual probe
  point rather than at a borrowed quadrature sample.
- Production point stencils reference the authoritative coefficient records in
  the solver table. They do not cache copies of material-law values, so a
  same-layout live law patch cannot leave point diagnostics on stale metadata.
  Static stencils remain explicit, and a temporal point consumer without the
  temporal references is invalid rather than silently using fixed coefficients.
- The hidden production-render-graph fixture uses travelling primary modulation
  and time-crystal complementary modulation on 2,214 Q / 4,182 b unknowns. On
  Apple M1 Max / Metal, after 16 steps its relative f64-oracle errors were
  `1.26e-8` for the primary field, `2.87e-6` for its rate, `1.14e-7` for the
  complementary magnitude, `1.31e-7` for flow and `5.78e-8` for energy.
- This is a checkpoint inside the diagnostic gate, not its closure. Temporal
  vector-overlay installation, line/area integrals, far-field exterior policy,
  temporal-work accounting, event-boundary deferral/rebase fixtures and
  modulation-aware AMR remain open.

## 2026-09-21 — Time-driven generations gain the paired grid filter

- Adopted the exact frozen-time extension rather than a reference-operator
  shortcut. At a zero-duration filter boundary every primary inverse and
  complementary map is evaluated at the same accepted time; the scale uses
  the trajectory-wide temporal CFL bound. CPU fixtures pin energy reduction,
  constant instantaneous primary fields, component totals, compatible flux
  and stationary complementary state.
- The temporal GPU filter adds two sparse passes to the existing three spatial
  stages, evaluates accepted/candidate energy with the instantaneous maps, and
  commits through the existing global event boundary. Both explicitly queued
  filters and resident every-16-step maintenance use this path; static
  generations retain their old dispatch extents and arithmetic.
- The production Metal fixture crosses a clock rebase, Switch reversal and law
  patch. One live filter ends at `3.88e-7` relative Q and `4.17e-5` relative b
  error against f64. Six resident filters end at `7.25e-7` and `2.02e-4`; the
  latter has a separate `3e-4` repeated-f32 acceptance bound rather than
  weakening the single-event gate. The unchanged static five-event fixture
  remains at `3.17e-7` Q and `1.10e-6` b.
- Next Stage 7 gates are modulation-aware diagnostics and AMR, supported
  boundary compositions, then actual-core incremental cost. Nonlinear filter
  semantics remain Stage 8 work.

## 2026-09-21 — Temporal event rejection retains the accepted generation

- Added a production-render-graph failure gate for a temporal law patch. A
  deterministic final-validation failure leaves accepted physical storage
  byte-identical, does not advance the clock or material serial, and reports a
  rejected live-event receipt rather than latching the whole solver.
- After clearing only the injected status, the same generation advances one
  step under the old law before retrying. Its agreement with the f64 old-law
  oracle proves the rejected candidate coefficient/runtime bank did not leak
  into execution; the subsequent clean retry then commits at its own paused
  GPU boundary and evolution continues under the target law.
- On Apple M1 Max / Metal, the rejected-event/old-step/retry fixture ended at
  `2.65e-7` relative Q error, `2.43e-6` relative b error and `8.92e-9 s`
  absolute clock error. The remaining Stage 7 work moves to filter, dynamic
  AMR, diagnostic, supported-boundary composition and incremental-cost gates.

## 2026-09-21 — Temporal runtime survives generation handoff

- Extended the canonical transfer layout with stable material-ID ownership and
  a per-drive phase policy. Retained drives preserve their accepted carrier at
  the exact GPU commit boundary even when frequency changes; drive-kind or
  explicit authored-phase changes intentionally start from the target-authored
  carrier. New materials also start from their target-authored trajectory.
- The GPU translates an accepted material-wide Switch origin into the target
  epoch and publishes identical accepted/candidate runtime banks. Target
  frequencies remain authoritative. Static transfers retain their old behavior;
  mixed static/temporal handoffs are rejected because they need an explicit
  physical initialization contract.
- Added a hidden production-render-graph handoff gate. On Apple M1 Max / Metal,
  an active Switch plus a preserved travelling-drive frequency edit and an
  explicit time-crystal phase edit crossed a 1,934-Q / 3,630-b handoff. After
  24 source and 40 target steps, relative f64-oracle errors were `2.99e-7` for
  Q and `2.09e-6` for b; absolute clock error was `1.25e-8 s`.
- Host tests pin stable-ID mapping and the frequency/phase distinction; Naga
  accepts both transfer shaders.

## 2026-09-21 — GPU-stamped temporal material events

- Added zero-duration live transactions for material-wide Switch begin/reversal.
  The host uploads material, target, duration and serial but no time. The GPU
  evaluates the accepted ramp at the actual complete-step boundary, fills the
  candidate runtime bank and publishes it only through the existing global
  acceptance dispatch. Mid-ramp reversal is therefore value-continuous even
  after clock rebasing or host/readback delay.
- Same-layout temporal-law patches stage complete target coefficient records
  in the transient eighth binding. Frequency edits preserve the accepted
  instantaneous carrier at the GPU boundary; depth, smooth-square sharpness,
  travelling spatial phase, reciprocal flag and alternate factor can change
  together. Candidate f32 factors and the target trajectory timestep bound are
  checked before coefficient records and the runtime slot are committed.
- The fast path rejects material/drive-kind ownership changes, static operator
  changes and explicit authored phase jumps. Those require a prepared
  generation transition until their distinct mapping/admission contracts are
  implemented. Other event classes remain gated on temporal generations until
  their composition tests close.
- The production-render-graph harness now crosses a clock rebase, reverses an
  active Switch, applies a frequency/depth/alternate/spatial law patch, and
  evolves after both transactions. On Apple M1 Max / Metal, 96 steps matched
  the f64 piecewise-operator oracle to `3.63e-7` relative Q error and
  `2.41e-6` relative b error; the absolute clock error remained `1.93e-5 s`.
  Naga and Chrome WebGPU accept the shader. Next: temporal runtime generation
  transfer and explicit temporal-event rollback injection.

## 2026-09-21 — Stage 7 temporal bulk passes the real GPU path

- Added a focused hidden-app gate around the production canonical render graph,
  storage bindings, shader stages, global step commit and full-state readback.
  The fixture combines travelling reciprocal primary modulation, smoothed-square
  complementary modulation, an in-progress material Switch and a nonzero
  absolute clock.
- The gate installs two steps before the production clock-rebase threshold, so
  a successful comparison also exercises GPU phase reanchoring, Switch-origin
  translation and accepted/candidate runtime-slot publication. A clock-aligned
  temporal-state constructor now represents an accepted generation directly;
  the test does not manufacture its start time through thousands of warmup
  steps.
- On Apple M1 Max / Metal, 96 production steps (2,214 Q DOFs and 4,182 vector
  samples) crossed epoch 7 to 8 with relative f64-oracle errors of `3.72e-7`
  for Q and `2.62e-6` for b. Reconstructed absolute-time error was
  `1.93e-5 s`; no GPU failure was reported. This is a correctness run, not the
  Stage 7 incremental-cost measurement.
- The path remains dormant in the application. Next: transactional live
  Switch/law events and temporal runtime transfer, followed by filter, dynamic
  AMR, diagnostic and supported-boundary gates before authoring controls are
  enabled.

## 2026-09-20 — Stage 7 bulk laws occupy the production GPU layout

- Added the dormant conservative temporal-plan compiler to the actual
  eight-binding GPU representation. It uses the already reserved material
  runtime slot, unused node contribution-range lanes, the unused sample record
  lane and the existing packed table buffer; static plans retain their current
  state/node/sample layouts and stepping path.
- Material-owned runtime records carry four phase anchors, Switch trajectory
  state and angular frequencies in accepted/candidate slots. Per-contribution
  primary records preserve independently driven material/frame contributions;
  complementary records point directly from their quadrature samples. Packing
  rejects trajectories that remain valid in f64 but collapse or overflow in
  f32.
- The shader KDK now has the gated temporal operations it will use in
  production: endpoint complementary factors in each kick and assembled
  midpoint primary mass in the drift. The four-dispatch conservative bulk cost
  is unchanged. Clock rebasing advances all material phases, shifts Switch
  origins and republishes both runtime slots.
- CPU decoding of the packed f32 representation matches the f64 node and
  quadrature maps at every KDK stage; a complete packed f32 step matches the
  f64 bulk oracle, and rebase invariance is pinned. WGSL passes both the Naga
  layout test and Chrome's WebGPU shader-module validator. Production
  installation remains disabled, and temporal
  filters, events, loss/open-boundary compositions and generation transfer are
  rejected until their individual Stage 7 gates close.

## 2026-09-20 — Stage 7 adopts the free bulk Poisson split

- Formalized the existing lossless bulk KDK as exact kick/drift subflows: a
  Poisson map globally and a symplectic map on nondegenerate leaves. Static
  linear stepping keeps the same arithmetic and cost.
- Added the dormant smooth-time reference through the autonomous `(t,p_t)`
  extension. Material laws now provide analytic coefficient rates; the CPU
  oracle evaluates endpoint/midpoint/endpoint stages and records material-pump
  work plus the remaining splitting residual without making `p_t` production
  state.
- The reference admits only free lossless bulk. It explicitly rejects loss,
  prescribed/weak/open boundaries and auxiliary couplings. We will not add an
  expensive global boundary solve merely to claim whole-system symplecticity;
  those systems retain their passive/transactional compositions. Thin gaps can
  be included later only if their existing local spring update is a cheap exact
  split.
- Tests pin exact inert parity with production KDK, driven forward/backward
  reversibility, analytic energy-rate finite differences, full-trajectory CFL
  contraction and second-order convergence of temporal-work residuals. The
  production GPU path still rejects driven materials pending the remaining
  Stage 7 boundary/filter/AMR/admission gates.

## 2026-09-20 — Stage 7 starts from one temporal-runtime contract

- Added the dormant f64 runtime primitives shared by material compilation,
  GPU events and handoff: bounded carrier phase anchors, material-wide Switch
  trajectories, and evaluated loss drives. A frequency edit can reanchor the
  old instantaneous carrier at the actual commit time, while an explicit phase
  edit remains an intentional phase change.
- Mid-ramp Switch reversal now has a precise value-continuous construction from
  the accepted-time blend. Zero-duration switching is an immediate complete-step
  event. Constitutive alternates use the shared blend independently per row.
- Reciprocal coefficient laws evaluate the reciprocal of the complete live
  drive/Switch trajectory. They never interpolate between reciprocal endpoint
  values. Focused tests pin phase continuity, reversal, hard switching,
  reciprocal semantics, and time-driven constant loss.
- This commit does not make authored drives executable. Next, the canonical
  production path still rejects them. A dormant temporal wrapper now retains
  exact material-frame coefficient/loss samples and stable material-ID runtime
  ownership beside the fixed operator. It evaluates assembled primary maps,
  complementary inverses, forces and frozen loss rates at an explicit time;
  inert calls delegate exactly to the existing fixed path.
- Coverage includes travelling node/quadrature coordinates, independent
  contributions at shared material nodes, TE primary/complementary placement,
  reciprocal switching, physical loss-channel placement, and explicit Stage 8
  rejection. Next, use these evaluators in the full f64 KDK composition, add
  trajectory timestep bounds and temporal-work accounting, then close the
  boundary/filter/AMR gates before GPU table work.

## 2026-09-20 — Physics skins expose only their own UI vocabulary

- Mechanical no longer offers the canonical complementary vector as an
  `In-plane field` arrow overlay. The shared solver still computes it, while
  the mechanical presentation exposes only energy flow; a persisted selection
  resolves to Off when opened in that skin.
- Boundary pickers now expose free/fixed-or-prescribed mechanical conditions in
  Mechanical and electric/magnetic walls in EM. Generic reflecting is retained
  as a persistence/core variant but is presented as its exact PEC/PMC alias for
  the active polarization; old scenes and skin changes are not rewritten merely
  by opening the inspector.
- Linear material rows now use skin-specific physical names: density/stiffness
  in Mechanical and permittivity/permeability in EM. The forthcoming nonlinear
  wire-up must extend this to the planned Simplified/Advanced response, drive,
  alternate and named loss-channel controls rather than reverting to generic
  storage-slot labels.

## 2026-09-20 — Arrow sampling and AC state survive view and mesh changes

- Pan and zoom previously replaced the GPU arrow lattice every UI frame and
  invalidated the last completed revision immediately. Readback could not catch
  a continuously moving target, so arrows disappeared until the camera stopped;
  a mesh handoff exposed the same asynchronous gap. Lattice replacements are now
  coalesced one at a time while the last completed world-space arrows remain
  drawable and reproject with the view. The coalescer also verifies recorder
  generation/revision ownership: an AMR handoff can orphan a zoom-triggered
  readback, which is abandoned immediately rather than freezing the gate until
  reset. A deadline retries any otherwise lost completion.
- A same-physics remesh keeps the prior arrows until the new samples arrive, then
  spatially rebases the presentation-only AC state onto nearby new screen-lattice
  samples. Physics changes and fresh zero-state installs still clear it.
- The DC blocker had treated every readback on a grid-filter cadence boundary as
  if its whole change were zero-duration maintenance, discarding genuine wave
  evolution since the previous readback. The GPU arrow record now includes the
  pre-filter complementary field: the blocker advances to that endpoint and
  suppresses only the actual accepted-state correction. Regressions cover static
  rejection, source-band gain, long-run mean, maintenance separation and remesh
  continuity.
- TODO: the live display can still appear to retain a nonzero arrow mean during
  long fixed-mesh runs despite the synthetic DC-offset regression settling to
  zero. Instrument raw and AC-coupled per-sample time means under real readback
  cadence, including source ramps and resident-filter boundaries, before deciding
  whether the remainder is physical state, sampling/exposure perception, or a
  presentation bug. Do not add another subtractor on visual evidence alone.
- Arrow density cells are now anchored to a quantized world-space lattice rather
  than to viewport pixels. Panning retains the same interior element winners and
  merely translates them on screen; a one-cell apron avoids resampling until a
  view edge crosses a world cell.

## 2026-09-20 — Threaded browser handoff packing matches native placement

- The GRIN-lens boundary-move report exposed another browser-only scheduling
  fallback: at 34,415 DOFs the canonical GPU plan and transfer took 333 ms to
  pack, and threaded Wasm performed all of it synchronously in the UI callback.
  Native already dispatched the identical owned job away from the UI thread.
- The isolated-browser pool now has a third execution slot for this dependent
  one-shot task, in addition to the two permanent preparation and AMR receiver
  loops. The native and threaded-browser call site is shared; only executor
  creation differs. Static Wasm retains synchronous packing as its explicit
  no-threads fallback.
- Browser startup coverage now requires topology and AMR worker handshakes, a
  real AMR result, and completion of a background GPU-pack job. This guards
  both worker capacity and the off-UI-thread handoff branch. Typed-array to
  `ShaderBuffer` serialization remains on the publication path and is the next
  place to inspect if a smaller mesh-scaled hitch survives.
- TODO: perform a full native/threaded-browser divergence sweep before polish
  is declared complete. Inventory every canonical preparation, AMR, packing,
  publication, readback and handoff stage; compare executor placement,
  cancellation/backpressure and fallback semantics; and add browser coverage
  for each intentional target-specific seam. The static no-threads Wasm path
  should remain an explicit, separately tested fallback rather than silently
  defining the threaded browser architecture.

## 2026-09-20 — Browser AMR worker starvation fixed

- The threaded WebAssembly bootstrap allocated one Rayon worker but submitted
  two permanent receiver loops to it: topology preparation and canonical AMR.
  The first loop to run occupied the pool forever, so the other queue could
  remain indefinitely at its submitted initial phase. In the reported run this
  left AMR at `Preparing canonical AMR estimate`.
- The isolated-browser bundle now allocates one worker to each long-lived loop.
  Both loops publish a startup handshake which the main thread exposes through
  its existing browser diagnostics; the WebGPU startup regression waits for
  both handshakes and for a real AMR job to report progress or completion,
  rather than merely proving that both jobs were scheduled. This restores the
  native architecture in the threaded browser bundle; only executor creation
  remains target-specific (`std::thread` versus shared-memory Web Workers).

## 2026-09-20 — GPU handoff admission and first display state share one receipt

- The remaining small-mesh handoff hiccup was a fixed-latency serialization:
  the GPU first mapped the candidate status, the host accepted the generation,
  and only then created and mapped its first target primary-state readback. The
  UI kept the old topology until that second round trip, so its `upload` phase
  could reach roughly 140 ms even when transfer arithmetic was tiny.
- After the transfer map has been fully consumed, its GPU buffer prefix now
  becomes a self-describing receipt containing validation status, accepted
  clock/slot data and every target primary-state word. A second dispatch of the
  already-required primary-transfer pipeline publishes it after commit; this
  avoids a separate cold compute pipeline. The paced reader remains bounded to
  one map in flight.
- The source generation and its readbacks stay owned until a valid receipt
  arrives. Success promotes the target request and first display snapshot in
  the same host transaction; failure destroys the candidate without exposing
  any of its state. Normal target state/control/status streams start after that
  atomic publication.
- On the M1 Max release fixture, a 2,692→4,200-DOF nonidentity handoff committed
  in 30.0 ms (20.9 ms from GPU submission through admission plus display), and
  the 8,938-DOF standard case committed in 53.2 ms (35.4 ms at that boundary).
  An injected failure rejected in 34.1 ms with byte-exact source rollback. The
  fixture now asserts that an accepted outcome can never precede its target
  display receipt. All 159 app tests, strict Clippy, static WebAssembly and the
  pinned-nightly shared-memory WebAssembly check pass.

## 2026-09-20 — Canonical AMR preparation crosses the worker boundary

- A five-second release sample at the reported large-mesh slowdown found one
  remaining periodic UI-thread AMR slice: building the direct-state canonical
  supplement took about 47 ms before the resumable indicator was submitted to
  `funfern-amr`. The primary-rate reconstruction took about 2 ms in the same
  sample. This can explain a recurring visible hitch, but not the gradual
  30–40 FPS ceiling at 112k DOFs.
- Canonical supplement construction is now a deferred first phase of the AMR
  indicator job. Native and shared-memory browser builds perform it on
  `funfern-amr`; static WebAssembly retains the existing synchronous cooperative
  fallback. Canonical failures return through the same accepted-result path with
  their original diagnostic instead of being lost at submission.
- The native worker fixture now exercises the deferred canonical phase. All 159
  app tests pass, as do native, static-Wasm and pinned-nightly shared-memory-Wasm
  checks. Hands-on verification still needs to confirm that the periodic hitch
  is gone in the autosaved 112k-DOF scene.
- The same live sample kept graphics resources bounded (about 36 MiB across 297
  graphics mappings) and showed render workers waiting on the next Metal
  drawable. The remaining gradual FPS loss therefore currently looks like real
  solver/render/presentation backpressure, not the previously fixed unbounded
  readback queue. AMR accuracy at that density remains under investigation.

## 2026-09-20 — GPU readback and solver submission gain completion backpressure

- The live process at the reported FPS cliff/foreground lock-up was blocked in
  Metal command-buffer creation, not in either CPU worker. Bevy's continuous
  `Readback` path had submitted a new staging copy every rendered frame even
  when earlier maps had not completed. The stalled process reached a 9.8 GiB
  physical footprint (16.8 GiB peak), 8.9 GiB of graphics mappings across about
  39,800 regions, 4,414 Metal command buffers and 8,194 resource lists.
- Every long-lived solver, status, arrow, probe and far-field readback now has a
  shared main/render-world gate and permits one staging copy in flight per
  entity. Requested full-state snapshots use it once; handoff admission polls
  its pending marker through the same bounded gate. Completion rearms a
  continuous reader; removal or rejection cannot leave a stream producing
  copies without a consumer.
- Host requests and render-world encoding also stay within 64 steps of the last
  GPU-completed boundary. If an expensive solve or a backgrounded window slows
  completion, Funfern drops stale wall-time catch-up instead of accumulating an
  arbitrarily long GPU queue; the existing speed-shortfall display reports the
  consequence.
- In a release soak of the same autosave, after more than three minutes the
  process remained responsive at about 0.88 GiB with no process compression.
  Graphics mappings were 41.1 MiB/268 regions, with 30 command buffers and 90
  resource lists. Native tests and strict Clippy pass; threaded and static
  WebGPU bundles both build and pass the Chrome startup/advance/resize check.
  Exact user-visible FPS and the foreground-after-background gesture remain a
  hands-on acceptance observation.

## 2026-09-20 — Solver overload yields display time; AMR leaves the UI thread

- The next autosave cliff was not another egui topology copy. At roughly 70k
  DOFs the shorter stable step made a real-time frame expensive enough to miss
  60 Hz. Pacing then spent the complete late wall interval on the next solver
  batch. The resulting positive feedback reached the 64-step frame ceiling at
  about 15 FPS: solver compute, drawing and coherent AMR readback all shared the
  same GPU queue, while AMR's CPU job received only one 2 ms slice per rendered
  frame.
- Interactive pacing now spends at most one 60 Hz wall interval per frame.
  Overload is reported as a simulation-speed shortfall instead of being repaid
  with ever larger GPU batches that starve presentation. Fractional step time
  and the independent high-speed batch ceiling remain bounded.
- A second long-lived worker, independent of candidate assembly, now owns both
  solution-indicator and mesh-adaptation jobs. Native and threaded-browser
  builds advance them continuously in bounded quanta; static WebAssembly keeps
  the cooperative fallback. Serialized results retain the topology/generation
  acceptance checks, and cancellation makes late results harmless after edits.
- A release profile on the same autosave showed indicator and adaptation work on
  `funfern-amr`, concurrent candidate work on `funfern-cpu-prepare`, and no AMR
  traversal on the UI thread. Exact post-change FPS and simulated-speed behavior
  at the user's threshold remain a hands-on acceptance check.

## 2026-09-20 — Dense presentation leaves egui's transient mesh path

- The earlier field-render correction was incomplete. The production autosave
  also enables the categorical Regions overlay, complete mesh and mesh-boundary
  display. At about 50k DOFs, `draw_solution` still rebuilt one filled triangle
  and one separately antialiased closed-line shape for every FEM triangle on
  every frame. The outline alone became tens of thousands of egui shapes and
  reproduced the reported frame-rate cliff even though field topology was
  already persistent.
- The field callback now also owns persistent linear-triangle positions and a
  unique edge list. Categorical colors are a compact per-triangle update; region
  fill, the quadratic field, unique mesh-edge quads and boundary-edge quads draw
  in order inside the same clipped GPU callback. Ordinary pan/zoom and frames no
  longer rebuild or tessellate topology-sized egui geometry. Property and AMR
  target overlays remain opt-in transient meshes for now.
- On the M1 Max autosave with field, Regions, mesh and boundaries all enabled,
  the corrected release run stayed around 54–60 FPS while adapting through
  52,660, 61,828 and 67,956 DOFs, then stabilized at 60 FPS at 67,956 DOFs. The
  solver stayed around 1,090 steps/s once the mesh settled. This supersedes the
  earlier unmeasured assumption that persistent scalar-field indices alone had
  removed the dense-presentation cliff.

## 2026-09-20 — AMR transactions stop fighting their own accuracy gate

- The controller correctly suppressed error-driven refinement once the global
  estimate met its target, but a simultaneous coarsening request still launched
  the bidirectional mesh job with the local error size field. That nominal
  coarsening pass could therefore split a locally marked patch, collapse another
  one, and publish a fresh solver mesh even though global refinement was settled.
- Each transaction now has one direction. Mandatory wavelength/maximum-edge
  violations and above-target global error permit refinement with coarsening
  disabled; coarsening requires two consecutive estimates below 65% of the
  requested global error and disables all edge splits. The interval from 65%
  to 100% is a hold band.
- Spatial hysteresis was also inconsistent: the app refined above `1.05 × target`
  but collapsed below `0.65 × target`, while a threshold edge split produces
  halves near `0.525 × target`. Collapse now requires `0.45 × target`, and
  changed vertices have a two-generation cooldown instead of one.
- A scan that applies no topology changes now advances only the cooldown state;
  it does not manufacture a mesh revision or trigger a canonical GPU handoff.
  Fixtures cover the global decision deadband, a mixed size field under a
  coarsening-only quota, and suppression of no-op handoffs.

## 2026-09-20 — Canonical AMR stops chasing invented derivatives

- The autosaved driven scene reproduced both reports: AMR held near 60% and
  refined toward roughly 130k DOFs, where presentation crossed a frame-budget
  cliff. Fixed-mesh traces ruled out canonical drift, gap/outgoing history and
  the wavelength floor. The scalar recovery and strong cell terms dominated.
- Splitting recovery made the defect explicit. Around 50k DOFs, primary-gradient
  recovery was only `5e-4–1e-3`, while a scalar rate reconstructed from the f32
  direct state contributed `0.09–0.22`. Adjacent `Q/M` endpoints lose their tiny
  per-step difference; evaluating the nodal balance instead still divides a
  cancellation-heavy f32 force by support area. The scalar semidiscrete-
  acceleration cell residual separately stayed near `0.05–0.10`.
- Static-linear canonical AMR now retains primary recovery, interface jumps and
  boundary checks, replaces rate recovery with region-local recovery of physical
  `Jb` in its `J^-1` norm, and uses direct endpoint defects for canonical drift,
  gaps and outgoing memory. It does not reinterpret the direct state as a scalar
  acceleration residual. The Performance panel reports primary/complementary,
  cell, jump and boundary contributions.
- On the same Metal autosave the corrected estimate crossed the 12% target at
  50,480 DOFs and then remained about 7–10%; the decreasing interior jump was
  the dominant term. This removes the route that had pushed the scene into the
  130k-DOF presentation cliff.
- The cliff itself was structural: egui flattened and recopied the quadratic
  field's static six-triangle-per-element index stream every frame. A paint
  callback now retains positions and indices in GPU buffers per mesh and uploads
  only one f32 value per node plus the view/exposure uniform. The existing
  material overlays keep their order beneath it. The display readback adapter
  also stopped deriving unused previous/rate/acceleration arrays at visual
  cadence; full canonical state remains sampled for AMR at its slower cadence.
  Native Metal startup and
  adaptive handoffs produced no render errors; the wasm32 app compiles and the
  WGSL parser fixture passes. Exact 130k-DOF FPS remains a hands-on acceptance
  measurement rather than a claim inferred from the smaller corrected mesh.

## 2026-09-20 — Arrow presentation state survives compatible GPU handoff

- The complementary-arrow DC rejector incorrectly treated every canonical GPU
  generation as a new physical sample space. An accepted material, boundary or
  timestep handoff therefore discarded every per-element baseline and made the
  arrows cold-start even when the mesh and observable were unchanged.
- Arrow-filter ownership is now the mesh discretization plus physics skin, not
  the transient GPU generation. Same-mesh handoffs retain per-element history;
  remeshing, a skin change, switching the filtered view, or installing a fresh
  zero state still clears it deliberately. Exposure already followed this rule
  and remains continuous.
- Compact arrow readbacks now carry the solver's compensated absolute time as
  two f32 lanes as well as the accepted-step marker. The presentation pole uses
  that time rather than multiplying a step delta by whichever timestep happens
  to be active after handoff, so retiming does not distort its decay.
- The full workspace suite, Clippy with warnings denied, the wasm32 application
  check and the production Metal timing harness pass. The harness also validates
  the updated compact vector-overlay shader against the CPU oracle.

## 2026-09-20 — Moving sources keep an explicit structural support

- A point source's packed layout was inferred from `weight != 0`. Narrow or
  distant Gaussian tails cross floating-point underflow as the source moves,
  so an ordinary drag was misclassified as a sparse-layout change and entered
  the full field handoff at `Locating complementary samples`.
- Canonical sources now distinguish structural support from numeric weight.
  The point-source slot reserves the nodes on its selected physical trace side,
  including nodes whose current weight is zero. Motion, width and enable/disable
  edits within that support use the atomic weight+drive event; genuinely changed
  region/volume support still requires a generation handoff. GPU plan packing
  and live-event packing consume the same explicit support.
- As a defensive fallback, a full handoff that shares the exact immutable
  canonical operator now creates the complementary identity map in one work
  unit rather than rechecking every quadrature carrier.
- Handoff rejection diagnostics now retain the GPU failure code, describe its
  validation class, and distinguish a rejected candidate (accepted generation
  retained) from a fault in the accepted generation.
- Full workspace tests, Clippy with warnings denied and the wasm32 application
  check pass.

## 2026-09-20 — Source and measurement transactions stop rebuilding the solver

- P2 now classifies prepared candidates by the smallest publication boundary.
  Measurement-only changes adopt probe/far-field metadata on the current GPU
  generation. Signal-only point/volume source changes use the staged canonical
  source event, and the CPU candidate is committed only after its GPU serial is
  accepted. Point position/width and volume-profile changes with unchanged sparse
  support use an atomic staged weight+drive event; point-region changes, volume
  enable/disable or sparsity changes, prescribed data and timestep changes still
  take the full handoff path.
- The drive-only source path requires identical source weights; both source
  paths require identical prescribed data and drive count. The shader events
  preserve instantaneous carrier phase and the integrated-rate anchor.
  Volume-source signals now reuse both operators
  and compiled spatial weights; unchanged probes, far-field contours, point-
  source placement validation and canonical forcing are retained.
- Handoff diagnostics split GPU-plan packing from requested-step drain. The old
  `drain` number included both and could not identify the remaining delay.
  Material/boundary revisions retain the exact mesh object, use the direct
  scalar identity constructor, and represent an exact complementary transfer as
  one flag instead of retaining and repacking a row for every quadrature sample.
  Vector-overlay sampling is keyed to the actual mesh revision, so metadata and
  source commits do not rebuild the unchanged arrow layout.
  Runtime regressions cover probe-only metadata publication, point/volume drive
  edits, a structural-support point-source move and full fallback for a
  support-layout change. The full app
  test and shader suite pass; hands-on autosave latency and frame pacing are the
  next acceptance observation before compact primary/full-plan reuse.
- The production Metal live-event harness now edits both temporal source
  parameters and spatial weights in one accepted event. Five paused events
  committed in 217.32 ms total (33.33 ms maximum); subsequent evolution matched
  the CPU oracle at relative L2 `Q=3.17e-7`, `b=1.10e-6`, with the clock unchanged.
- The follow-on identity compaction now covers the primary map as well as the
  complementary map. Same-discretization handoffs retain no duplicate scalar
  interpolation rows, and GPU transfer packing consumes the identity marker
  directly instead of allocating exported rows to rediscover it.
- Canonical reassembly now retains an unchanged second-order outgoing modal
  system by `Arc`. Reuse requires an exact signature match over trace nodes,
  impedance weights and sparse tangential-operator entries; otherwise the
  cooperative eigensolve runs normally. The topology regression demonstrates
  that a loss-only material edit keeps the same modal object, while a core
  regression rejects reuse when the outgoing trace is removed.

## 2026-09-20 — Browser preparation worker and overhaul branch

- Canonical-overhaul work now lives on `canonical-overhaul`; local `main` was
  returned exactly to remote `main` at `beca47e`, and no remote ref was changed.
  The preceding browser packaging/toolchain corrections are commit `56a58fd` on
  the overhaul branch.
- P1W moves the same owned, token-checked topology preparation job into exactly
  one shared-memory Web Worker. The worker keeps the native eight-millisecond
  quanta and newest-request replacement policy, while the accepted GPU solver and
  main browser thread remain live. Worker bootstrap failure after module startup
  still falls back to the bounded cooperative runner.
- The threaded bundle is explicit: `scripts/trunk` selects
  nightly-2026-05-28, rebuilds `std` with Wasm atomics, enables the worker feature,
  and local Trunk/nginx serving supplies COOP/COEP. `bevy_egui` is pinned to its
  Bevy-0.19-compatible 0.41.1 line; 0.42's atomics configuration deliberately
  makes browser dropped-file handles non-`Send`, which cannot be an ECS component.
- Static hosting remains supported rather than silently broken. Plain stable
  `trunk build` emits the cooperative bundle, and Pages publishes that variant
  because it cannot set cross-origin-isolation headers. Chrome/WebGPU checks pass
  for both paths; the threaded check asserts `crossOriginIsolated` and actual
  worker activation before checking frame advance and resize. Hands-on browser
  preparation timing and frame pacing remain release observations.

## 2026-09-20 — Canonical preparation latency correction begins

- Headless reproduction of the current autosave found both a bug and real
  nonlocal cost. A 10,756-DOF material edit with a 288-node second-order trace
  used about 0.94 s of CPU across 235 four-millisecond slices, explaining the
  roughly five-second request-to-visible delay at display cadence plus upload.
  Four repeated edits were stable within 2%; AMR growth, not accumulated work,
  explains the increasing delay.
- Point-source placement was being revalidated after assembly on virtually
  every cooperative work unit. Its linear boundary/triangle search accounted
  for about 0.46 s at the saved mesh and much more after refinement. Validation
  is now one-shot per candidate, with a call-count regression. Preparation
  diagnostics also report total CPU work and the unclassified remainder rather
  than showing phase buckets that omitted this work.
- Added a staged preparation-latency subplan to the canonical specification.
  P0 and P1 are now complete: native builds move the owned job to one long-lived
  worker, report its progress through the existing diagnostics and policy gates,
  and admit its result through the same token-checked transaction. Between
  eight-millisecond quanta the worker keeps only the newest queued request, so
  rapid edits neither publish stale work nor create competing assembly threads.
  WASM retains the four-millisecond cooperative path; worker spawn/channel failure
  falls back to it on native builds as well.
- The release autosave fixture (10,756 primary DOFs, 3,516 triangles, 288 outgoing
  trace nodes) now reaches a material candidate in 488.6 ms wall time for 488.3 ms
  of worker CPU. This is 61 worker quanta rather than 121 display-frame slices.
  A 4,096-step production render-graph run measured 10.28 simulated seconds per
  wall second both alone and under repeated saved-scene assembly load on Apple M1
  Max, so this fixture showed no measurable solver-throughput loss.
- External completion, stale-token rejection and native worker return have focused
  regressions; topology runtime and UI suites, Clippy and the WASM check pass.
  Hands-on frame pacing remains a release observation. Next is dependency-specific
  reuse, followed by exact/proportional second-order trace fast paths.

## 2026-09-20 — Chrome WGSL validation repair

- The canonical solver and handoff shaders passed wgpu's pinned Naga validator
  but failed Chrome/Dawn. Reduction kernels could return on storage-buffer state
  before reaching a workgroup barrier, which WGSL uniformity analysis forbids.
  Every invocation now reaches each reduction barrier; stopped, identity and
  out-of-range workgroups contribute zero and leave only after the reduction.
- Bevy's Naga writer also expanded the exact `f32::MAX` sentinel into a rounded
  decimal just above the representable range. Canonical shaders now use `3e38`
  as the finite-state ceiling, retaining ample overflow detection headroom. The
  handoff accounting failure sentinel uses the existing runtime-dependent NaN
  construction required by WebGPU constant-expression rules.
- A browser test now submits every raw WGSL module to Chrome, in addition to the
  native Naga shader test and full application smoke test. The full rebuilt WASM
  app starts, advances and resizes in Chrome without rendering validation errors.

## 2026-09-20 — Browser packaging and local serving repair

- The toolbar logo became a compile-time asset after the original container
  recipe was written, but the builder still copied only the workspace manifests
  and `crates/`. The image build therefore failed when Rust evaluated the logo's
  `include_bytes!`. The builder now copies `assets/` before compiling. Its release
  target, Cargo registry and Trunk helper downloads use architecture-specific
  BuildKit caches, so transient registry/helper-download failures no longer
  discard the expensive WASM compilation or mix host executables between ARM64
  and x86-64 builds. The build step also gives Cargo explicit retry and timeout
  budgets for slow container-network transfers.
- The generated Trunk page, JavaScript and WASM all serve correctly, and a
  self-closing Chrome/WebGPU smoke check reaches the running canvas without page
  or console errors. The bare-`trunk serve` failure was Trunk 0.21.14 rejecting the
  conventional inherited `NO_COLOR=1` before it bound a socket. The active
  toolchain is now Rust 1.96.1 and Trunk 0.22.0-beta.5, whose upstream correction
  accepts the literal documented commands without an environment workaround.
  Rust 1.96.1 also emits WASM metadata that Binaryen 123 rejects, so Trunk's
  `wasm-opt` helper is explicitly pinned to current Binaryen 132. Docker and CI
  use the same pins. Reverse-DNS address discovery is disabled so the server
  reports only its configured `127.0.0.1` endpoint instead of misleading Docker
  Desktop aliases.
- Validation includes Compose configuration parsing, a locked release Trunk build
  with nonempty HTML/WASM output, and the Chrome smoke check. With the new pins,
  the literal inherited-`NO_COLOR=1` `trunk serve --release` command binds
  `127.0.0.1:8080`; a GET returns HTTP 200 with a nonempty page and Chrome reaches
  the WebGPU canvas without console or page errors. The test server and browser
  were closed afterward. A repaired container build had already compiled the
  application past the former missing logo; final nginx image assembly still
  needs a rerun because the available Docker network subsequently timed out at
  GitHub, crates.io and even Docker Hub base-image metadata.

## 2026-09-20 — Measured terminal tail and self-describing snapshots

- Interactive retesting showed no observable change from the first dormancy
  correction. Reproducing the current autosave with its source disabled, a unit
  Gaussian pulse and the production filter every 16 steps explained why. The
  energy settles slowly from its peak to `1.63e-5` after 40 simulated seconds;
  primary p98 and complementary p90 settle around 0.2–0.3% of their propagated
  peaks. The attempted `1e-6` energy / 0.1% amplitude gates were both below the
  actual damped tail and never fired. The scalar field had not been connected to
  the new energy gate either.
- Automatic scalar and vector exposure now share a measured 1% amplitude quiet
  floor and smooth fade. AMR uses its squared `1e-4` run-relative energy floor.
  This is intentionally a presentation/controller threshold, not additional
  mutation of canonical state; the paired filter still preserves its physical
  kernel. The saved-scene tail is below both corrected thresholds. The periodic
  canonical energy snapshot maintains the AMR run peak even while AMR is off,
  so enabling it after a pulse cannot redefine numerical residue as its peak.
- Full-state readback previously copied the state buffer alone, then decoded its
  accepted lane and timestamp from whichever asynchronous continuous control
  readback the host had most recently received. That is not an aligned snapshot
  contract and could intermittently swap the current/previous endpoints used by
  AMR. Canonical layout version 3 appends a state metadata word containing a
  finite magic, accepted lane and split exact absolute accepted step. Every
  successful step, event, resident filter, clock rebase and handoff publishes it
  atomically with state; energy and AMR now use that exact metadata.
- Validation covers the saved-scene drain trace, layout/metadata parity and tail
  exposure. Both the ordinary and resident-filter/vector-overlay Metal harnesses
  retain CPU-oracle agreement. All 653 workspace targets/tests pass with the one
  known curved-boundary reproducer ignored; warning-denied Clippy, shader
  validation, wasm32 checking, formatting, diff checks and the release app build
  also pass.
- TODO: this remains an empirical stopgap rather than a finished terminal-field
  policy. Refine scalar/vector exposure and AMR dormancy around a principled,
  quantity-aware numerical-noise estimate that behaves consistently across
  scenes, meshes, material scales and genuinely persistent low-amplitude fields;
  reassess whether more of the tail should be removed by the resident filter
  without damaging its intentionally preserved stationary kernel.

## 2026-09-20 — Filter-boundary consumers and dormant-field AMR

- The intermittent terminal static was not evidence that the resident paired
  filter had stopped running. The filter damps compatible top-of-grid dynamic
  modes, but deliberately leaves constants and stationary complementary flux in
  `ker(CᵀWJ)` untouched. It is therefore neither a DC remover nor a terminal
  zeroing operation.
- The resident filter is a zero-duration accepted event every 16 solver steps and
  flips the state lanes after the ordinary step. At that exact boundary the other
  lane is the pre-filter state, not the physical endpoint one `dt` earlier. AMR
  could intermittently interpret the deliberate filter correction as a temporal
  residual, and the complementary-arrow AC view could interpret it as new wave
  content. AMR now waits for the next ordinary endpoint; arrow AC state decays for
  that sample and rebases its input without admitting the maintenance jump.
- Relative AMR error is undefined in the useful sense after the whole field has
  decayed into f32 residue: both its energy and residual normalization approach
  zero. The first implementation attempted a `10⁻⁶` peak-energy floor; the
  measured correction above supersedes that uncalibrated value. Forced-wavelength
  and maximum-element limits remain active because they are resolution contracts,
  not error estimates. Fresh zero-field commits reset the AMR peak; ordinary
  field-preserving handoffs do not.
- Regression coverage distinguishes a filter maintenance boundary from an
  ordinary endpoint, keeps its jump out of arrow AC output, checks run-relative
  terminal silence, and proves that dormant AMR still obeys hard wavelength
  limits. All 653 workspace targets/tests pass with the one known curved-boundary
  reproducer ignored; warning-denied Clippy, the wasm32 application check,
  formatting and diff checks also pass.

## 2026-09-20 — View-stable arrow AC state and terminal silence

- The first AC-arrow implementation keyed its history by screen bin. Moving the
  view reused a bin for a different world point, treating the spatial jump as AC,
  while a newly occupied bin passed its whole unknown DC value on the first sample.
  The filter now keys a lazy cache by physical mesh element, measures elapsed time
  per element when a sample returns to view, and clears identities at a mesh
  generation change. An element without history starts at zero output. This avoids
  allocating filter state for the full canonical field and prevents a pan or zoom
  from manufacturing a transient.
- Arrow exposure had a separate arithmetic bug: length was clamped *after* the
  quiet-tail multiplier, so a large isolated residual could cancel an arbitrarily
  small global fade and still hit full length. Saturation now precedes the shared
  fade, and the overlay stops drawing below a visibility whose longest supported
  arrow is about 0.1 pixel. Near-zero f32 directions can no longer appear as random
  full-size arrows after the field has drained.
- Regressions cover cold starts, physical-sample continuity across a moved screen
  position, off-screen simulated-time decay, ordinary-frequency retention, and a
  sparse outlier under terminal exposure. All 313 app targets/tests (163 library,
  147 binary, two catalog and one shader validation), warning-denied Clippy, the
  wasm32 application check, formatting and diff checks pass.

## 2026-09-20 — AC arrow view and quiet-tail exposure

- Removed the old `0.82 old + 0.18 new` arrow smoothing. It was a component-wise
  low-pass inherited from the derived scalar-field reconstruction, and its physical
  time constant changed when the compact sampler raised arrow updates from 15 Hz to
  display cadence.
- Complementary-field arrows now optionally subtract a presentation-only baseline
  with the former reconstruction's 0.5 rad/s (about 0.08 Hz) corner, evaluated from
  accepted simulated time. Static display bias decays without touching canonical
  `b`, probes, energy, transfer, or energy-flow arrows. A 3 Hz regression retains
  more than 99.9% of the vector amplitude.
- Auto exposure now multiplies scalar colour and arrow length by a squared smoothstep
  below the run-relative quiet floor. The reference still releases continuously and
  preserves genuine surviving DC above that floor, while late f32 residue fades to
  black instead of being normalized into a static pattern.
- Version-22 presentation JSON keeps the retired smoothing key as `false` for older
  readers and stores AC coupling separately. Reading an older file migrates its
  former checkbox value to the new presentation option.
- All 311 app tests (308 unit, two catalog and shader validation) pass;
  warning-denied Clippy and the wasm32 application check also pass.

## 2026-09-20 — Scrollable inspectors

- Long Edit, View, Simulation, Materials and Probes inspectors now scroll within
  the available height instead of placing their lower controls off-screen. Each
  inspector remembers its own scroll position. The narrow-screen floating
  inspector is also capped below the toolbar so its scroll viewport remains
  reachable on short windows.

## 2026-09-20 — Continuous handoff display and compact vector overlay

- The remaining handoff hitch had two distinct clocks. At 93,144 `Q` / 185,310
  `b`, `begin_handoff` serialized its already-packed buffers in about 10–12 ms,
  while target upload, transfer and the asynchronous admission readback took
  about 75–80 ms. The accepted source had unnecessarily stopped for that whole
  latter interval.
- The source pass now precedes the one-shot transfer and remains live while the
  target is uploaded and admitted. The transfer publishes its exact absolute
  and epoch-local snapshot steps in the accepted status record. Requests made
  during admission remain a small absolute-step backlog; after admission the
  target consumes them from the transferred clock before its first new state
  reaches the display. Rejection still retains the advancing source. A large
  Metal fixture advanced eight steps before the snapshot and eight more during
  admission, then matched the CPU target at `Q=2.95e-7`, `b=1.63e-6` relative
  error. The passive second-order/history case also passes this live-source
  path; injected failure retains byte-exact rollback when no later source work
  was requested.
- Vector arrows no longer force a full physical-state readback at 15 Hz and a
  CPU traversal/reconstruction of every triangle. The UI selects at most one
  representative stencil per visible screen bin when the view changes; one
  compact GPU pass samples complementary field and energy flow after each
  rendered solver batch, and only those arrow records are read back. Full state
  remains on the 4 Hz AMR/energy cadence.
- With 1,024 compact arrow samples, resident grid filtering and production
  primary readback, the 93,144 / 185,310 fixture completed 512 measured steps in
  386 ms (2.56 simulated seconds/wall second); vector error was `1.19e-6` and
  the solver/energy gates retained their prior tolerances.
- Formatting, all 643 workspace tests (plus the known ignored curved-boundary
  reproducer), strict Clippy, shader validation, wasm32 checking and the release
  app/examples build pass.

## 2026-09-20 — Resident canonical grid damping

- Follow-up interactive testing isolated the roughly 220-step/s cap to “Damp
  unresolvable detail”. The canonical cutover had represented the fixed
  every-16-step filter as a host-authored live event. Each filter therefore
  uploaded an event buffer, changed the request revision, rebuilt render-world
  bindings, stopped the next batch at the event boundary and waited for host
  acknowledgement/readback. That fixed-frequency round trip explains both the
  weak DOF dependence and the periodic microstutter; ordinary damping and the
  outgoing condition were not the cause.
- Restored the old execution property in the canonical solver: the paired `Q,b`
  filter is now encoded directly in the resident command stream at the exact
  accepted-step cadence. It reuses candidate-state validation and atomically
  accepts or latches failure, but creates no live event, request revision,
  upload, bind-group rebuild or host wait. Probes observe the post-filter state.
- Corrected the lane-paired step-accounting boundary exposed by outgoing modes.
  Pending loss/work is consolidated before a resident filter, and every
  zero-duration event starts a fresh contribution bank when it flips the
  accepted lane. This retains filter removal and boundary loss in one ledger.
- The terminating Metal fixture matches the CPU oracle with periodic filtering:
  at 93,144 `Q` / 185,310 `b`, 512 measured steps take 521 ms (about 982
  steps/s, 1.90 simulated seconds/wall second); relative errors are 5.14e-6 and
  1.32e-6. The passive second-order fixture also passes, including its auxiliary
  state and energy gate. These are solver-path measurements, not a claim about
  final UI frame pacing.
- Formatting, all 641 workspace tests (plus the known ignored curved-boundary
  reproducer), strict Clippy, wasm32 checking and the release application build
  pass. The terminating five-event GPU harness also retains CPU-oracle parity.

## 2026-09-19 — Stage 6 interactive-performance correction

- The first follow-up isolated the steady canonical evolution kernels from the
  apparent 220-step/s regression at roughly 100k primary DOFs. The
  production harness sustains 1,025 steps/s with continuous full readback and
  1,164 steps/s with the production primary-only stream on a 93,144 `Q` /
  185,310 `b` reflecting fixture (about 2.04× and 2.25× real time). Subsequent
  UI isolation identified the periodic host-scheduled grid-filter event as the
  remaining fixed-rate bottleneck; the 2026-09-20 entry records its correction.
- Fixed step-rate accounting after Stage 5 made accepted-step totals continuous
  across generations. A handoff now establishes the new observation baseline
  instead of crediting the entire run again, so the speed-shortfall notice can
  no longer remain falsely satisfied.
- Replaced complementary handoff's target-sample/all-source-triangle scan with a
  cooperative uniform locator and recognize identical discretizations across
  transaction revisions. On the standard 8,938-DOF handoff, vector transfer fell
  from 102.0 ms to 1.3 ms; the full transfer-map preparation fell from 109.9 ms
  to 5.7 ms. Different-mesh and thin-gap handoff fixtures retain their accuracy.
- Continuous display readback now maps only the primary state prefix. Full
  complementary/auxiliary snapshots are requested at 4 Hz for AMR and energy,
  or 15 Hz while a vector overlay is visible. Sparse acceleration assembly moved
  from every display frame to the AMR snapshot cadence, and display vectors reuse
  their allocations.
- GPU plan construction reuses the validated scalar CSR instead of rebuilding
  the compatible stiffness with millions of tree insertions. Canonical assembly
  likewise uses compact sort/deduplicate adjacency. At 93,144 DOFs canonical
  assembly measured 73 ms and GPU-plan compilation 54 ms. The latter and the
  transfer-plan packing now run on a native background worker while the accepted
  solver continues. Upload serialization keeps the encoder's owned bytes rather
  than copying an 80.7 MiB generation twice; the measured main-thread handoff
  call is 11.5 ms at that size.
- Extended the terminating timing/handoff harnesses with large-mesh controls,
  phase timings, byte breakdown and primary-readback validation. No persistent
  application process is used by these checks.

## 2026-09-19 — Stage 6 acceptance: AMR liveness and preparation latency

- Native acceptance found that AMR completed its first handoff and then discarded
  every estimate as stale: the then-host-scheduled periodic grid filter changed
  the GPU buffer revision, which is not snapshot ownership. AMR now keys an
  estimate to the topology token and canonical generation, retaining the sampled
  accepted step only for cadence. A Metal run completed repeated 9,653 → 12,669
  → 15,683 DOF adaptive handoffs. Mandatory wavelength/max-edge limits also win
  when the error target binds the same element, rather than being suppressed by
  the global accuracy gate.
- Preparation's wall-time budget was checked after 256-element batches, and the
  dense outgoing-boundary eigensolve still hid in one final unit. The trace saw
  222–631 ms worst slices as AMR grew the mesh. Deadlines are now checked after
  every canonical element/validation unit and the Jacobi solve yields in small
  rotation blocks. Phase completion yields before the next job starts.
- Removed the remaining avoidable tails: source compilation moves its finished
  node table instead of deep-cloning it, complementary extension uses an edge
  adjacency index rather than all triangle pairs, and outgoing-history transfer
  validates its dense result in blocks. The app now grants preparation 4 ms of
  each frame; total second-order preparation remains visible but no longer owns
  one monolithic solver-blocking slice. Regression coverage exercises live-event
  AMR ownership, overlapping hard/error limits and cooperative eigensolving.
- Details and the acceptance rationale are folded into the
  [Stage 6 report](spikes/funfern-material-laws-stage6-report.md).
- Formatting, the 639-pass workspace suite (plus the one historical ignored
  reproducer), strict workspace Clippy, wasm32 checking and a release build pass.

## 2026-09-19 — Canonical production cutover (material-law Stage 6)

- Connected the accepted canonical `Q,b` generation to application evolution for
  Mechanical, TM and TE. Rendering and vector overlays now read direct physical
  state; the inverse-derivative reconstruction and display mean subtraction are
  gone. The old scalar GPU code remains only as optional comparison/recorder
  infrastructure, not as a skin-specific production formulation.
- Added shared f64/GPU point and area consumer contracts plus canonical point,
  line, area and far-field shaders. Probes expose accepted rate, complementary
  field, canonical energy and directed flow; histories survive mesh handoff only
  when their physical meaning is unchanged. Total diagnostics include thin-gap
  and outgoing physical storage.
- Ported static-linear AMR through a synchronized canonical supplement: direct
  energy normalization, primary loss, complementary endpoint defect, gap and
  outgoing residual/storage distribution, while retaining interface jumps.
  Dynamic/nonlinear AMR remains gated for its own derivation.
- Made every topology candidate prepare canonical operator/forcing/transfer,
  retained cooperative assembly, and switched run/reset/step/pulse/filter/live
  edits to global accepted-state ownership. Fixed edited-source handoff so target
  parameters replace the old authored values while old instantaneous runtime
  anchors remain continuous; disabled volume slots no longer shift ownership.
- Apple M1 Max / Metal steady 5,000-step rates are 16.13 reflecting, 13.74 first
  order and 8.71 second order simulated seconds/wall second, above the declared
  12.48/12.48/5.0 targets. Standard 1,000-step `Q,b`, auxiliary and energy
  accuracy fixtures pass. The standard core remains 7.64 MiB; second order is
  8.62 MiB including 0.93 MiB boundary data. See the
  [full Stage 6 report](spikes/funfern-material-laws-stage6-report.md).
- Formatting, strict workspace Clippy, all-target native checks, wasm32 checking,
  release build and a native app smoke test pass; the workspace suite has 639
  passes and one historical ignored legacy curved-second-order reproducer. Stage 7 is
  time-driven media/runtime switching; authored non-inert laws remain unavailable.

## 2026-09-19 — Latest-state GPU handoff (material-law Stage 5)

- Added the dormant canonical generation transaction: packed local `Q,b` maps,
  bounded component correction, prescribed exchange, thin-gap and basis-invariant
  outgoing histories, clock/source/runtime records, cache rebuild, energy
  accounting, validation and one global generation commit. No live CPU field
  readback/calculation/upload was introduced; the old accepted GPU generation
  stays installed until target acceptance and rejected targets are discarded.
- Made physical-coordinate complementary preparation and changed outgoing modal-
  basis composition resumable. Exact same-index primary/vector maps and unchanged
  outgoing operators use verified compact identities. On irregular first/second-
  order fixtures the jobs take 14.45/17.43 ms total but their largest measured
  slices are 1.19/0.19 ms; final GPU packing is at most 0.80 ms. The standard
  same-mesh total preparation is 9.42 ms.
- Added genuinely live zero-duration source, fixed-law, pulse, paired-filter and
  maintenance events. Five paused events committed in 217.00 ms total (32.91 ms
  maximum), preserved the clock and runtime serial ownership, and resumed within
  `3.03e-7/1.10e-6` Q/b parity. Source frequency edits preserve instantaneous
  phase through an independent accepted runtime slot.
- Apple M1 Max / Metal: the standard 8,938-Q/17,472-b same-mesh first-order
  handoff is 32.82 ms request-to-visible with `2.13e-7/1.20e-6` parity. Full old/
  new resource overlap is 15.28 MiB (7.64 MiB each) with an eight-word map; the
  two packed main-state buffers are about 0.806 MiB. Changed second-order history,
  thin-gap, prescribed and mapped-source fixtures pass; injected failure rejects
  in 35.62 ms with byte-exact source rollback. See the
  [full Stage 5 report](spikes/funfern-material-laws-stage5-report.md).
- Formatting, strict workspace Clippy, wasm32 application check and all 602
  workspace tests pass; the one pre-existing curved-boundary reproducer remains
  ignored. Stage 6 is consumer migration, static-linear AMR/diagnostics and the
  shared-skin production cutover; the connected scalar solver is unchanged.

## 2026-09-19 — Linear f32 GPU core (material-law Stage 4)

- Added a dormant production-intended canonical WebGPU path beside the connected
  scalar solver. One shared Rust/WGSL manifest uses eight storage bindings and
  integer metadata lanes; accepted/candidate `Q,b`, physical auxiliaries, force
  caches, clock, accounting and event serials commit through one global boundary.
  The old application evolution path is unchanged pending transfer and consumers.
- Ported reflecting, first-order, thin-gap, fixed two-sided loss, direct/legacy/
  prescribed forcing and the passive three-state second-order boundary. The
  latter exports the CPU reduced solve as a dense parallel inverse plus physical
  trace transforms; prescribed intersections use a cached constrained factor.
  Parallel trace preparation/reduction/solve/recovery avoids a serial shader LU.
- Added staged pulse, paired-filter, fixed-law-patch and maintenance writers,
  bounded epoch rebasing, priority failure latching, exact accepted-state rollback
  and recovery. A real Bevy render-graph harness checks the same buffers, shader,
  dispatch sequence, readback and commit path; injected failure leaves physical
  state, accounting and clock bit-exact before a successful resumed step.
- Apple M1 Max / Metal, standard h=.08 case, 1,000 steps: reflecting 18.46,
  first-order 18.50, two-sided loss 16.51 and second-order 10.18 simulated
  seconds/wall second. Worst standard `Q,b` parity was 1.88e-5, auxiliary RMS
  6.14e-8 and energy residual 9.02e-6. Prescribed second-order ran at 9.73 with
  1.46e-5/1.30e-5 parity; the shipped eight-obstacle geometry ran first order at
  10.35 with 1.65e-6/3.55e-6 parity. Standard steady allocation is 7.64 MiB and
  accepted physical state 0.17 MiB; second-order adds 0.93 MiB boundary factors.
- Standard second-order CPU/operator/factor/packing preparation measured
  302–311 ms, while a 128-step GPU request completed in 50.2 ms and sustained
  6.29 simulated seconds/wall second. The preparation cost is reported separately
  and remains a Stage 5 cooperative-preparation concern, not folded into steady
  throughput. See the [full Stage 4 report](spikes/funfern-material-laws-stage4-report.md).
- Formatting, strict workspace Clippy, wasm32 application check and all 598
  workspace tests pass; the one pre-existing curved-boundary reproducer remains
  ignored. Stage 5 is latest-state GPU transfer, physical-history handoff and
  genuinely live/paused event integration.

## 2026-09-19 — Linear CPU composition and transfer (material-law Stage 3)

- Extended the direct f64 reference with integrated point/volume/weak-boundary
  sources, analytic version-22 acceleration-to-rate conversion (including zero
  frequency and edit anchors), field-valued prescribed ownership and pulse
  exchange. Added the pure legacy-loss migration that moves the full symbolic
  rate to exactly one named physical channel; persisted version-22 data is not
  rewritten while the legacy solver remains the production path.
- Added independent fixed-linear electric/magnetic losses across Mechanical,
  TM and TE; physical thin-gap jump state; first-order outgoing impedance; and
  the adopted passive three-pole second-order trace with force-coupled midpoint
  kicks, reduced trace elimination, prescribed intersections and complete
  source/boundary/loss/edit energy accounting. Dense and matrix-free generators
  and cached-vs-oracle solves agree in tests. Immutable factors are shared rather
  than counted as accepted physical state.
- Added the paired Q,b grid filter with constrained-node exchange, component-
  scoped roundoff maintenance, support-aware conservative Q remap, six-sample
  physical-coordinate b reconstruction, and bounded two-ring extension. Separate
  thin-gap and outgoing maps transfer physical histories, account changed/new/
  deleted energy, and are invariant to modal signs, order and degenerate rotations.
  Irregular production meshes pass the declared 3% one-handoff and 5% twelve-cycle
  bounds; unchanged data is bit-exact and disconnected new islands stay zero.
- Re-ran the [corrected scattering fixture](spikes/funfern-boundary-scattering-spike.md):
  all 248 reflection, passivity, temporal and fixed-CFL checks pass; the rejected
  separate split remains its negative control. The core suite passes 268 tests
  with the one pre-existing ignored legacy curved-boundary reproducer. The full
  workspace passes 586 tests plus that ignored reproducer; strict workspace
  Clippy, native build, wasm32 check and isolated release Trunk package pass.
- Standard h=.08 CPU-oracle timings (8,938 Q DOFs, 17,472 b samples): reflecting
  compile/preparation 8–10/0.07 ms and about 5.2 simulated seconds per wall second.
  Second order has Nb=276 and 825 auxiliary scalars: typical compile/factor
  preparation 206/21 ms, 0.58 MiB modal rows, 0.62 MiB immutable rank-one factor,
  about 0.15 ms per reduced solve, and 1.89 simulated seconds per wall second.
  One repeated eigensolve was 266 ms, so preparation can still cross 250 ms.
- The CPU correctness oracle therefore does not satisfy the Stage 4 production
  second-order throughput target (5 simulated seconds per wall second), and its
  preparation typically clears but has exceeded the 250 ms review point. This is
  recorded rather than relaxed or hidden. Stage 4 must close f32/GPU throughput,
  portable layout, failure injection and global accepted/candidate atomicity;
  the old production solver remains connected until those and later consumers pass.

## 2026-09-19 — Linear direct-state CPU core (material-law Stage 2)

- Added a separate f64 reference core with synchronized integrated nodal `Q`
  and six independent two-component `b` samples per enriched-quadratic element.
  It retains per-element nodal material contributions, immutable direct and
  inverse quadrature tensors, generation tags, and an explicit empty physical-
  auxiliary slot. The old scalar solver remains unchanged and in production.
- Implemented deterministic direct incidence/gather, endpoint KDK, constitutive
  energy, compatible potential/velocity initialization per free component, and
  resumable compilation from owned mesh/material snapshots. Constant primary
  fields are stationary without a bulk potential; nonpotential complementary
  flux remains present instead of being projected away.
- Fixtures verify exact compatible-force parity with the existing stiffness
  matrix for Mechanical/TM/TE, both orientation signs, rotated anisotropy,
  scalar-recurrence parity, second-order endpoint/energy defects, connected-
  component compatibility and stationary extra modes.
- A release run of the real CPU path on the current arm64 machine compiled the
  standard h=.08 scene (8,938 DOFs, 2,912 elements, 17,472 vector samples) in
  5.56 ms. Its operator/state estimates were 4.54/0.33 MiB; 128 KDK steps took
  49.31 ms, or 6.41 simulated seconds per wall second. This is a CPU-reference
  baseline, not a GPU prediction or satisfaction of the later production gate.
- Core tests pass (250 plus one known ignored curved-boundary reproducer).
  Stage 3 is the source/boundary/loss/filter/transfer composition on this CPU
  oracle; none of those paths or the application have been cut over yet.

## 2026-09-19 — Material-law Stages 0–1 complete

- Froze the operational source, legacy-loss, trace/history, transfer, event,
  failure, serialization, f32 and target-device contracts in the
  [Stage 0 contract](spikes/funfern-material-laws-stage0-contracts.md). These are the
  acceptance inputs for the direct-state implementation, not new solver behavior.
- Coalesced the useful unfinished material-law scaffold into inert authoring
  infrastructure. Added physical coefficient roles, independent electric and
  magnetic loss channels, analytic tangent/passivity checks, reciprocal identity
  normalization, corrected cosine drives/frame use, full parameter traversal,
  and version-22-compatible persistence defaults. Reserved oscillator forms
  remain Gate O data.
- Dropped the superseded reciprocal-row swapping semantics. A non-inert authored
  law now fails explicitly at the legacy evaluator and skin converter rather
  than being silently ignored; the old production solver and legacy `damping`
  field otherwise remain unchanged until their staged migration.
- Verified formatting, Clippy with warnings denied, all workspace targets, an
  optimized native app build, and the `wasm32-unknown-unknown` app check. The
  workspace suite passed 558 tests; the one known curved second-order instability
  reproducer remains ignored. Stage 2's linear f64 direct `Q,b` core is next.

## 2026-09-19 — Material-law architecture adopted and plan consolidated

- Adopted direct integrated nodal Q plus independent quadrature b, with genuine
  thin-gap/outgoing/oscillator history. Adopted the passive three-state outgoing
  law with force-coupled midpoint kicks; retain both outgoing orders. No blanket
  nonlinear-domain ban, legacy unstable auxiliary port, or separate-boundary split.
- Reconciled the [specification](spikes/funfern-material-laws-plan.md) and
  [detailed stages](spikes/funfern-material-laws-review.md): removed production bulk
  potential/gauge/seam-offset obligations and the assumed one-CSR fast path;
  added direct-state energy/filter/remap and physical-history contracts.
- Linked all three spike reports and preserved their reproducible artifacts,
  measured limitations and expected failures. Boundary history, nonlinear full
  composition, irregular remap, f32/atomic acceptance, curved radiation and real
  core performance remain explicit gates before dependent enablement/cutover.
- Ready for Stage 0's bounded contract/fixture work and Stage 1's inert build
  repair. No production source changed; existing unfinished authoring changes
  remain outside this planning/evidence commit. No new numerical result or
  successful production build is claimed by this documentation consolidation.

## 2026-09-19 — Isolated single-boundary scattering follow-up

- Added a Bloch-periodic enriched-FEM strip that separates incident/reflected
  discrete modes at one flat face, removing packet/corner ambiguity. The passive
  candidate converges to its analytic reflection, including grazing improvement
  over first order; legacy second order remains better on this ideal planar test.
- Found a fixed-CFL reflection floor in the earlier boundary-only Strang split:
  21 refinement failures remain recorded. Coupling the held interior force into
  each boundary-aware midpoint kick removes the floor without making the bulk
  implicit. All 248 corrected checks pass, including full coarse-2D loss/stability
  regressions. See [derivation and results](spikes/funfern-boundary-scattering-spike.md).
- This supersedes the prior timestep recommendation, not its passive auxiliary
  law. Production source and unfinished NL work are unchanged. Performance,
  f32, nonlinear full-system composition and history handoff are to be validated
  in the real solver core; no further broad spike or GPU benchmark was added.

## 2026-09-19 — Isolated nonlinear-core boundary follow-up

- Added a reproducible boundary correctness experiment on clean `beca47e` FEM
  data; production and unfinished material-law source changes are untouched.
- Derived a passive three-state rational boundary response. It retains the
  quadratic tangential expansion and quartic small-angle reflection order,
  while removing the previous linear complementary-loss instability in the
  tested continuous and split-step systems. It is a nonlocal candidate, not
  a claimed CRBC implementation or exact curved DtN.
- All 64 follow-up checks pass: energy identities, spectral/long-time tests,
  second-order stepping, packets/corners, annular energy decay, nonlinear Kerr
  boundary substeps, and reduced trace-solve parity. Original spike failures
  remain recorded separately. See [derivation/results](spikes/funfern-boundary-auxiliary-spike.md).
- Remaining: review the reflection/nonlocal-cost tradeoff; implement and measure
  the actual solver core, including f32, boundary-history transfer and rollback.
  Curved radiation accuracy and the original ignored topology reproducer remain
  gated. No blanket nonlinear-material ban or first-order-only downgrade was
  adopted. No production implementation or performance claim was made.

## Current TODOs

Outgoing-boundary construction, not urgent:

- [ ] The second-order outgoing assembly is the dominant reassembly cost -
  `280.88 ms` against `4.99 ms` without that boundary on an 8938-DOF mesh, about
  ninety-eight per cent of the compile, and it is a dense symmetric
  eigendecomposition of the tangential operator over the trace. Worth attacking
  on its own terms rather than only as a prerequisite for driven media. One
  shape suggested: start from the first-order approximation, which needs no
  decomposition at all, and improve the boundary while the simulation runs -
  a Krylov process producing the dominant modes first. It would fit the
  existing structure, which is already a truncated modal rational
  approximation with fixed residues and already drops modes whose decay falls
  below a threshold; whether the accuracy can be raised mid-run without
  disturbing the accepted history is the part to check first.

Found while pacing the solver, not yet diagnosed:

- [ ] `simulated_time()` runs backwards for a frame or two at a handoff. The
  offset is only updated when the upload commits, while the step counter resets
  when the buffers install, so between the two the steps taken on the old
  generation are missing from the total. Counted per frame over a twenty-second
  run: 18 of 1876 frames go backwards, worst 1.23 s. Pre-existing and unchanged
  by the step-ceiling work, which measured 19 of 1929 on the same scene.

The null mode nothing removes:

- [ ] Ease a source out as well as in. The switch-on envelope only covers a
  source that is live from the run's start; cutting a sine off mid-cycle injects
  an impulse of the same kind, which is how disabling a source leaves an offset
  behind. Measured while building the envelope: ramping in but stopping abruptly
  cut the drift only twofold, against fiftyfold when the stop was taken out of
  the measurement. A cross-fade between the outgoing and incoming forcing would
  cover enable, disable, and every amplitude or phase edit with one mechanism,
  at the cost of carrying both signals on the GPU and a per-change timestamp.
- [ ] Reproject the constant out of each isolated domain, every displayed frame.
  A constant is in the stiffness operator's null space, and a radiating wall is a
  dashpot with no restoring term, so no outer condition can remove one: a planted
  0.37 offset is still 0.370000 after t = 11 under both reflecting and
  first-order-outgoing walls, while one Dirichlet wall drains it to 9.3e-3. Only
  a component with a pinned node has a determinate constant, so any global
  correction is wrong the moment a reflecting separator splits the domain — it
  has to be per isolated domain, over the coupling graph of the operator's own
  sparsity. Two findings shape the work. Subtracting a constant from both levels
  leaves velocity untouched by construction, so it cannot remove a *drift*: with
  no damping anywhere the mean velocity held +1.211486e-2 to seven digits from
  t = 5.5 to 38.8 while the offset ramped linearly and without bound, so a
  cadenced projection would chase the ramp and flicker at its cadence. And
  removing the drift means removing momentum, which is real kinetic energy and
  not a gauge choice, so it needs deciding rather than doing. Mesh transfer is
  the injector the source envelope cannot reach: interpolating between meshes
  does not conserve the mass-weighted mean exactly, so every remesh can leave a
  little behind.

Follow-ups from the welding work:

- [ ] `detach_endpoint` still leaves a two-arm vertex when the other two arms are
  open ends, so a seam a manual detach produces stays a locked C0 corner until
  something touches that junction. Fold it into the deferred unweld/split work.
- [ ] A curve at the 128-control ceiling cannot be sharpened to C0 and cannot
  accept a divider, because both spend controls and refinement only adds more.
  Reachable at about 41 polygon vertices. The message says so; a pre-emptive gate
  would need the cost of the whole attachment, not just of one knot.

Low priority, correctness rather than anything that shows:

Carried over from the cutover follow-up work, not from the review:

- [x] Retire the pre-cutover `editor` and `persistence` modules (2026-09-16).
  The shared value codecs moved into `topology_persistence`, which was their only
  caller, and brought five round-trip tests with them; 8,977 lines of legacy
  editor, legacy schema, legacy suite and its fixture went with them.
- [ ] Measure representative browser frame timing for the cooperative topology
  job. The cutover recorded that it advances in fixed 256-work-unit slices but
  never measured what that costs in a real browser frame.

Longer-standing work:

- [x] Incremental mesh repair by carving (2026-09-15), replacing the deferred
  coordinate repair and the legacy caps: the core carving job (`mesh/carve.rs`),
  the exact identity path in the transfer map, the runtime's Repairing phase
  with the full rebuild as the fallback and the counts in the Performance
  panel, and the classifier treating every plan difference except the outer
  domain, a resolution change and a requested rebuild as a repair. Browser
  checks for it are listed in `docs/browser-checks.md` and not yet run.
- [ ] Let a repair's refill legalize or split into the first ring of kept
  triangles. Beside the frozen rim, refinement accepts elements it cannot fix,
  because rim edges can be neither flipped nor split; the ceiling-and-floor
  sizing rule removes the pressure that made this catastrophic and
  `DegenerateRepair` refuses a recurrence, but quality there is still accepted
  rather than guaranteed.
- [ ] Deformation-first repair for small motions: move the existing vertices
  with a cap-free displacement field and re-legalize, carving only what inverts.
  Keeps connectivity for nudges; carving stays as the fallback underneath.
- [ ] Live repair during drags. The runtime never sees a gesture today; the
  carve time in the Performance panel says whether a per-frame repair is
  affordable, and cancel semantics for a gesture whose field changes have
  already happened need deciding.
- [ ] Outer-domain resize through carving; it is the one geometry change that
  still takes the full rebuild.
- [ ] Split arrangement segments longer than the chord cap into several atoms.
  An outer wall is one atom per side today, so moving a curve's attachment along
  a wall rebuilds the whole wall band, and pieces longer than the target are
  what makes the two sides of a separated span subdivide independently.
- [x] Raise viewport video capture from the initial 30 FPS implementation to 60 FPS
  (2026-09-16). Measured first: the rate was never the limit, the single readback
  in flight was. Browser encoding is still unmeasured and needs a real browser.
- [x] Replace the vector overlay's run-peak exposure heuristic with a robust
  automatic scale (2026-09-16). One envelope, used by the scalar field as well,
  which had the same problem the other way round.

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
- [ ] Stabilize second-order outgoing conditions on curved hole and baffle spans.
  Diagnosed 2026-09-15 with an ignored reproducer test: the divergence is a
  property of the condition on curved spans, not of the discretization. Postponed
  on 2026-09-15 into the isoparametric effort (plan §10), where curvature is
  available exactly; keep examples on reflecting or first-order curved faces
  until then.
- [x] Decouple the mesh plan's boundary atoms from the arrangement's sampling
  density (2026-09-15, `TopologyMeshPlan::coarsened`). Atoms are merged runs of
  arrangement segments within the meshing curve tolerance and under the target
  edge; linear subdivision inside an atom therefore stays within that tolerance
  of the curve, so no spline evaluation was needed. Coordinate repair will pair
  atoms between plans by span and parameter range.
- [ ] Extend outer-boundary measurements across more angles/frequencies and assess
  whether higher auxiliary orders justify their state and compute cost.
- [ ] Decide whether the load-compatible closed-wall role still warrants assigned
  face conditions now that holes and open baffles cover the primary workflows.
- [ ] Evaluate a dissipative relative dashpot for thin gaps. Keeping centered time
  integration would require an off-diagonal damping solve; the implemented gap
  spring is conservative.
- [x] Make operator assembly and transfer-map construction resumable
  (2026-09-15). Probes and the far field still run inside one slice; they
  measured at 0.0 ms on the Obstacle array, so they stay synchronous until a
  scene shows otherwise. The GPU upload frame is not yet measured in-app; the
  handoff record's `upload_ms` is the place to read it.
- [ ] Replace the sharp zero initialization at newly exposed domain with a localized
  transition/blur pass. A hard jump against the retained field produces artificial
  wideband excitation when an obstacle boundary moves inward. Measure added spectral
  energy and keep established regions outside the transition band unchanged.
- [ ] Run a longer browser soak with a representative multi-obstacle scene and
  record solver throughput and memory behavior over time.
- [ ] Extend coordinate-edit local repair to closed walls if that legacy role
  remains worth supporting.
- [ ] Extend region sources beyond the initial bias-plus-sinusoid time law. Candidate
  follow-ups include bounded time-envelope expressions, pulsed/chirped drives,
  vector source terms when a vector field exists, and nonlinear field-dependent
  source/material laws.
- [x] Implement topology-aware solution-driven AMR on immutable mesh plans as
  specified below.

## 2026-09-16 — Slow motion shortens the step instead of skipping frames

The speed ceiling shipped a day earlier paced by step *count*: a frame banked its
wall-clock time, scaled it, and spent it in whole steps of whatever size the mesh
allowed. Below a ceiling of one step per frame that count floors to zero on most
frames — measured at 0.53 steps a frame with only 52 % of frames advancing at
0.05x. The jumps are not the problem; each one is `1 * dt`, smaller than the
`14 * dt` a full-speed frame moves. The cadence going irregular is, and a
1-0-1-1-0 pattern reads as judder however small the increments.

The threshold is `dt / frame_time`, which is why a coarse mesh suffers worst: the
GRIN rod runs `dt` 6.6e-3, so at 120 frames a second it crosses at 0.8x and
almost the whole slider sits below it.

So beneath that point the step shrinks instead, to the frame budget times the
ceiling, and every frame gets one. Shrinking is always safe — the mesh's figure
is a stability limit, an upper bound, and this never goes above it. Measured
after: 99 % of frames advancing at 0.05x and 95 to 97 % at 0.02x, against 52 %
before, with full speed untouched because the ceiling only binds below about
0.13x on that mesh. The residual few percent are frames that arrive faster than
the fixed 120-per-second budget assumes; halving the budget would close it at
double the step count, which is not worth it.

The estimate of what this would cost was wrong and the correction came from the
user: every mesh handoff already changes the step, so the clock accounting, the
probe strides and the GPU parameter path all already cope. Logged across one run,
`dt` swings 8.6e-4 to 2.0e-3 — a factor of 2.4 — while `simulated_time` stays
continuous. So there was no new machinery to build; a ceiling that wants a
different step just clears the requested revision, the same lever a document load
pulls, and the scene republishes unchanged onto a reused plan, mesh and operator.

One thing that survives the change and one that does not. The eleven sites
reading `recommended_time_step()` now read the step the GPU was actually uploaded
with, because between a speed change and the republish that carries it the two
differ and the clock and probe strides would disagree with the solver. And the
clock dip at handoffs, found while checking this, is pre-existing: 18 of 1876
frames on the unchanged build against 19 of 1929 with the ceiling, so it is
listed above rather than laid at this change's door.

## 2026-09-16 — The solver is paced to a speed again

The simulation speed control did not survive the UI rewrite. The pacing that was
left targeted exactly one simulated second per wall second: a frame's wall-clock
delta was spent at one time step a substep, capped at `MAX_STEPS_PER_FRAME`. It
is scaled by a ceiling now, persisted with the document beside the view settings,
and the panel says what the solver is actually reaching when it falls short.

Two things the measurements changed. The windowed rate dips to about three
quarters of the rate asked for whenever a handoff withholds stepping inside its
half-second window — at every speed, including ones the solver reaches
comfortably, so a direct comparison flashed the note at random. The note reads a
held best instead: instant to a better reading, giving one up at 1.15 a second,
which rides over a dip of a second and still lets a real slowdown through in
under two.

And the note was first suppressed while a handoff was pending, on the reasoning
that the solver is not to blame there. That silenced it almost entirely on
exactly the scenes heavy enough to need it, because adaptation keeps a handoff
pending much of the time: asking for twelve times real time on a 75k-DOF mesh
reached three and said nothing. The held rate already rides over those frames, so
the suppression was both redundant and wrong; only a paused solver is quiet now.

Verified at both ends. Asked 0.1, 0.5, 1 and 2 the reached rate tracks to within
a percent and the note stays quiet through raw dips to 0.82. Asked twelve — past
the slider, which egui clamps, so the range had to be widened for the run — it
reaches about three and says so on every sample. On this machine nothing at or
below the slider's maximum strains it: 78k DOFs at `dt` 8.89e-4 still reaches
two.

## 2026-09-16 — Starting the scale again, at the right moment

Reported: pressing Reset over a residual field flashes one frame of that residue
at full brightness before everything goes black. Reproduced at 0.09 % before the
press, 100 % for about six frames, then zero — the clear fires when the reset is
*asked for*, but the GPU readback still holds the old field for a few frames, so
the scale re-measures the residue and paints it whole.

The same fault was in the document-load path and worse there, because the
outgoing scene stays on display for as long as the new mesh takes to prepare:
measured at four tenths of a second against the reset's one tenth, and it scales
with the scene.

Neither needs clearing at the request at all. A zero field paints as the base
colour whatever the scale says, and the new field takes the scale over as it
grows. So both were removed and the only restart left is the one at commit, where
the readback already holds the new field.

That exposed a third, smaller flash: the first frames after a commit are
numerical dust — 1.5e-11 measured — and an instant attack onto a scale with
nothing behind it latches straight onto that and paints dust at full colour.
Restarting now keeps how loud the session has been, so the quiet floor sits above
the dust: 0.30 % instead of 100 %, and the real field takes over a tenth of a
second later. `clear` is gone and `restart` replaces it, which is the method this
started with before the handoff rule was removed — it was right, it was being
called in the wrong place.

Both measurements were redone after the first pass used a scene with reflecting
walls, where modes with nodes at the radiating walls are trapped and the field
relaxes toward a standing mode instead of emptying. Every document a decay
measurement touches now has all four walls radiating.

## 2026-09-16 — The scale stops falling off a cliff at every handoff

Reported straight after the release retune: the scale jumps down sometimes
instead of easing. A fall faster than the release cannot come from the release —
it is bounded by the rate — so it had to come from something clearing the
reference outright, and it did. `retune_exposures` zeroed it whenever the
solver's generation moved, and adaptation moves the generation every second or
two. Logged against a draining scene, the reference eased down about three and a
half percent per sample and then collapsed at each boundary:

    gen 18 -> 19   8.09e-3 -> 1.21e-3
    gen 19 -> 20   1.09e-3 -> 3.39e-4
    gen 20 -> 21   2.95e-4 -> 7.84e-5

The rule was wrong in its premise. A handoff *transfers* the field: it is the
same field on a new mesh, so its scale carries across untouched. Only a field
actually replaced with zeros — a reset, or another document — starts a new run,
and both of those are now where the clearing happens. The same log now runs
2.96e-2 down to 8.79e-3 over 8.7 seconds through nine handoffs with no step at
any of them, a factor of 3.37 against the release rate's 3.36.

`AutoExposure::retune` went with it. Keeping the run's peak while dropping the
reference only ever made sense for the handoff case, and there is no handoff case
any more.

Two tests cover it: one that a transferred handoff leaves the scale alone while a
zeroed one does not, and one that no path through `update` may drop the reference
faster than the release rate — which is the shape the symptom took. That second
one needed the same `f32` widening the solver does to compare to the bit; written
against `1.0 / 60.0` in `f64` it failed on the last few digits.

## 2026-09-16 — The scale stops chasing a field on its way out

Two reported artifacts, one cause. When the sources stop, the field drains and
then weak reflections slosh and decay — but the view kept them at full
brightness, so the wave looked like it never left. And the high-frequency modes
the grid-scale filter exists to kill swam back into view as the scale caught up
with their artificial decay.

The exposure was already asymmetric — instant attack, geometric release. The
release was simply far faster than anything it was following, at eight times a
second, so the reference sat on the level and the painted brightness never
changed. Recorded a level series from the running app to measure what it has to
beat, and replayed candidate settings against it offline rather than guessing.

The first measurement was on the Starter obstacle and was wrong to use: two of
its walls reflect, so modes with nodes at the outgoing walls are trapped and the
field relaxes toward a standing mode rather than draining. Re-measured with all
four walls radiating, where the field really does empty: it falls two thousandfold
in eight seconds after the sources stop, and the old rate still painted that at
48 %.

| per second | drain tail | 20x spike clears |
| --- | --- | --- |
| 1.15 | 0.07-0.37 % | 21 s |
| 1.25 | up to 1.4 % | 13 s |
| 1.4 | up to 3.8 % | 9 s |
| 1.7 | up to 8.2 % | 6 s |
| 8.0 | 100 % then 48 % | 2 s |

Above about 1.25 the scale catches the slow late decay and the picture creeps
back up, which is the second artifact in miniature. 1.15 never does.

A two-stage follower was built and measured first — a fast release floored on a
slowly-moving baseline — which clears a spike in two seconds instead of
twenty-one. It was dropped: on the tail that matters here it is *worse* than the
flat slow rate, 1.6-1.9 % against 0.07-0.37 %, because its baseline eventually
follows the decay down while a flat release just runs out of room against the
quiet floor. One extra state variable bought only the pulse recovery, so the
simpler change won. The two-stage shape is worth remembering if placing pulses in
sequence turns out to grate.

One objection raised against the simple rate did not survive checking: a slow
release sags to 82 % during steady drive because it cannot follow the level's own
dips. In colour that is `tanh(0.82)` against `tanh(1.0)`, 0.68 against 0.76, which
is not visible.

## 2026-09-16 — The view stops being fooled by a rigid offset

A field that has radiated away leaves a uniform offset behind, and because the
exposure is relative that offset becomes the scale and paints the whole domain
one flat colour. The offset is not part of the wave: a rigid displacement is in
the stiffness operator's null space, carries exactly zero energy — a planted 0.37
uniform field measures 5.0e-15 — and no radiating wall can damp it. So it is
taken out before the field is measured or painted.

Per isolated subdomain, not globally. The null space is spanned by the indicator
of each connected component of the coupling graph that holds no prescribed node,
so a reflecting separator leaves two constants that drift independently and one
global mean would half-centre each. `ConstantModes` unions over the operator's
own sparsity; the pattern is the coupling, because assembly emits no structural
zeros — counted on a cavity at two resolutions, 7161 and 28817 entries, none of
them zero. A component with a Dirichlet node is skipped: its offset is part of
its solution.

Once per rendered frame rather than on a solver cadence. That matters: subtracting
a constant from both levels leaves velocity untouched by construction, so a
cadenced projection cannot remove a drift and would re-accumulate between firings
— a sawtooth at the cadence. Correcting every frame has no such problem, and it
costs nothing because the frame already walks the field for the exposure's
quantile.

Because the correction hides what it removes, and an undamped offset grows until
it eats the mantissa the wave is carried in, `Performance diagnostics` reports
each subdomain's offset and its drift rate, and the status marker is raised once
when one passes a thousand times the field's own scale. Single precision carries
about seven digits; three spent on an offset still leaves the wave legible, past
that it does not.

Also here: `field_auto_exposure` brings the manual scale back — off, the
intensity slider is the whole scale again, exactly `tanh(value * gain)` as it was
before the field measured its own. The subdomain centring is a separate concern
and stays on either way. And the vector overlay's mode combo had no label at all,
which is a poor way to be discovered; it has one now, matching the Overlay combo
below it.

One test caught a mistake of mine rather than the code's: asserting that the wave
survives centring untouched is wrong, because centring removes the component's
whole mean, the wave's own share included. The property that actually holds is
that centring is a shift, so every difference within a component survives it.

## 2026-09-16 — Sources stop kicking the domain on the way in

A harmonic source started from rest at a phase whose cosine is not zero hands the
domain a net impulse: integrating `A sin(w t + p)` from rest leaves the term
`(A cos(p) / w) t`. Demonstrated by driving the same scene at two phases and
measuring the mass-weighted mean afterwards — `cos(p) = 1` settled at +1.09e-2,
`cos(p) = 0` at +2.2e-4, fifty times less.

Nothing takes that impulse back. A constant is in the stiffness operator's null
space, and a radiating wall contributes damping and no restoring term, so it is
transparent to a static offset: a planted 0.37 offset reads 0.370000 after t = 11
under reflecting *and* under first-order-outgoing walls, identically, while one
Dirichlet wall drains it to 9.3e-3. With an absorbing wall the drift velocity is
damped and the offset settles; with none it does not, and the mean velocity held
+1.211486e-2 to seven digits from t = 5.5 to 38.8 while the offset ramped
linearly and without bound.

So every source in a scene is now eased in together by one smooth envelope over
four periods of the slowest of them. One envelope rather than one per source,
because a phased array steers with the phases *between* its sources and a
per-source delay would turn the beam; and an envelope rather than starting every
source at a cosine phase, because the phase is the user's and is persisted.
Integrating by parts, a ramp leaves the residual impulse at `1 / (w * ramp)`
instead of `1 / w`, and the vanishing slope at both ends of a smoothstep takes it
to second order. Measured in a reflecting cavity: the switch-on drift fell from
6.47e-3 to 1.21e-4, a factor of 53. In the app the residual offset over a
thirteen-second run fell to a hundredth of the field's own spread, from a half on
the Starter obstacle to -0.01.

The first version of the test measured two-fold, not fifty-fold, because it
stopped the drive abruptly at the end — and cutting a sine off mid-cycle injects
exactly the same kind of impulse. The test now drives continuously and samples a
whole number of periods apart so the oscillating part cancels. Easing a source
*out* is a separate piece of work and is listed above; this covers a source live
from the start of a run, which is what every bundled example is.

## 2026-09-16 — A document load kept the field it was replacing

Reported as a large uniform offset when switching examples, and logged yesterday
as electromagnetic-to-mechanical. That was wrong. Mapping the transitions:
Material lens (EM) to Phased array is clean, Obstacle array to Luneburg is clean,
and Luneburg to *anything* is not — mean over standard deviation of +47, +35 and
-15 into the Obstacle array, the Phased array and the Material lens. The trigger
is the predecessor's amplitude, not its physics, and the Luneburg lens is the one
example whose field is fifty times the rest.

Tracing the uploads found only the first one of a session `fresh`. Every load
after it took the transfer path, so the outgoing scene's field was carried into
the incoming one. Its oscillation radiates out — standard deviation fell 0.486 to
0.007 — and the constant left behind cannot be removed by any wall the scene has,
so it stays as a flat wash.

The cause is one flag doing two jobs. `set_document` raised `reset_requested`,
which the GPU reset spends earlier in the same frame and against the topology
still active — the scene being replaced. So a load reset the *outgoing* scene,
which then ran on with its sources live for the seconds its replacement took to
prepare, and handed over a full-amplitude field at the end of it. `fresh_requested`
is now separate and is only spent by the preparation request.

A forced GPU reset was tried first and changed nothing, which is what said the
flag was not the lever: the upload that follows re-installs the field over the
top whatever the reset did.

| Load | before | after |
| --- | --- | --- |
| Luneburg to Obstacle array | +47.1 | +0.23 |
| Luneburg to Phased array | +35.4 | +0.39 |
| Luneburg to Material lens | -15.3 | +0.25 |

## 2026-09-16 — The field sets its own scale

Reported: the scalar overlay is too faint in several examples and at the default.
Measured before changing anything, by stepping every catalog example on the CPU
reference solver at the app's own mesh edge with its real sources, out to
simulated time 4. Steady-state 90th percentile of the nodal magnitude, and the
colour the default gain of 2.0 paints:

| Example | p90 | at gain 2 | at gain 12 |
| --- | --- | --- | --- |
| Starter obstacle | 3.1e-2 | 6.2% | 35.7% |
| Double slit | 2.3e-2 | 4.6% | 26.9% |
| Material lens | 7.5e-3 | 1.5% | 9.0% |
| GRIN rod | 6.0e-3 | 1.2% | 7.2% |
| Anisotropic crystal | 7.6e-3 | 1.5% | 9.1% |
| Luneburg lens | 7.1e-1 | 89.0% | saturated |
| Phased array | 1.8e-2 | 3.6% | 21.1% |
| Obstacle array | 1.5e-2 | 2.9% | 17.3% |

So it was never a default-value problem. The amplitudes span 118x and the slider
spans 48x, so no setting of it serves the catalog: at the maximum, three examples
still paint under a tenth of full colour while the one that reads well at the
default washes out flat. The default was left exactly where it was and the scale
is measured from the field instead.

The vector overlay already had an automatic scale and it had the opposite fault:
a monotone run peak that never came down, so one **Place pulse** darkened the
overlay for the rest of the run. It was also only ever cleared on an overlay mode
change — not on a document load — so clicking from the Luneburg lens to the GRIN
rod in the new gallery scaled the arrows by a reference 118 times too large, well
short of the 1e-4 guard that would have silenced them. One mechanism now serves
both.

The shape that works: rise to a louder field at once, so nothing is clipped; fall
back by at most a fixed factor a second, so a spike leaves the scale in a second
or two; and never fall below a thousandth of the loudest level the run reached,
which is what refuses to magnify decayed noise. Falling by a factor rather than
by a difference is the part that matters and it was not the first attempt — a
linear release toward the current level took five seconds to give up a twentyfold
spike and would have taken longer for a larger one, which the recovery test
caught. A geometric release takes the same time whatever the size of the spike,
which is the only behaviour that reads the same on a field of 6e-3 and one of
7e-1.

A new solver generation clears the scale but keeps the run's loudest level, so a
field decaying through an adaptation handoff is still held down; loading another
document clears both. The field's quantile is read from a strided sample of about
4096 nodes rather than by sorting sixty thousand every frame, which is fair
because nodes are numbered in meshing order.

Checked against the real GPU field, not just the CPU prediction: every example
settles with about half its nodes in the lowest quarter of colour, a third in the
next, and two to three percent above three quarters. The references the running
app measured — 4.7e-2 for the Starter obstacle, 1.0e-2 for the GRIN rod, 1.03 for
the Luneburg lens — track the CPU numbers above, which is a useful check on the
reference solver as well.

That sweep turned up something unrelated, now a TODO: loading a mechanical
document over an electromagnetic one leaves the field with a large uniform
offset. It is pre-existing — the exposure only made it legible — and it is not
touched here.

## 2026-09-16 — Capture was never limited by the rate it asked for

The TODO said to measure before raising 30 FPS to 60, and the measurement is the
whole story. Recording the Obstacle array for ten seconds on an M1 Max at 60,
with nothing else changed, wrote 596 slots of which only 329 carried a new frame.
The rest were the previous frame held. At 30 it was 282 of 297. So asking for 60
bought 33 distinct frames a second against 28 — twice the file for almost nothing
new.

The limit was `recording_readback_in_flight`, a single `AtomicBool` that let one
`Screenshot::primary_window()` be outstanding at a time. A readback's round trip
is about 35 ms while the app renders a frame in 8 ms, so one in flight caps
capture near 28 a second whatever rate the slots are cut at. Three in flight
covers 565 to 577 of ~596 slots, and the app's frame time does not move: median
8.3 ms idle, 8.3 ms capturing, at both depths. Depth 2 gets most of it and 4 a
little more; 3 is where the curve flattens.

Pipelining costs ordering. Readbacks do finish out of order — one in six hundred
over several runs, always adjacent slots — and the old arithmetic would take
`previous_slot` backwards and then over-insert held frames against it, drifting
the file a frame per inversion. The written slot only moves forward now:
`slot_action` skips a frame that arrives behind one already written, because the
slot it belonged to was filled from the frame before it and writing it late would
push everything after it late too. That skip is deliberately not counted as a
dropped frame — no time is lost and nothing about the machine's throughput is in
question, and the dropped count has to stay a signal that capture cannot keep up.

The cap on a frozen run was `min(150)`, a frame count that quietly meant five
seconds at 30 and would have meant two and a half at 60. It is `held_frames(fps)`
now, off a `MAX_HELD_FRAME` duration.

Browser capture is untouched and unmeasured: its `captureStream(fps)` cap rises
to 60 with the constant and the `requestAnimationFrame` draw loop already runs at
display rate, but what `MediaRecorder` does with twice the frames on a real
browser needs a real browser.

## 2026-09-16 — The example gallery comes back, and a new scene has somewhere to start

The cutover left Examples as a flat submenu of eight names with the description
on hover. The gallery it replaced — own window, thumbnail, heading, description
— could not be reverted: its painter walked `scene.obstacles` and
`scene.internal_boundaries`, and the old catalog carried a cached
`property_preview` triangle soup that the topology catalog does not have. So the
window is restored and the painter is new.

A thumbnail is built from the example's own compiled scene: `compile(0)` gives
faces with their boundary cycles and an assignment from face to region, region
gives material, material gives colour. Which property shades a spatial example
needed no decision — the catalog already says. GRIN rod and Luneburg lens carry
`Property(WaveSpeed)`, Anisotropic crystal `Property(Anisotropy)`, Phased array
`Property(VolumeSource)`; the rest carry `Regions` and fill flat.

Faces are rasterized by scanline rather than triangulated. A face is an
arbitrary polygon with holes, so filling it needs either a triangulator or a
rasterizer, and the rasterizer answers a second question at the same time:
throwing every cycle of a face into one crossing list and pairing the crossings
even-odd excludes the face's own holes with no extra bookkeeping. A flat face
costs one quad per row; only a shaded face pays per cell.

Sampling the property at cell centres rather than at polygon corners is the
whole point, and there is a test named after why: every corner of a Luneburg
lens sits on the same circle, so a profile read at the corners alone is one flat
colour. The rendered preview shows the radial gradient it should.

Cost decided the caching. Compiling the eight examples takes 0.03 to 19.5 ms
each in release, 35 ms for all of them, so the window builds at most one preview
per frame: a row shows its name and description at once and its thumbnail a few
frames later. The grid is 48 rows down the domain, under three pixels a row;
72 rows doubled the quads a shaded example costs and changed nothing visible.

New starts `TopologyDocument::default()` — nothing drawn, one background
material, every wall already second-order outgoing — with the point source
switched on, because an empty scene that makes no wave is a still picture rather
than a starting point.

The gallery stays open across a pick, so the catalog can be clicked through, and
the row the document came from is marked. Any other load clears that marker,
which is why it is cleared in `set_document` and set again by the one path that
knows better.

Startup with nothing to restore used to open the first catalog entry every time;
it now opens one at random. The pick lives in the startup block rather than in
`Playground::default`, so the twenty-nine tests that build a default playground
stay deterministic. Randomness without a dependency: the wall clock's sub-second
bits natively, `Math::random` in the browser, where `SystemTime::now` is not
available. A corrupt autosave counts as nothing to restore, which it did not
before — the old branch fell through and kept the startup document.

Seven tests, each run first against a deliberately broken implementation: a face
filled rather than traced, the radial profile reaching the thumbnail, scanline
spans skipping holes, a new scene being empty and outgoing and driven, the
gallery surviving a pick and the marker not outliving its scene, every catalog
entry previewing with the cache filling one per frame, and the random index
staying inside the catalog at a fraction of exactly one.

Checked: `cargo fmt --all`, `cargo clippy --workspace --all-targets --locked
-- -D warnings`, `cargo test --workspace --locked`, `cargo build --release -p
funfern-app --locked`.

## 2026-09-16 — The last schema with two owners

The version-22 file was written by `topology_persistence` but its scalar values
— physics, material coefficients, time signals, wall and face conditions,
splines, presentation, probe presets — were encoded by `persistence`, the
pre-cutover schema, which in turn took its document types from `editor`, the
pre-cutover editor. So the live file format sat on 5,871 lines of code nothing
else called, and the only direct test of the codecs it borrowed was
`tests/editor.rs`, a suite about loops, holes and material interfaces that the
application no longer has.

The codecs went into `topology_persistence` rather than into a module of their
own. They had exactly one caller, and a module that exists to serve one caller
is a hop, not a boundary; the whole version-22 schema now reads top to bottom in
one file. Two migration wrappers came off on the way: `StoredScalarField` was an
untagged enum over a bare version-14 number and the tagged shape, and
`StoredTimeSignal` an untagged enum over the version-17 shape and the
version-16 one. Version 22 writes only the tagged shape and, since it accepts
only its own version, can only ever read it back, so both collapsed onto the
tagged inner enum. That is byte-identical on the way out and a tightening on the
way in: a bare coefficient inside a version-22 file is now turned away by the
reader rather than by a hand-written message.

Byte-identity was the thing to prove, not to assume. A throwaway test hashed
what `save` produces for the default document, all eight catalog examples and
the shipped `eight-obstacles.json` before the move and again after: the same ten
digests and the same ten lengths. Nothing a user has on disk sees any of this.

Five tests replace what the deleted suite covered, and cover more than it did:
every physics model and polarization; every wall and face condition carrying its
own signal; every material and vector overlay and every sampling preset; a
version-22 file written by an earlier build, which still loads because six
presentation keys have serde defaults and the retired adaptation-target flag
still migrates to the overlay it became; and the value guards — a formula that
does not parse, a bare coefficient, an untagged signal, a non-finite control
point. Each was run against a deliberately broken codec first: nine breakages,
nine failures.

Two of those tests were wrong when first written, and the code was right both
times. A thin-gap coupling is only a law between two reflecting faces, so
rotating arbitrary conditions through it is not a valid span. And two walls
meeting at a corner may not resolve to Dirichlet with different signals — in
mechanical physics an electric wall resolves to Dirichlet with a zero signal, so
it cannot sit beside a prescribed one. A pass now wears one condition all the
way round, with a separate mixed-but-legal set to pin which slot is which wall.

One guard turned out to be unreachable from a file: a control point of `1e400`
is rejected by serde_json as out of range before `decode_open_spline` can call
it non-finite, and JSON has no way to write an infinity or a NaN at all. The
guard stays — the codecs are callable from elsewhere — but the test now checks
the reader and the codec separately rather than claiming one covers the other.

Checked: `cargo fmt --all`, `cargo clippy --workspace --all-targets --locked
-- -D warnings`, `cargo test --workspace --locked` (121 app lib, 95 app bin, 2
examples, 1 shaders, 215 core lib, 33 geometry, 30 mesh), `cargo build --release
-p funfern-app --locked`.

## 2026-09-16 — The grid shows what it snaps to

Two small ones from the outstanding list.

The README named a control called "Flip direction" in two places. Neither
exists: a line probe has **Swap ends**, which reverses the arclength axis and
the flux sign together, and a boundary probe has **Reverse direction**, which
turns the axis alone and leaves the side and the sign where they are. The
behaviour is right in both and they are not the same operation, so the two names
stay and the README says them.

The grid stepped in 1/2/5 per decade from the zoom while Shift snapped to a
fixed 0.05, so at most zooms it drew one lattice and landed on another. The plan
was to snap to the drawn step, and that turns out to be too coarse: at the
default zoom it draws fifths of a unit, and snapping to those is a quarter of
the precision placement has today. So each decade divides instead - a 2 into
four parts, a 1 or a 5 into five, all round numbers - the divisions are drawn
under the labelled lines at a third of their alpha, and Shift lands on them. At
400 pixels per world unit that puts the fine step at exactly the 0.05 it used to
be nailed to, so nothing about placing anything feels different; it just follows
the zoom now, and a line is drawn wherever a point can land.

`snap_point` and `snap_scalar` take the step rather than reading a constant, and
every caller asks the viewport for it. The two tests that held the old constant
name their zoom now, which is the honest shape for a test of something the zoom
decides.

Checked: fmt, clippy -D warnings, workspace tests, release build.

## 2026-09-16 — The probe rings outlive the mesh that filled them

A handoff rebuilt the point, curve and area rings from scratch, so whatever the
GPU had written since the last readback went with the old buffers: 2 to 4
samples at 120 Hz, a 33 ms hole measured over 25 adaptations.

This entry used to say what it would take, and was wrong about it. The claim was
that the rings had to be rekeyed to the wave clock first, the way the far
field's are, "because their write cursor is still a generation-local step count,
which is why keeping the buffer across a handoff would scramble it". The cursor
is indeed `(completed / stride) % frames` and does restart with the generation.
It does not matter. Every readback sorts its records by time before the host
sees them, and ingestion takes a record only when it is newer than the trace's
last. Slot order carries no meaning at all, so a ring that resumes writing
somewhere else is not scrambled - the stale entries ahead of the cursor are
older than what has already been ingested and are dropped, and the entries the
handoff would have thrown away are newer and are kept. Which is exactly, and
only, what was missing.

So the fix is the one the far field already had and no shader changed: when the
clock has not restarted and the recorder set is unchanged, keep the output ring
and replace the stencils and control around it. Identity is the ordered probe
ids for points and areas, and the descriptors - id, offset, count - for lines,
because those are what the ring is addressed by. A probe added, a line reshaped,
or a reset starts a fresh ring.

Three smaller things went with it. `FarFieldHistory` is `RecorderHistory` now,
since four recorders share it and one flag decides for all of them, and the step,
the physics and that flag travel together as a `RecorderContext` rather than as
three more arguments each. And the three update functions used to clear their
buffers before validating their arguments, which was harmless when every path
cleared anyway; now that the success path keeps them, each rejection clears
explicitly.

Checked: fmt, clippy -D warnings, workspace tests, release build. The test
holds all three rings across an adaptation at a shorter timestep, and watches
them restart for a restarted clock, an added probe, and a reshaped line. Held
against the keep being removed for points and for lines, which fails it.

## 2026-09-16 — The shaders are read by the suite now

Nothing in the workspace parsed the seven WGSL kernels. Every edit to them was
checked by hand against a throwaway naga crate outside the build, which worked
because it was remembered, and did not the once it was not: `filter` is a
reserved WGSL keyword, and that reached the device.

naga 29.0.4 was already in the lockfile under wgpu 29.0.4, so pinning it as a
dev-dependency adds no tree and means the parse the test does is the parse the
app's own device does. `tests/shaders.rs` reads every `.wgsl` beside the source,
parses it, and validates it. It refuses to pass on fewer than seven, so a test
looking in the wrong directory fails rather than reporting nothing.

Held against real breakage before it was kept: renaming a local to `filter`
fails with "name `filter` is a reserved keyword", the original bug; giving a
`let` a type its initialiser does not have fails with the mismatch. It does not
cover backend translation, so a Metal or browser problem still needs the app.

Checked: fmt, clippy -D warnings, workspace tests, release build.

## 2026-09-16 — A weld asks the same question a deletion does

The follow-up said a weld can merge subdomains, and the obvious objection is
that welding only inserts: it joins two curves, or attaches an end, and never
removes an edge. Both halves are true, and they are compatible, because
separation is not a property of edges existing. It is a property of a circuit
closing. A divider separates two faces only while its path runs wall to wall,
closes into a loop, or reaches a wall through junctions.

The demonstration is one line: a baffle from the bottom wall to the top gives
two faces and two regions, and deleting its last span - which removes no edge
between those two faces, only shortens the divider - makes
`span_removal_choices` return both regions. The faces merged because the divider
stopped short.

A weld inserts, but it also moves the end it welds. So welding a divider's loose
end onto a free-standing baffle reattaches it to something that reaches no wall:
the circuit opens, you can walk around that baffle's tips, and the two regions
land on one face. Reproduced as `Err("two anchors claim the same subdomain")` -
the compiler's `DuplicateFace`, which is a true report of an unanswered question
rather than of a broken document.

So the weld now asks. `weld_endpoint` takes the survivor and returns
`TopologyWeldOutcome`: welded, or the regions it would fold together. Deletion
and welding share the settlement now - `Landings` records where every authored
assignment landed and which face two regions landed on together, and
`settle_merge` decides which face keeps which region and what becomes of the
regions, sources and probes left without one. The removal plan holds a
`Landings` where it held four fields.

A question spends no ids: the weld snapshots the three allocators and restores
them before reporting one, because the caller asks and welds again, and
`refine_curve_to_spans` and `assign_new_faces` both reserve ids on the way
through. A test holds that, along with the document being untouched.

In the UI the staged question is now a `PendingMerge` over both kinds, and the
scene picker does not care which it is answering. The weld reports whether it
settled, so a question that still stands is not cleared by an answer that failed.

Checked: fmt, clippy -D warnings, workspace tests, release build. Three tests:
the question and the untouched document and allocators, the answer deciding
which material is left, and the staging and picking through the UI. The existing
weld suite went through a shim rather than being rewritten, so all nine of its
cases still watch the same path.

## 2026-09-16 — One Delete is one deletion

The log had this as a history-entry defect: a mixed selection lands as two undo
steps, the curves that needed no question and then the one that did. Reproducing
it found something worse. `delete_selection` looped curve by curve and `return`ed
at the first one needing a survivor, so **everything after it in the selection
was never deleted and nothing said so**. Two subdomains selected, Delete, answer
the question: one goes, the other stays, no message. Had the question landed on
the second curve instead, the first would already be gone in its own entry -
which is the case the log recorded, and only half the story. The same function
had a second silent skip: a selection covering one curve whole and part of
another deleted the whole one and ignored the run.

The rule the user asked for turned out to be the rule already written. Every
merge a deletion makes is worked out from where the old faces land in the
recompiled arrangement; one landing face collecting two or more regions is the
question, two such faces are refused with "This deletion would merge subdomains
in more than one place". It had simply never been shown more than one curve at a
time. So no second picker and no gathering of several answers: `RemovalTarget`
became `TopologyRemovalTarget`, which names any number of curves going whole plus
at most one cut down to a contiguous run, and `plan_removal` takes them all
before it prunes, promotes, fuses and compiles. Everything after that point was
already written against the whole candidate and did not change.

What falls out: the question is asked once, over the whole deletion, so two
subdomains merging into the background across two deleted curves are offered
together - three candidates where the old path offered two and then dropped the
rest of the gesture. Many-to-many is refused before anything is touched. And the
history entry is single by construction rather than by bracketing, so
`remove_curve_during_edit` and the `settle_editor` that fed it are gone, along
with `delete_span_selection`, `TopologyCurveRemoval`, `TopologySpanRemoval` and
one of the two removal reporters.

Two partly selected curves stay refused - a cut is defined against one contiguous
run, and two of them are two questions about where the pieces land. The message
now says what to do about it rather than naming a restriction that no longer
holds for whole curves.

Checked: fmt, clippy -D warnings, workspace tests, release build. Five tests: the
two silent skips as gesture-level regressions with the undo entry asserted, a
loop and the separator across it deleted together so every remaining face is dead
and the merge has to come from where the faces landed, two subdomains in two
halves refused together while either alone is an ordinary question, and the
target's one-cut rule.

## 2026-09-16 — The accuracy reading was leaving the panel between estimates

The estimate line added above appeared only while an estimate was in hand, and
committing a mesh drops the estimate it was measured against - which every
adaptation does. So on each cycle the line blinked out and back and took the
source, pulse and energy sections below it down a line and back up, under
whatever the pointer was on. It holds its place now and says it has nothing
instead. The gold line about a forcing under the element floor stays
conditional: that is a standing condition, not one that flickers.

Checked: fmt, clippy -D warnings, workspace tests, release build.

## 2026-09-16 — The mesh had a step down and no idea of enough

Adaptation drove the mesh to its smallest element and stayed there. The target
that would have stopped it was not missing by design: `AmrQuality` - Fast,
Balanced and Detailed, setting the error tolerance, elements per wavelength, the
coarsening scale and the transaction budget - went out in `eddad77 Switch
production app to unified topology` as collateral of the `ui.rs` rewrite, and
nothing replaced it. `refresh_amr` has been building every estimate with
`..Default::default()` since, so the tolerance has been a hidden 0.06, and the
README went on promising the presets.

Exposing that tolerance alone would have changed almost nothing, which is the
part worth recording. The estimator turns an indicator into a size by
`scale = sqrt(tolerance / indicator)`, clamped to [0.6, 2.2]. Once the indicator
is above 2.78 times the tolerance the clamp binds, and from there the element
shrinks by the same 40% every cycle whatever the tolerance says, with nothing
checking whether the last refinement helped. Measured on a square carrying a
smooth bump on one side and a grid-scale ripple at 1e-4 of its peak on the other
- a wave that has passed, leaving speckle - sweeping the tolerance from 0.02 to
0.5, a factor of 25, moved the quiet half's target not at all (0.042 throughout)
and the loud half's by 44%. The ripple on its own had already taken the quiet
half from a mean indicator of 0.006 and a target of 0.145 - coarsen me - to 10.2
and 0.042, and the refine candidates from 1412 to 2516. Raising `amplitude_floor`
five orders, to a tenth of the mean element energy, did not move that target
either: the estimator is right that a grid-scale ripple is unresolved, and
refining it only moves the ripple to the new grid scale.

The estimator itself is in good order, which is what makes a target worth
having. On a real instant of a real solution - a bump carrying the acceleration
the wave equation asks of it, died away long before the wall - the relative error
of the whole field in the energy norm falls second order under uniform
refinement: 0.418, 0.110, 0.029, 0.0072 at h = 0.16, 0.08, 0.04, 0.02. Two things
follow. The three accuracy presets land near the three resolution presets, and
the hidden 6% was asking for finer than the default mesh at all times. And at
h = 0.04 the whole field is inside 3% while 3582 elements individually still ask
to be refined - which is exactly the disagreement a target has to settle.

So `SolutionIndicatorReport` now carries `global_indicator`, that whole-field
relative error, and splits `refine_candidates` into the ones the error estimate
asked for and the ones a limit asked for: too coarse for a forced wavelength, or
larger than the largest element allowed. Limits are floors rather than judgements
about error, so they refine on their own account; error-driven refinement stops
once the whole field is inside the target. The trade is explicit and worth
stating - error concentrated in a small part of a field that is comfortably
inside the target stops being chased. One number for the whole field buys the
stop, and that is what it costs.

The control is Coarse, Medium and Fine at 24%, 12% and 6% over a slider that
reaches everything between, named and shaped like the mesh resolution presets
above it because they answer the same question at either end of the loop.
Elements per wavelength - six, which is twelve nodes quadratically, against a
silent five - and the smallest and largest element moved into Advanced settings.
That fold is drawn after the adaptation section, so the settings tuple that drops
an estimate in flight is compared at the end of the panel rather than in the
middle of it, and a test pins each of the five.

One more thing, worth knowing because it has nothing to do with the error
target: the wavelength rule caps every element at `c / (f * n)` wherever a source
or a boundary signal forces a wave, whether or not the wave is anywhere near.
With the default 2.5 Hz source and unit wave speed that pins the whole domain at
0.067; past 8.3 Hz it asks for less than the 0.02 floor and the mesh sits at its
smallest element with the accuracy target reading satisfied. The panel now says
so when that happens.

Checked: fmt, clippy -D warnings, workspace tests, release build. Three core
tests and three app tests; each core test was run first against a deliberately
broken implementation and all three failed.

## 2026-09-16 — A subdomain marker was standing where the triangles were

The marker for a subdomain probe drifted on every remesh, and the guess in the
report was right: it was placed by the mesh. `region_anchor` averaged the
centroids of the region's triangles with equal weight, so a triangle counted the
same whether it covered a thousandth of the region or a tenth of it, and the
marker was pulled wherever triangles were dense. Boundaries are meshed finely, so
it leaned against whatever the region wrapped around; adaptation refines wherever
the wave is, so under adaptation it followed the wave about.

Measured on a scene with a small subdomain in one corner, meshing the same
geometry at three densities moved it (-0.196, +0.162), (-0.057, +0.033),
(-0.005, -0.006) - about a quarter of a unit on a two-unit domain, all of it
mesh.

It now comes from the compiled geometry: the area-weighted centroid of the faces
the region is assigned, over `CompiledFace::centroid` and `::area`. Nothing about
a mesh enters, so no remesh or adaptation can move it. Area-weighting the mesh
triangles instead would have fixed the drift under refinement too - splitting a
triangle preserves its area moment, and it agreed with the geometry answer to
6e-5 - but it would have left the marker decided by a thing that has no say in
where a region is.

Two smaller things came with it. The anchor is read from the draft scene falling
back to the accepted one, so the marker follows an edit instead of holding still
and jumping at commit. And placing it no longer needs a committed candidate, so
the probe metadata token had to survive there being none: a subdomain probe is
now marked in a scene that has never been meshed, where before it had no marker
at all.

Unchanged: for a region shaped like a ring the centroid is in the hole. That was
true of the mesh average too, and giving it a guaranteed interior point is a
different job.

Verification: `cargo fmt --all`, `cargo clippy --workspace --all-targets
--locked -- -D warnings`, `cargo test --workspace --locked`, and `cargo build
--release -p funfern-app --locked`. Two new tests, both checked against the old
implementation: the marker exists without a mesh and sits away from the hole its
region wraps, and it is identical across a 323 and a 2198 triangle mesh of one
scene, which the average moved by 0.041.

## 2026-09-16 — The probe's arrow points at its marker, not away from it

The first version of the indicator above read badly in the scene, and the fix
turned out to cost nothing. The arrow was drawn from the badge outwards, so it
named the side it read only by where its tail was, which is not how an arrow is
read. Drawn the other way - from the sampled side into the badge - it says where
the reading comes from, which is the question someone actually has.

That is the same vector. The sampled side lies at `-outward` from the badge, so
an arrow running from there into the badge travels `+outward`: it still points
the way positive flux points. Both meanings sit on one mark, and the earlier
version had them too, just translated to the side nobody was asking about.

With the arrow living on the side it reads, the offset band said the same thing
a second time, so it is gone. The arclength arrow was 5 pixels tucked onto that
band and easy to miss; it is now full size and leaves the middle of the normal
arrow's stem, so the two read as the probe's local frame - tangent along the
arclength axis, normal across it - clear of the path. Hung off the stem's tail
instead, the pair read as two marks that happened to meet, and looked misplaced;
from the middle it is one mark. The normal's head lands on the badge ring rather
than short of it, which ties the stem to the marker instead of leaving it
floating beside the curve.

Anchoring everything at the midpoint removed the reason for
`boundary_probe_orientation` to take a fraction, and with it the wrinkle that
the fraction was measured from whichever end the arclength started at. It now
answers for the badge alone.

Verification: `cargo fmt --all`, `cargo clippy --workspace --all-targets
--locked -- -D warnings`, `cargo test --workspace --locked`, and `cargo build
--release -p funfern-app --locked`. The tests carry over; the arclength one now
holds that reversing turns one arrow and moves neither the corner nor the side.

## 2026-09-16 — A boundary probe now says which trace it reads

A span's two traces are geometrically coincident, so a boundary probe drew as a
line with a badge and nothing in the scene distinguished the side it sampled
from the side it ignored. Neither did anything show which way its arclength axis
ran. Both were in the inspector only.

A line probe carries one arrow because Swap ends flips the arclength order and
the flux sign together. A boundary probe cannot: `side` picks the trace, and
with it the direction positive flux points, while Flip reverses the arclength
axis alone - `compile_boundary_probe` filters the planned pieces by `side`
whatever `reversed` says, and only reverses their order afterwards. So the two
facts need two marks, and the side needs a third that is not an arrow, because
an arrow leaving a side names it only by its tail.

The scene now draws a thin band offset onto the sampled trace, a chevron on that
band a quarter of the way along the arclength, and at the badge the line probe's
own arrow along the outward normal. The band reuses `side_offset`, which the
editor already uses to show a selected span's side, so the two agree by
construction. The chevron's quarter is measured from wherever arclength starts,
so reversing moves it to the other end rather than only turning it.

The directions come from the drawn polyline: its samples run with increasing
curve parameter, `Left` names the face on that side, and the normal leaving a
left trace is therefore the right one. Getting that backwards is invisible in a
screenshot and wrong in a reading, so a test in the runtime pins it to the
solver rather than to the reasoning: on a closed subdomain, every planned left
trace covers the face `face_at` finds on its left, and every stencil sample's
`outward_normal` points right of the nearest segment of the polyline the scene
draws. Both halves fail when either is flipped.

That test first went to a free separator and failed. The first sample sits on
the dangling tip, where the boundary wraps around the end and the element behind
it can be on either side - a real ambiguity at a free end, not a wrong
convention. A closed loop has no such point.

`polyline_midpoint` came out into `polyline_anchor`, which returns the segment
carrying the point, since a mark needs the direction there and not only the
place. The midpoint is now that at a half, falling back to the first point as
before for a path too short to walk.

Verification: `cargo fmt --all`, `cargo clippy --workspace --all-targets
--locked -- -D warnings`, `cargo test --workspace --locked`, and `cargo build
--release -p funfern-app --locked`. Five new tests: the anchor and its segment,
the normal leaving the trace on both sides, reversal turning the chevron and not
the side, a path too short to orient, and the runtime's convention check.

## 2026-09-16 — The grid had no rows, and was under the field anyway

Two things, and together they made the toggle do nothing visible.

`draw_grid` took its bounds from opposite corners of the viewport, and `world`
flips y, so the bottom of the screen is the *smaller* world coordinate. The
vertical loop started at the top and ran while the value was below the bottom,
which is false on the first test: the grid had only ever drawn columns. At an
800x600 view, 300 pixels per unit, the step is 0.5, `y` starts at 1.0 and the
condition asks `1.0 <= -1.0`.

What was left was painted before `draw_solution`, and the field wash is opaque -
`field_color` lerps from a solid base when no overlay is under it - so the
columns were covered across everything meshed, which is the whole domain. Only a
scene with no mesh yet could show them.

The grid now draws after the field and before the geometry, so it sits over the
wave and under the curves and handles, and it tints rather than covers: a cool
gray at low alpha, roughly double on the two axis lines. The old `from_gray(42)`
was chosen to sit beneath the field and would have read as solid dark lines on
top of it.

Both loops now walk upwards from the lower bound through one `grid_lines`, which
returns nothing for a reversed or degenerate range rather than looping, and the
caller came out into `grid_axes` so the direction itself is what the test holds:
swapping the two bounds back reproduces the empty row list.

Not changed: the grid steps in 1/2/5 decades from the zoom while Shift snaps to
a fixed 0.05, so at default zoom the lines are drawn at 0.5 and snapping lands
between them. The grid does not show what it snaps to, which is worth deciding
separately.

Verification: `cargo fmt --all`, `cargo clippy --workspace --all-targets
--locked -- -D warnings`, `cargo test --workspace --locked`, and `cargo build
--release -p funfern-app --locked`. One new test over the spacing and both
axes, checked to fail on the old direction.

## 2026-09-16 — A hole absorbed is a face decided

A contour in the autosave refused almost every span deletion with "Removal
merged unrelated face assignments". Driven span by span: one worked, twelve
failed.

The contour's interior is a hole inside the region-1 background. Deleting a span
opens it, so the interior stops being enclosed and its face joins the
background - two anchors then resolve to one face, the outer one naming region 1
and the hole's naming nothing. The merge detector counts only active regions, so
it saw one, declared no merge and left the face out of `forced`; the rebuild then
found two anchors for a face nobody had decided and refused. The one span that
worked was the one carrying the hole's own anchor, which dies with it and leaves
a single anchor to land - which is why it looked like almost every edge rather
than all of them.

A face that several anchors land on which name at most one region between them
is now decided rather than left to the rebuild. Two or more regions would have
been the merge face already, so there is nothing to ask: the face can only
become that one region, or stay a hole when there is none. The existing hole
test removes a whole loop, where every anchor on it dies, so it never reached
this shape.

All thirteen spans of the autosave contour now delete, none of them asking a
question, each leaving a scene that compiles.

Not chased, and untested either way: a face whose only surviving anchor is a
hole's, absorbing an active region whose anchor died on the removed span. It
would keep the hole rather than the region. Constructing it looks to need a
merge that cannot happen in one deletion, so it is noted rather than guessed at.

Verification: `cargo fmt --all`, `cargo clippy --workspace --all-targets
--locked -- -D warnings`, `cargo test --workspace --locked`, and `cargo build
--release -p funfern-app --locked`. Two new tests; the first fails without the
fix with the reported message, the second documents two holes merging, which
already worked.

## 2026-09-16 — Shift reaches probe placement too

The other half of the same omission. A probe already snapped to the grid when it
was dragged, with a test to say so, but placing one read no modifier at all, so
a probe could be dropped anywhere and would then jump onto the grid at the first
nudge.

Placement now follows the conventions dragging already set: a position goes onto
the grid, and a disk's radius is itself a multiple of it rather than the distance
to a snapped rim, which is what dragging a disk's edge handle does and what keeps
the radius a round number. A region is picked by the face under the pointer, so
the grid has nothing to say about it. The placement preview follows, ending where
the click will land.

The click body came out into `probe_placement_click` so it can be driven from a
test. `snap_point` and `snap_scalar` now share one `SNAP_STEP` rather than
declaring the same constant twice.

Caught while writing the test: inserting it directly above an existing one put
the new function between that test's `#[test]` and its `fn`, which silently
disarmed `shift_snaps_the_probe_rather_than_the_cursor` and double-registered
the new one. The listed test count is what showed it. Both are attributed
properly now.

Verification: `cargo fmt --all`, `cargo clippy --workspace --all-targets
--locked -- -D warnings`, `cargo test --workspace --locked`, and `cargo build
--release -p funfern-app --locked`. One new test over all four placements,
checked to fail without the snap.

## 2026-09-16 — The mean window stops where the trace does

The averaged flux row's window slider went further than any run could fill. Its
ceiling was the frame ring divided by the preset's nominal sample rate, and the
recorder does not sample at that rate: it strides the solver's own steps,
`round(1 / (rate * dt))` of them, so a coarse enough time step rounds the stride
down and oversamples.

Measured on a default scene, edge length 0.08, `dt` 6.69 ms:

| preset | stride | interval | 512 frames reach | slider offered |
| --- | --- | --- | --- | --- |
| Low 30 Hz | 5 | 33.5 ms | 17.10 s | 17.07 s |
| Medium 60 Hz | 2 | 13.4 ms | 6.84 s | 8.53 s |
| High 120 Hz | 1 | 6.69 ms | 3.42 s | 4.27 s |

At High the preset records every step - about 149 Hz, not 120 - and the top
quarter of the slider could never fill. Low lands just right by coincidence,
which is why it did not look systematic. The coverage rule adds one frame to
that: the newest frame needs a frame at or before its window start, so a window
equal to the whole span is marginal on float equality alone.

The ceiling now comes from the trace. The interval is the smallest gap in it -
a dropped readback inflates an average and would put the limit back out of
reach, while the smallest gap is still the stride, and a time step that changed
inside the ring gives the shorter of the two, which errs short - and the limit
is that interval over all but two of the ring's frames. Before anything is
recorded the nominal rate stands in, so the slider has a range from the start;
the interval is constant, so the ceiling settles on the second frame.

Verification: `cargo fmt --all`, `cargo clippy --workspace --all-targets
--locked -- -D warnings`, `cargo test --workspace --locked`, and `cargo build
--release -p funfern-app --locked`. One new test over the limit and what it
fills, including the dropped-frame case, checked to fail against the nominal
ceiling.

## 2026-09-16 — A condition keeps the name it was chosen under

The boundary picker listed "Driven Dirichlet" and then showed "Prescribed
Dirichlet" once it was chosen, because the list carried its own strings while
the selected text came from the condition's own `label()`. Neumann did the same,
and the outer domain had a third of its own: its second-order condition listed
as "Second-order outgoing" and read back as "Second-order auxiliary", which no
face condition ever called it.

"Prescribed" is what both condition enums already said and what the docs use, so
the list is what moved. The names now live once, on the picker's own
`BoundaryKind`, and a test holds the three sources to them in both directions:
every condition reads back as the kind that lists it, every kind is reachable
from some condition, and no two kinds share a name. Drift is a test failure
rather than something to notice in the interface.

The outer label is now "Second-order outgoing" like the face one. The docs keep
"second-order auxiliary" where they describe the auxiliary field the term
carries, which is about the mechanism rather than the control; the two README
lines that named it as something to pick follow the label.

Verification: `cargo fmt --all`, `cargo clippy --workspace --all-targets
--locked -- -D warnings`, `cargo test --workspace --locked`, and `cargo build
--release -p funfern-app --locked`. The new test was checked against the old
names and fails on the first of them.

## 2026-09-16 — Shift reaches the drawing tools

Shift snaps to a 0.05 grid everywhere something is dragged - controls,
endpoints, spans, the gizmo, probes, the outer domain - but the drawing tools
never read the modifier. `draw_click` was handed the raw pointer position, and
the branch that handles a live gesture returns before any of the code that looks
at modifiers, so the key did nothing at all while drawing.

The point is snapped in one place now, ahead of the tool branches, so every tool
gets it: a circle's centre, a rectangle's two corners, and each vertex of a
polyline, polygon or spline. An attachment still wins where there is one - the
point being welded to is where the curve has to land - and the grid applies
everywhere else. The rubber band follows the same rule, so it ends where the
click will land rather than at the cursor.

Two consequences worth having: closing a polygon is easier, since a click near
the first point snaps onto the same grid node, and a rectangle drawn with Shift
comes out on the grid.

Verification: `cargo fmt --all`, `cargo clippy --workspace --all-targets
--locked -- -D warnings`, `cargo test --workspace --locked`, and `cargo build
--release -p funfern-app --locked`. One new test, checked to fail without the
snap on the first placed point.

## 2026-09-16 — The draw palette stays where it is put

Picking a tool closed the Draw palette, so laying out several primitives meant a
trip back to the toolbar between each one. It stays up now, and closes only when
it is closed - from a title-bar button of its own or from the toolbar toggle. It
is also no longer anchored, so it can be dragged out of the way and stays there.

The palette is visible during a gesture as a result, which means picking another
tool mid-draw restarts with that tool and drops the points already placed. That
reads as switching tools rather than as a fault, so it is left as it is.

Verification: `cargo fmt --all`, `cargo clippy --workspace --all-targets
--locked -- -D warnings`, `cargo test --workspace --locked`, and `cargo build
--release -p funfern-app --locked`.

## 2026-09-16 — Steps per second belongs to the solver, not to a generation

The status bar's steps/s dropped to zero on every handover. The GPU's step
counter belongs to the generation that produced it - `WaveBindGroup` carries it
forward only while the generation matches, and a handover restarts it at zero,
which is correct, since it is the generation-local step index the simulation
clock pairs with `sim_time_offset`. The rate meter was differencing that counter
across its half-second window, so a commit landed as `saturating_sub` of a
smaller number by a larger one: zero progress, for up to half a second, every
time.

The rate now banks what the counter advanced between frames and averages the
bank, and a changed generation restarts the per-frame baseline at zero rather
than at the previous generation's total. Keying that on the generation rather
than on "the counter went backwards" keeps the first frame of a new generation
exact even when it passes the old total inside that frame. A real stall while a
candidate is prepared now reads as a dip proportional to the wait, which is what
it is; it used to be indistinguishable from the reset.

Verification: `cargo fmt --all`, `cargo clippy --workspace --all-targets
--locked -- -D warnings`, `cargo test --workspace --locked`, and `cargo build
--release -p funfern-app --locked`. One new test over the accounting, checked to
fail against the old expression on exactly the handover window.

## 2026-09-16 — The diagnostics keep what the channels said

Every channel that reports trouble overwrites itself: a status line by the next
message, a preparation or adaptation error by the next success, a repair
fallback by the next transaction. An error that clears itself a frame later was
unreadable - the window would flash open and the reason would already be gone.

The diagnostics window now carries a Log directly under the frame graph, above
the sections whose height follows the last transaction: a 200-entry ring, newest
first, with Clear and Copy, each line stamped with the session time and tagged
by the channel it came from. Errors read red, repair fallbacks gold, status
lines plain.

It is filled by watching the channels rather than by instrumenting the places
that write them. `self.message` alone is assigned from about two dozen sites,
several inside closures that borrow only that field, so a `notify` funnel would
have churned them all and still missed whatever it missed; comparing each
channel against the last value logged from it catches every path by
construction, and the channel supplies the severity. Identical lines arriving in
a row count as `×N` rather than filling the ring, which is what an error that
clears and returns every frame would otherwise do. The one exception is the
repair fallback, which is queued by the transaction that reported it: a fallback
is an event, not a state, and two transactions that fall back the same way
should read as two.

The status marker is now sticky - lit by an error, cleared when the diagnostics
are opened on it - and the window no longer opens itself. The marker was there
all along but did nothing, because the window had already popped up; on its own
it is enough, and it no longer takes the screen away mid-edit.

Verification: `cargo fmt --all`, `cargo clippy --workspace --all-targets
--locked -- -D warnings`, `cargo test --workspace --locked`, and `cargo build
--release -p funfern-app --locked`. Two new tests over the ring: one change per
channel change, a held value logged once, repeats counted, and the bound.

## 2026-09-16 — A reused mesh has to be renumbered, not just restamped

Switching a span that separates a hole from the domain to Transmit gave
"Invalid adaptation source: invalid topology trace vertex". The question that
came with it - which condition a transmitting span falls back to against an
excluded face - was already settled and is the one worth wanting: the plan
rewrites it to `SpanBehavior::REFLECTING`, a homogeneous Neumann wall, and the
authored span stays transmitting so reactivating the far face restores
transmission. First-order outgoing is not involved.

The error had nothing to do with the condition. The arrangement hands out one
trace id per sector a vertex has: a separated node carries a pair, one per face,
and a transmitting node carries one shared between them. Flipping the variant
therefore renumbers every trace from that curve onward even though nothing
moved. Against a hole nothing else moves either - the excluded side never
emitted an atom, and the live side reads `Separated` before and after, since
transmit is walled here - so the face, source and behaviour signatures all
match, `junctions` is empty, and `same_geometry` compares planned trace
vertices *by point, never by id*. The plans agree to reuse the mesh, and the
mesh is handed forward with only its `geometry_revision` restamped, still
carrying the previous numbering. Measured on a 0.2 hole at the default
resolution: 17 of the plan's 36 traces absent from the mesh and 17 of the
mesh's absent from the plan, plan trace 17 sitting where the mesh said 25.
Nothing notices until an adaptation checks each trace against
`contract.trace_points`.

Only reuse is affected. A carve rebuilds its vertices with no trace at all and
derives every one of them from the plan it was handed (`pair_traces`), so
repairs and rebuilds were already right - and they *heal* a mesh that came
through a bad reuse, which is why a later move made the symptom disappear.

`topology_trace_remap` builds the correspondence from the atoms rather than
from the vertex list, so a side is part of the match and two traces at one
point cannot swap: two plans that agree to reuse a mesh hold the same boundary
in the same place, and its ends are the same two topological points under
either numbering. It is validated as a bijection onto the new plan's own
vertices and returns `None` otherwise, which the runtime answers with
`FullRebuild(TraceIdentityChanged)`. The reused mesh is now prepared alongside
the action, because preparing it can fail.

The Edit panel also says so now. Beside Inactive - a span excluded on both
sides - a transmitting span excluded on just one side reads **Walled**, with
the reason on hover. Its test asserts both halves of the claim: the span
context the badge counts, and that the plan really does come out reflecting.

Verification: `cargo fmt --all`, `cargo clippy --workspace --all-targets
--locked -- -D warnings`, `cargo test --workspace --locked`, and `cargo build
--release -p funfern-app --locked`. Three new tests; the runtime one was
checked to fail with the remap stubbed out, on exactly the id sets above.

## 2026-09-16 — A slit is pulled out of the polygon, whichever way it transmits

Three things, one shape. A free separator repaired when it was added but rebuilt
about half the times it was *moved*, with two errors by turns: "cavity cycle has
no area" and "topology ear clipping stalled".

The cavity boundary that follows a transmitting chain out and back encloses
nothing. Its shoelace sum is zero in exact arithmetic and lands either side of
zero in floating point - measured at -1.4e-17 and +6.9e-18 on consecutive moves
of the same curve - and carving sorts its cavity cycles by that sign. A needle
that came out positive became a component of its own, and ear clipping has no
ear to take from a polygon with no interior. That is the coin flip the user saw.

The fix is the one the shape asks for: a slit walked twice by one face is left
out of the cavity polygon and recovered into the triangulation afterwards, which
is what carving already did for a baffle. Only the tail differs - a separated
slit is cut, so each side gets its own vertices; a transmitting one is not, both
sides being the same medium, so its recovered chain is only labelled. Moving a
free separator now carves every time, exactly like moving a baffle. The area
sign is also normalised where it is still read, so a flat cycle cannot be sorted
by rounding.

**Drawing a curve onto its own middle** worked in one direction and not the
other: the loose end's node index was read before the other end's attachment
split a span of that same curve, and every node past a split shifts by one. The
endpoint is carried now and the node located at join time. Both directions
produce the same single curve meeting itself at one vertex.

**A slit along the domain's centre line** could not be recovered at fine
resolutions. Refinement puts vertices exactly on a line of symmetry, and a
vertex standing in a constrained segment leaves no edge to flip - the way
through is a point, not a gap. The legacy interface recovery already split the
segment at such a vertex; the topology path now does the same. Horizontal,
vertical, and curved slits through the centre all mesh at 0.08 and 0.18, area
exactly 4.000000, no orphans.

Three tests, each verified to fail beforehand: six consecutive moves of a free
separator all carving, the self-drawing gesture in both directions, and the
centre-line slit at the application's own settings - the core's test options are
coarser and never reproduced it.

**The recovered chain needed its lineage.** Adapting a mesh whose transmitting
chain had come from a carve rather than a rebuild failed with "topology interval
endpoint lost its trace identity", intermittently: the adaptation checks every
constrained edge against the plan, and a vertex the recovery created carries
nothing of its own, so an interval endpoint held a trace only when it happened
to be a vertex some other chain had already made. Labelling now assigns each
interval endpoint its trace and each vertex between them the label and parameter
it sits at, which is what the cut does for a separated slit. A fourth test
carves a free separator and then adapts over it.

Checked: `cargo fmt --all`, `cargo clippy --workspace --all-targets --locked -D
warnings`, `cargo test --workspace --locked` (506 tests), and the release build.

## 2026-09-16 — One atom per direction, and both debts are paid

Two consequences were left behind by free separators: adding one forced a full
rebuild instead of a repair, and a boundary probe worked on only one of its two
sides. Both were the same line.

`append_boundary_sides` emitted the right-hand atom of a transmitting edge only
when the two sides named *different* faces, and `coarsened` repeated the rule.
For a chain interior to one face - a divider bridging two loops, or one that
ends in open space - that left a single atom where the face's boundary walks the
chain twice, once in each direction.

- Carving expands the face's steps whose atom is in the changed set. The return
  traversal was not in it, so nothing came back along the chain and the cavity
  rim ran into a dead end at the free tip, which is the error it reported.
- A boundary probe asks `contains_boundary_target` whether the plan holds an
  atom for the span *and side* it wants. The second side had none, hence
  "Boundary probe span has no active trace".

Both sides are emitted now, whatever faces they name. The pair carries the same
two trace vertices in the opposite order, so it introduces nothing new, and
every reader of that list asks whether some atom matches rather than summing
over it - the far field, the probes and carving's own change detection all go
through `any` or a set.

Adding a free separator now repairs: the carve keeps most of the mesh and the
field crosses over with the usual exactness. A probe compiles on both sides, 64
sample points each, with exactly opposite outward normals - which is what makes
the flux sign mean anything, and carrying a probe is the reason to draw a curve
that changes nothing in the first place.

Two tests, both verified to fail with the guards put back: the repair, and the
pair of probes. Checked: `cargo fmt --all`, `cargo clippy --workspace
--all-targets --locked -D warnings`, `cargo test --workspace --locked` (503
tests), and the release build.

## 2026-09-16 — A curve that divides nothing is a state worth holding

Four separate rules refused a separator that does not divide: the compiler's
`FreeTransmittingEnd`, the editor's "needs two attached endpoints" and "must
create exactly one new face", and the viewport's "Start a separator on an active
boundary". None of them defended a capability the engine lacks. Bypassing the
compiler's check and running a free transmitting curve end to end: it compiles
to one face, and it meshes at 0.08 and 0.18 with total area exactly 4.000000 and
no vertex left out of the triangulation.

Which makes the refusal a modelling opinion, and a costly one - the plainest way
to switch a wall off without deleting it is to set its spans transmitting, and
for a *free* baffle, the commonest kind, that made the document invalid. An
attached baffle could already be toggled: drawing a chord across a subdomain and
switching it to Transmit compiles, stays valid and meshes, merging the two faces
back into one.

- Removed the check and the `TopologyIssue::FreeTransmittingEnd` variant. Its
  two tests now assert the opposite: a transmitting curve alone in the domain
  leaves one face with itself on both sides of it, and a transmitting crossing
  with four free ends is atomized into a vertex without dividing anything.
- The editor draws a separator with any number of attachments. `assign_new_faces`
  accepts zero new faces - there is nothing to give a material to - and still
  refuses more than one. The excluded-face rule moved there too, so it fires
  when a face would actually be cut out of an excluded one rather than on every
  separator drawn in one.
- The viewport no longer requires a separator's first click to land anywhere in
  particular, and paints every boundary as a target for both purposes.
- The material chosen while drawing applies only if the separator encloses
  something as it is drawn. A face enclosed later, by attaching an end, inherits
  the region it was cut out of - which is what the attach path already did, and
  is the only version that does not require remembering a panel selection across
  arbitrarily many edits.

**Checked past a fresh mesh**, since a dangling transmitting chain is a shape
several paths had never seen:

- Carving could not follow one being introduced: the cavity rim walked into the
  chain's dead end and the repair fell back to a full rebuild. Fixed the same
  day, below.
- Adaptation over a free separator refines cleanly: 541 triangles to 2116, area
  exactly 4.000000, no orphans.
- A boundary probe compiled on one side of a free separator and reported
  "Boundary probe span has no active trace" on the other. Fixed the same day,
  below.

Checked: `cargo fmt --all`, `cargo clippy --workspace --all-targets --locked -D
warnings`, `cargo test --workspace --locked` (502 tests), and the release build.

## 2026-09-16 — A click names a boundary, not a side of one

Connecting two closed curves with a baffle was refused with "Open-curve
endpoints must attach to the same face". The editor was not at fault: on the
reported file, 9 of the 64 straight paths between the two curves draw fine
through `create_open_curve`, and that message never appears. The side the click
reported was.

An attachment carries a `CurveTraceSide`, and the face it resolves to is the
face on that side. The side comes from which pixel the pointer landed on.
Hovering all the way round both curves of the file at fixed offsets:

| pointer offset | curve 1 | curve 2 |
| --- | --- | --- |
| 6 px outside | 0/160 wrong | 0/160 wrong |
| 2 px outside | 24/160 report the interior | 40/160 |
| on the line | 57/160 | more |

Neither curve owns a vertex, so 88 and 136 of those 160 positions are
*breakpoint* hits rather than edge hits, and `hit_breakpoint` takes its side
from the chord between the node and the span's midpoint. On a rounded span that
chord cuts the corner, so a point genuinely outside the curve lies inside the
chord. With two closed curves, one wrong-side click names an interior and the
two ends disagree.

A better side test would only narrow the window. The side is not the user's
choice: they clicked a boundary, and which side of it the new curve lies on is
visible in the curve they drew. So:

- `open_curve_face` replaces the pairwise face comparison. Each attachment
  offers every face it could name - both sides of a curve anchor or breakpoint,
  every assigned sector of a junction - and the face is taken from the drawn
  path, sampled at half, a quarter and three quarters of its length. If the path
  cannot say, the sides the clicks reported are tried, and then the only face
  the two ends share. The error survives only for ends that genuinely touch no
  common subdomain.
- `attach_end_to_vertex` does the same for a weld, with the curve already in the
  document standing in for the drawn one. A two-sided target there picked the
  source region for `assign_new_faces`, so the wrong side handed a new face the
  wrong material and frame.
- The viewport no longer restricts a separator's snapping to the face its first
  click reported, and a separator's first point is accepted when *either* side
  of the boundary has an active subdomain. `DrawGesture::face` and the UI's own
  `attachment_face` are gone with it.
- `materialize_attachment` already ignored the side entirely, which is what made
  this contained: the side existed only to answer the face question.

Two regression tests, both verified to fail beforehand. A baffle bridging two
loops is drawn for all four combinations of reported sides. A baffle chord drawn
through a subdomain splits it whichever side the clicks named, and the daughter
region inherits the subdomain's material - on the old code, a pair of
outside-reported clicks handed it the background's instead, silently.

Checked: `cargo fmt --all`, `cargo clippy --workspace --all-targets --locked -D
warnings`, `cargo test --workspace --locked` (499 tests), and the release build.

## 2026-09-16 — A slit that joins two loops breaks the face it cuts

Two reports, one defect. A file whose geometry could not be meshed, and a
separate one a day later: two subdomain loops with a baffle drawn between them,
which the mesher also refused. Both came back as `Refinement limit reached
(minimum angle 0.0°)` - a degenerate triangle nothing can repair.

**What was wrong.** A separated span whose two sides face the same face is a
slit: `prepare_topology_builder` leaves it out of the face's polygon,
triangulates without it, and cuts it back in afterwards. That is sound only when
removing the slit leaves one closed loop. A slit that *bridges* does not: the
face's cycle runs around one loop, along the slit, around the other, and back
along the slit, so the two holes are one cycle. Dropping the slit steps and
keeping the remainder as one polygon concatenated the two loop boundaries, with
a zero-width jump across the gap at each end. That jump is the 0° triangle.

Measured on the shapes, before the fix:

| shape | before |
| --- | --- |
| baffle hanging off one loop, free tip | meshes |
| baffle chord across one loop (splits the face) | meshes |
| baffle T onto another baffle | meshes |
| baffle bridging two loops | refinement limit, 0° |
| baffle bridging a loop to the outer wall | refinement point outside the domain |
| a curve meeting its own interior (a loop on a stem) | fails |
| the same bridge switched to Transmit | meshes |

The transmitting version always worked, which is what localized it: a
transmitting edge is interior to one face and never goes through the slit path.

**The fix**, all in `mesh/topology_plan.rs`:

- A face cycle is now split at slit removals into closed stretches, one polygon
  each, starting just after the first removal so a stretch spanning the cycle's
  own start stays in one piece. Each stretch has to come back to the point it
  started from, which is an assertion now rather than an assumption.
- Outer versus hole can no longer come from cycle order: a slit joining a hole
  to the outer boundary splits the face's *first* cycle into one of each. They
  are classified by orientation instead - the plan's cycles run with the face on
  the left, so the outer boundary turns counter-clockwise. Exactly one per face,
  checked.
- A stretch that leaves a junction through one sector and returns through
  another closes on a different trace vertex at the same point. That vertex is
  seeded to the departing one before the chain is expanded, so the boundary
  edges and the polygon agree; otherwise the arriving one belongs to no triangle
  and the slit leaving that junction is cut from a vertex the mesh does not
  have. `split_slit_trace_vertices` separates the sectors again after the cut,
  which is what it is for. This is the self-touching case.

Both reported documents mesh now: the bridged pair at 3024 triangles and the
earlier loop-on-a-stem at 2932, with no vertex left out of the triangulation in
either. A first attempt resolved the missing vertex inside `cut_free_slit`
instead; seeding it upstream made that unnecessary and was removed.

**Follow-up the same day.** Classifying every positively-oriented run as an
outer cycle is too strict: a cycle that walks a transmitting chain out and back
encloses no area at all, and whether its shoelace sum lands just above or just
below zero is rounding. The largest signed area is the outer boundary instead.
Nothing legal reaches that today - a dangling transmitting chain needs a free
transmitting end, which the compiler refuses - but it is what a free separator
would produce, and the old order-based rule had handled it by accident.

**Also measured, not fixed.** A baffle lying exactly on y = 0, the domain's own
centre line, still fails to mesh at fine resolutions with `could not recover an
internal-boundary segment`. Any offset works - y = 0.05, 0.1234, -0.37 all mesh
- so it is an exact-symmetry degeneracy in constrained recovery, not the same
defect. Worth its own look; reachable, since the grid snaps there.

Fixed later the same day, below.

Checked: `cargo fmt --all`, `cargo clippy --workspace --all-targets --locked -D
warnings`, `cargo test --workspace --locked` (497 tests), and the release build.
Three new mesher tests - the two-loop bridge, the bridge to the outer wall, and
the self-touching curve - each verified to fail on the previous code and to pass
on this one, and each asserting total area and that no vertex is orphaned.

## 2026-09-16 — What a line probe actually carries

A line or boundary probe reported the instantaneous normal flux and its arclength
integral. Neither answers the question the probe is usually placed to ask: a
standing wave's flux swings symmetrically about zero at twice the driven
frequency, so a snapshot of it says nothing about transport, and the integral of
that snapshot is just as ambiguous. The far field has had an averaged pattern
since it landed; the path probes had no equivalent.

- Added a fifth row to the plot matrix, `LineProbeQuantity::MeanFlux`, drawn from
  a causal trailing mean of the recorded flux: every frame carries the average of
  the window ending at its own time, sample point by sample point. All three
  existing representations then apply unchanged - the profile against arclength,
  the waterfall against arclength and time, and the arclength integral against
  time, which is the net power the path carries.
- The averaging window is a slider, not the visible window. The far field takes
  its average from what is on screen, which is right for a single polar snapshot
  and wrong for a time series: zooming would rewrite the numbers instead of
  moving over them. It defaults to 1.0 s - two and a half periods of the default
  2.5 Hz source, five of the flux - and is bounded by what the 512-frame trace
  actually holds at that probe's preset, 17/8.5/4.3 s at Low/Medium/High.
  Offering more would be a setting that shows nothing however long the run goes.
- A frame whose window the record does not cover in full yields a row of NaN
  rather than a partial average. That keeps the derived series one row per
  recorded frame, so the waterfall's row layout and the profile's nearest-frame
  pick stay aligned with the raw series, and the renderers already skip what is
  not finite. The readout reports the fill fraction until it is complete, in the
  idiom the far-field delay window already uses.
- Gaps are skipped per point, not per frame: a path that leaves the domain
  halfway still averages the half that is inside. A width change in the trace -
  a sampling-preset change, or a boundary remesh - restarts the window, since two
  sample layouts have no common average.
- One pass over the trace serves all three averaged views and runs only when one
  of them is drawn. It is a sliding window with per-point sums and counts, so the
  cost is one add and one subtract per sample, not a window-sized sum per frame.
- Two fixes found on the way. The arclength-integral trace now drops non-finite
  entries instead of pushing them at the plotter, which mapped them to NaN screen
  positions; and an arclength profile whose samples are all invalid now says so
  instead of leaving an empty box.
- The row is called Average flux, not the bracketed notation it started with:
  egui's default font has no glyph for angle brackets and drew them as tofu.
- Nothing moved on the GPU, in the core, or in the file format: the recorder has
  always written `normal_flux` per point per frame, and view state is per-session.

Checked: `cargo fmt --all`, `cargo clippy --workspace --all-targets --locked -D
warnings`, `cargo test --workspace --locked`, and the release build of
`funfern-app`. Five new tests cover the withheld window and its fill fraction,
the cancellation of a standing wave against a travelling one's surviving offset,
per-point gaps, the layout-change restart, and that the plot matrix addresses
every cell exactly once.

## 2026-09-16 — Two ends of the spectrum nothing was taking care of

Two reports, one root: this scheme neither transports nor dissipates its own
extremes, and an enclosure keeps whatever lands in them.

**The DC runaway is not a regression.** The difference form from 2026-09-15 is
holding: a sealed reflecting cavity carrying a displacement bump with zero mean
velocity kept its mean at `1.0053e-2` unchanged over 32 s, and a reflecting loop
present from the first step, driven from outside, sat at `3.9e-10` after 24 s.
What the report hit is the residual that entry flagged and left open. A sealed
Neumann region conserves its mean velocity exactly, so any trapped mean velocity
integrates without bound and does it perfectly linearly: seeding `v0 = 1.0053e-2`
gave a mean matching `v0 * t` to five digits at 8, 16 and 32 s. Raising a wall
over a live wave traps whatever is passing — measured at a constant `-0.0105` per
second afterwards — and no source change can prevent that.

Two things inject the velocity. Sealing over a live field is one, and it is the
model being right: with `Reflecting` as zero normal gradient, a free-edge region
that can translate does translate. Nothing here drains it, because a drain would
pick one physical reading over another and need a corner frequency that would be
wrong for somebody's cavity. The second was ours. Switching a sinusoid on at
`t = 0` leaves the field a mean velocity of `amplitude * cos(phase) / omega` —
what the forcing's running integral keeps — and the default source shipped at
phase zero. In an open domain it drains through the boundary; measured with the
same source, a reflecting outer wall ramped `5.18e-2, 1.04e-1, 1.55e-1, 2.07e-1`
at 8/16/24/32 s while second-order outgoing sat at `3.0e-3` and decayed. The
default is now a cosine start, which looks the same and carries no impulse. A
phase the user picks does carry one; that is theirs.

**The speckle is the other end, and it is now filtered.** Nothing dissipates at
any wavelength: a seeded grid-scale field held its energy to five digits over
32 s, and the rough part of a released step stayed in the radius bin holding the
old wall from 8 s to 32 s without moving or fading. Releasing a step leaves a
residue at 0.44 of the operator's spectrum; remeshing was ruled out as a source,
since a full remesh of a smooth field with 3 of 32363 nodes landing exactly took
roughness *down* and 25 rounds left it unchanged.

`QuadraticWaveOperator::apply_grid_scale_filter` and the `filter_stage` /
`filter_apply` pair in `wave.wgsl` remove it. `L = (K/M) / lambda_max` has
eigenvalues in `[0, 1]`, is exactly zero on a constant field, and scales as the
square of a mode's frequency, so applying it twice separates the physical band
from the mesh ceiling by the fourth power of their frequency ratio. Only the
difference between the two levels is damped, symmetrically, so the midpoint never
moves and a sealed region's standing offset is untouched; prescribed nodes are
skipped. It runs every 16 steps, which is two extra gathers for about a tenth of
the solver's work, and the toggle leaves the dispatches out rather than zeroing
anything.

Two choices were settled by measurement rather than argument. Normalising per row
instead of by one global bound is locally the more meaningful thing to do, and it
does clean a 7:1 graded mesh uniformly where the global bound is 53x weaker on
the coarse half — but at matched residue removal it costs the resolved band four
times as much (`0.978` against `0.995` at ten nodes per wavelength), because the
residue concentrates on exactly the rows the global bound weights hardest. Global
it is. A fourth power instead of a square was no better at matched cleanup and
slightly worse, because the residue is not a thin spike at the ceiling. Strength
`1.5` against a hard limit of `2.0`, where a mode at the ceiling would stop
decaying; `3.2` was still stable in practice and `8.0` was not.

Measured by the test that pins it, at `h = 0.12` over 14 s: the residue comes
back at `1.65e-2` against `7.13e-2` unfiltered, and a mode at about thirteen
nodes per wavelength keeps 99.95% of its energy. At `h = 0.05` over 32 s the cost
is 0.15% at sixteen nodes per wavelength and 1.0% at the ten the mesh indicator
asks for, rising steeply below that — which is the honest shape of a grid-scale
filter, and the reason the knee and the indicator's target have to stay in step.

What it does not do, learned from running it: it clears what a sharp event
leaves behind, not what a source keeps making. On the default scene with the mesh
pinned and a width-0.06 source on `h = 0.18`, filtered and unfiltered runs sat at
the same roughness to within half a percent, because a source that badly under
resolved re-injects as fast as this removes. That is the mesh indicator's job,
and AMR does it. The dispatch itself was confirmed wired by counting it in the
render node: both passes over 67 workgroups every sixteenth step with the toggle
on, cleanly absent with it off.

Also: both handoff shaders were still evaluating the stiffness as a plain row
product, so the previous entry's "on the CPU and in the shader" was half true.
Worth about `5e-8` of a DC offset per handoff — invisible, but it sets the
velocity the next generation starts from, and a reader was entitled to believe
the claim. Both are differences now, with a test on each.

`filter` is a reserved WGSL keyword; the throwaway naga validator caught it
before the GPU did, as it did `target` last time.

- Verification: formatting, Clippy across all targets with warnings denied, all
  **489 workspace tests**, and the native release build. The solver setting is
  session state, matching `amr_enabled` and `mesh_edge`, so nothing is persisted
  and no schema version moves.

## 2026-09-15 — The solver clock is carried, so nothing may add to it

Reported: the probe traces gained holes at every remesh, and the far field still
never appeared under adaptation. One bug behind both, and it was mine to find in
the shader I had not read closely enough.

`wave_transfer_new.wgsl:170` writes the old generation's `time_data.z` into the
buffers that replace it - only the step counter restarts. Every recorder stamps
that clock, so its samples are already on the app's timeline, which is what
`probe_shader_uses_the_continuous_transferred_solver_clock` has been saying all
along. `ingest_probes` added `sim_time_offset` on top of it, counting the run so
far a second time. The gap at a handoff is therefore the whole lifetime of the
mesh that just ended, not anything the handoff costs: measured on the autosave,
a trace running to 0.746 s resumed at 1.832 s, and the next handoff moved it on
by another 0.724 s.

The far field had the same fault, introduced by me: I passed `sim_time_offset`
as the ring's clock origin, so every handoff jumped the ring about a second
forward and left 54 empty buckets inside a 205-bucket window - then 97, then 111.
A single hole invalidates every direction that reaches through it, permanently,
which is why nothing was ever plotted. The ring itself was being kept at all 26
handoffs of the run. The origin is gone; `state_time` is the solver's clock, the
same expression probe.wgsl uses.

Two more things the measurement turned up:

- `newer.w != newer.w` never fired. These shaders compile under fast math, where
  a NaN self-comparison is folded to `false`, so every unwritten frame read as
  data and the rings came back flagged valid with NaN amplitudes - only the
  readback's `is_finite` filter was rejecting them. Both rings carry a finite
  `-1.0e30` sentinel now and the guard is a comparison against a real number.
- `sim_time_offset` was computed from the wave readback's step count, one or two
  frames behind what had been encoded - 1166 against 1176 - losing about 8 ms of
  clock per handoff. It comes from the solver's own tally now, read before the
  upload resets it. It is still what turns a step count into a time; it is no
  longer added to a time.

`install` no longer drops the point, curve, and area rings either, for the same
reason it stopped dropping the far field's: the readback already in flight still
lands, and the app keeps the previous upload addressable so those last samples
are ingested rather than filtered out as stale. Worst hole in a 30 s run with 25
adaptations: 0.71 s before, 33 ms after, most under 20 ms.

Measured with the app itself rather than reasoned about - the autosave loaded, a
point probe injected at startup, the contour ring temporarily read back and its
bucket coverage printed. That is what showed the ring was being kept while the
window had holes in it, which no amount of reading the diff was going to.

Left open, as a low-priority item in the TODOs above: the samples between the
last readback issued and the freeze still go with the ring when the commit
rebuilds it, 2 to 4 of them at 120 Hz.

Checked: `cargo fmt --all`, `cargo clippy --workspace --all-targets --locked -D
warnings`, `cargo test --workspace --locked`, `cargo build --release -p
funfern-app --locked`, the shader validated with naga 29, and a 30 s run of the
app against the autosave with adaptation on.

## 2026-09-15 — The far-field recording outlives the mesh it was read through

Reported as no handoff procedure: the plot restarts on every remesh, and with
adaptation on it never appears at all. It did not appear because a projection
cannot report anything until the ring holds a whole delay window - every
direction reads all 256 contour points at their own retarded times, and one NaN
or out-of-range age invalidates the whole direction - and `update_far_field`
opened by clearing the ring, on every commit. For the current autosave that
window is 3.4 s of simulated time (a 1.79 s margin to the domain's corner plus
1.63 s of contour reach at c = 1), while adaptation re-evaluates every 0.75 s
and adapts as soon as four elements want refining. The ring never survived long
enough to fill once.

Nothing about the recording is mesh-bound. `rectangular_far_field_contour` takes
the positions, normals, spacing, speed, and margin from the document; only the
seven-node stencil per point reads the mesh. So the ring is now keyed to what it
describes - `FarFieldContour` - and a new mesh over the same contour swaps the
stencils and keeps recording. `install` no longer clears it: what it holds is the
exterior at fixed world points, and `update_far_field` decides. Nothing samples
into it while a handoff is in flight, because a recorder pass is only ever
encoded inside the step loop and no step is encoded until the commit.

Two things stood in the way of keeping it, and both were the ring being counted
in steps. The cursor was `(completed / stride) % frames` over a step counter that
restarts with each GPU generation, and `sample_interval = stride * dt` changed
whenever an adaptation moved the time step. A frame is a bucket of the app's
clock now - `floor((origin + t) / period)` with the period leaving 1/60 s only
for a step longer than that - so the cursor is a function of time and continuous
across a swap by construction, and the projection brackets by the times recorded
in the ring rather than by a uniform age, so buckets filled at one step size
still read correctly under another.

The dispatch gate cannot follow the solver's clock: `time_data.z` accumulates a
step at a time in f32 and runs away from any arithmetic over step counts by more
than a whole step within a few thousand steps, and a missed dispatch is a hole
that invalidates every projection reaching back through it for a whole lap. So
the shader asks the ring instead - a frame already holding a sample of this
bucket is done - and the stride only has to be dense enough to visit every
bucket. A quarter of a bucket keeps the recorded sample near its start.

`a_new_mesh_over_the_same_contour_inherits_the_far_field_ring` runs the real
`update_far_field` over a `World`: an adaptation at a shorter step keeps the ring
and replaces the stencils, a restarted clock and a moved contour do not, and
nothing is left in `Assets` afterwards. The ring's own rule is run at a clock
that drifts and then changes step, and held to one sample per bucket with no
gaps, by `the_far_field_ring_records_every_bucket_once_across_a_handoff`.

The readout and the Probes panel now say how much of the delay window is
recorded, which is the difference between a plot that is empty and a recorder
that is not ready.

One limit left where it was: the wave clock is f32, so a long enough run loses
the resolution to separate buckets. It is the same clock the solver and every
other probe already run on.

Checked: `cargo fmt --all`, `cargo clippy --workspace --all-targets --locked -D
warnings`, `cargo test --workspace --locked`, `cargo build --release -p
funfern-app --locked`. The shaders were also parsed and validated with naga 29
outside the workspace, which is what caught `target` being a reserved word in
WGSL before it reached a GPU.

## 2026-09-15 — Probes belong to the wave buffers, so they follow them back

Reported as probes sticking after a Reset, with Clear as the only way out. Two
faults, stacked.

Every probe buffer lives and dies with the wave buffers: `install` opens by
clearing the point, curve, area, and far-field buffers and despawning their
readbacks. The only thing that ever recreated them was `configure_probes`, and
it was reachable only from the two commit paths. A reset changes no document
revision, so no candidate is prepared and no commit arrives - the probes were
left with no buffers and no way back, and a rolled-back transfer was the same.
`probe_gpu_token` was written at both commits and never read: the staleness
comparison it was meant to drive was lost in the unified-topology swap, where
the old engine had `probe_compiled` keyed to the GPU generation and revisions.

It is `probe_upload` now - topology token, GPU generation, and the four probe
revisions - and one check per frame re-runs the upload whenever the token or the
generation moves. That covers the commits it replaces, the reset, and the
rollback, by construction rather than by remembering to call it. It waits while
an upload is in flight, like the pulse below it, because mid-upload the buffers
are the candidate's while `runtime.active()` still names the old mesh.

The second fault outlives the buffers. Reset puts `sim_time_offset` back to zero
while the traces keep their high-water mark, and ingestion only takes a record
newer than the trace's last, so every sample of the new run was dropped - until
Clear removed the trace and its mark with it. The clock restarting is what makes
the old samples foreign, so `restart_probe_traces` now runs where it restarts:
the reset path, and a `fresh` commit, which is how loading a file or an example
arrives. Ingestion also checks the generation and revision a readback was
recorded against, so a readback still in flight from the run that ended cannot
land in the run that started.

Both guards were confirmed to bite by disabling each in turn against
`a_restarted_run_records_from_its_own_clock`.

Checked: `cargo fmt --all`, `cargo clippy --workspace --all-targets --locked -D
warnings`, `cargo test --workspace --locked`, `cargo build --release -p
funfern-app --locked`.

## 2026-09-15 — The adaptation target is an overlay, so it is finally visible

Reported invisible, and it was: `draw_solution` painted the target wash first,
then the Materials fill, then the field - and with the Overlay list on `Off` the
field is fully opaque, since `field_color` lerps from an opaque base and only
`field_color_over_overlay` carries alpha. So the wash sat under an opaque field,
or under the default Materials fill, whichever the scene had. It was only ever
visible with the field switched off.

It is a `MaterialOverlay` choice now rather than its own checkbox, which is where
it belonged: that slot is what fills the domain under a field the same slot turns
translucent. Being one of the overlays also makes it exclusive with them, which
it always was in practice. It draws as a single `egui::Mesh` like the categorical
fill next to it - per-triangle polygons carry their own antialiased outlines and
imprint the mesh on the wash - and takes its alpha from the Overlay intensity
slider instead of a hardcoded 75.

The overlay had three ways of being empty and said nothing about any of them:
adaptation off, no estimate yet, or an estimate whose element count no longer
matches the mesh after an adaptation. The View panel now names whichever applies,
and shows the target range when the overlay is live.

`PresentationSettings.adaptation_target` is gone. The stored key is still read,
so a scene that had the flag on and no material overlay arrives with the new
overlay selected - an overlay that was actually visible wins - and is still
written as `false` so a build from before this change can load a scene saved
after it. Covered by `presentation_round_trips_and_older_scenes_receive_defaults`.

Checked: `cargo fmt --all`, `cargo clippy --workspace --all-targets --locked -D
warnings`, `cargo test --workspace --locked`, `cargo build --release -p
funfern-app --locked`.

## 2026-09-15 — The mode arms are buttons, not captions

`Place pulse` was a bare `selectable_label`, which egui draws as plain text until
it is switched on, so it read as a caption over the pulse sliders rather than the
thing that arms pulse placing. It is a framed `Button` with `.selected()` now, the
same shape the far-field window's `Live` uses, and it says on hover what arming
does. The four probe placements - `+ Point`, `+ Line`, `+ Disk`, `+ Region` -
were the same widget and toggle against the same pair of mode flags, so they
moved with it and each gained the hint for how many clicks its shape takes.

Nothing behind the widgets changed: the modes still clear each other, the
crosshair cursor still marks an armed mode, and Escape still cancels through
`cancel_interaction`. No test - this is the widget, not the rule behind it.

Checked: `cargo fmt --all`, `cargo clippy --workspace --all-targets --locked -D
warnings`, `cargo test --workspace --locked`, `cargo build --release -p
funfern-app --locked`.

## 2026-09-15 — Transmit and Boundary report a state instead of offering an action

The pair of buttons showed nothing about where a selection stood: each was
disabled once it had been applied, which is only a hint, and the test behind the
Boundary one was `behavior == REFLECTING` exactly, so a baffle with a Dirichlet
side or thin-gap coupling left both buttons enabled and looked like neither
state.

They are tick boxes now, side by side. Transmit is ticked when every selected
span transmits, Boundary when every one of them is separated, whatever its faces
carry; a selection holding both ticks neither and says `Mixed` beside them.
Ticking applies, unticking does nothing, since there is no third state to fall
into. The one thing lost is the old Boundary button's side effect of resetting
custom conditions back to reflecting; the condition editor below does that a side
at a time, and a widget that reports a state should not also be a reset.

`span_behavior_state` holds the rule, away from egui, and
`a_span_selection_reports_one_state_only_when_every_span_agrees` covers the three
answers including the impedance case the buttons could not express.

Checked: `cargo fmt --all`, `cargo clippy --workspace --all-targets --locked -D
warnings`, `cargo test --workspace --locked`, `cargo build --release -p
funfern-app --locked`.

## 2026-09-15 — A span's selected side is visible again, and on the right side

Reported: the Left/Right selector for baffle faces is hard to use because the
scene never shows which side is selected. Two things were wrong, both from the
unified-topology swap.

The white band the pre-swap UI painted along the selected side of every selected
baffle span, and the gold start-to-end arrow beside it, were dropped with the old
renderer. `README.md` still described the arrow. Both are back, drawn for every
selected curve span on `selected_side`, sitting clear of the condition strokes
when that overlay is on.

Worse, the condition strokes themselves were mirrored. `draw_sampled` offset by
`(-t.y, t.x)` in screen space and painted the left law there, but the viewport
flips the vertical axis, so that direction is world-space right. `left_probe`
defines `edge.left` as the world-space left of increasing parameter, `hit_edge`
classifies a click as Left by the negative screen cross product, and the pre-swap
renderer used `(t.y, -t.x)`; the overlay disagreed with all three, so View >
Boundary conditions has been showing each law on the wrong side of the curve.

The convention now lives in two named places that a test holds together:
`screen_side` in `topology_viewport.rs` (extracted from the cross product inside
`hit_edge`, unchanged) and `side_offset` in `ui.rs`, which every band and stroke
offsets by. `the_side_band_falls_where_a_click_reads_the_same_side` offsets a
point to each side of several tangents and asserts a click there reads back the
same side; restoring the old normal makes it fail.

Checked: `cargo fmt --all`, `cargo clippy --workspace --all-targets --locked -D
warnings`, `cargo test --workspace --locked`, `cargo build --release -p
funfern-app --locked`. The band, arrow and stroke sides are geometry the tests
pin, but their look on screen is for the next interactive pass.

## 2026-09-15 — The formula reference is a window again, pinned to the parser

The unified-topology swap kept the material panel's `?` but lost what it opened.
The menu popup added with the reference became a `small_button` carrying a
one-line hover tooltip, and the tooltip had drifted from the language: it
advertised `ln` and `pow`, which the parser has never accepted, and omitted
`log`, `^`, `smoothstep`, `pi`, and `e`. The harness test that had guarded the
popup went out with the old UI harness, so nothing noticed.

`?` now toggles a `Formula syntax` window — deliberately a window and not a
menu, because a menu closes the moment the pointer enters the formula field,
which is when the reference is wanted. It lists coordinates and constants,
operators, and every function with its argument order, and it sits beside the
material Library and beside an enabled volume source's profile.

The reference is data: `FORMULA_SYMBOLS` and `FORMULA_FUNCTIONS` in `ui.rs`,
each function entry carrying an expression. `the_formula_reference_names_what_the_parser_accepts`
parses and evaluates every one of them through `ScalarField::formula` with no
parameters — an unknown name parses as a parameter and only fails on
evaluation, so evaluating is what proves a listed symbol is built in — and
asserts `ln(1 + r)` and `pow(r, 2)` are still rejected. The reference cannot
drift from the parser again without a red test.

Checked: `cargo fmt --all`, `cargo clippy --workspace --all-targets --locked -D
warnings`, `cargo test --workspace --locked`, `cargo build --release -p
funfern-app --locked`.

## 2026-09-15 — Requests are a ceiling, the existing density the floor

Two reports against the requested-size refill. A repaired band came out about
twice as fine as adaptation had made the same place, and wiggling a two-sided
subdomain boundary a few times with adaptation on produced super-fine elements,
`Invalid solution-indicator mesh`, a dt near 1e-8 and a frozen app.

Both reproduced from the autosave: one closed seven-span curve between two
materials, a 2.5 Hz source beside it, target 0.08, adaptation 0.02–0.16 with
the UI's options (512 changes per pass, collapse 0.65, 12°). A headless loop
runs the indicator on a synthetic wave from the source, one adaptation pass,
then a 3 mm move of one control. The first repair inserted 7,501 triangles for
599 removed, 139 of them under one degree, dt 2.3e-8, and every indicator pass
after that rejected the mesh. The same cavity with the size field off refilled
with 481 triangles at 18°; with no adaptation at all, 528. It also reproduces
in core with no adaptation: a fresh 0.08 mesh, a constant 0.04 request stamped
on every triangle, one wiggle — 3,692 inserted, 115 degenerate.

The ×2 is the indicator's semantics. Its target is
`(current edge × scale).clamp(min, max)` with the scale in `[0.6, 2.2]`: a
step from the current size that adaptation takes once and re-measures.
Stamping the step's endpoint as an absolute request and refilling a band to it
in one go lands at `0.6×`, and each wiggle re-applies the step to the already
refilled band.

The degenerate elements are born in the cavity's initial triangulation, across
nearly collinear frozen rim vertices, exactly as the fresh mesher's corner fans
are; a builder tripwire showed them pushed from `step_clip` before any
refinement. The fresh mesher splits such fans away through its boundary edges.
Rim edges can be neither split nor flipped — a flip needs a strictly convex
quadrilateral — and a sliver's circumcenter lands outside the cavity, so
refinement drops it and accepts the element. With no pressure to refine finer
than the rim, legalization alone cleans the initial triangulation up; with a
request finer than the rim, thousands of insertions leave the slivers in
place. Only 7 of the 139 shared an edge with a kept triangle, so the rim
hypothesis alone would have been wrong: it is the pressure, not the adjacency.

The rule: per removed triangle the refill target is
`min(meshing target, max(request, the edge length the triangle had))`. Requests
can only coarsen a refill, never refine it below what was there, so nothing
compounds, and the band matches what adaptation realized rather than what it
asked for. Refill triangles still inherit the request itself. Measured: the
core reproduction goes to 652 inserted, 0 degenerate, 18.3°; the autosave loop
runs three adaptation passes and three wiggles with 0 degenerate elements, dt
steady at 1.2e-3, and repairs inserting about what they remove.

Two guards beside it. A carve verifies its refill and fails with
`MeshError::DegenerateRepair` — the count, the worst angle, its place and the
floor, half the mesher's minimum angle or half the repaired mesh's worst angle
— so the runtime's fallback line names the defect and a recurrence is a bug
report, not noise; the floor follows the mesh so a legitimately small input
angle does not trip it. And the mesher's capacity caps now bound what a repair
adds rather than the whole mesh: with converged adaptation the mesh was 23k
triangles and the repair failed with `Mesh capacity reached at 12000 vertices`,
falling back to a rebuild that lost the adaptation.

Tests: the inclusion curve with constant and stepped requests keeps the fresh
mesh's worst angle within a degree and inserts at most twice what it removes;
the verifier is refused by name on a hand-built degenerate refill and passes a
mesh whose poor angle it inherited; a mesh beyond the caps repairs. The browser
check gains the wiggle with adaptation on.

The refill's quality beside the rim is still "accept what cannot be fixed";
the rule removes the pressure that made that catastrophic, and the verifier
catches a recurrence. Guaranteeing quality there means letting the refill
legalize or split into the first ring of kept triangles; recorded as a TODO.

## 2026-09-15 — Repairs refill at requested sizes, not measured ones

Dragging one curve back and forth over the same ground refined the mesh
without bound. Reproduced in core: a 5 cm nudge of a hole repeated fourteen
times at target 0.06 took the mesh from 4,746 to 10,343 triangles and the
shortest edge from 0.0133 to 0.0007, the removed band growing every pass
(235 → 2,428 triangles). The size field the carve built over the removed
triangles recorded their measured longest edges, and the refill took the
minimum of that and the target. Two sources of smallness are always present
and neither is adaptation: a fresh mesh already has elements about 4.5× finer
than the target beside a curved boundary, from the chord subdivision, and the
refill has to meet frozen rim vertices at arbitrary distances, which
encroachment splits answer with smaller elements still. Both were recorded,
copied into what had become interior, recorded again, and min'ed with the next
pass's slivers — a monotone min with a fresh supply of small elements every
time. Switching the field off held the mesh at the fresh count and shortest
edge for all fourteen passes; widening the band to one target length with the
field on made it three times worse (31,630 triangles), so the band width was
never the lever.

The fix is one mechanism, as agreed: every `TriMesh` carries `requested_sizes`,
per triangle the edge length adaptation last asked for where it lies, stamped
by the adaptation job at compaction from `triangle_target` on the finished
mesh and empty for meshes no adaptation has touched. The carve's
`LocalSizeField` is built from those requests only; measured sizes are gone
from it. Kept triangles keep their request, the refill's triangles inherit the
request of the removed triangle under their centroid, and ground a moved hole
uncovers has none and refills at the target. A request is copied, never
re-measured, so it is bounded by adaptation's minimum and nothing compounds.
The carve takes `preserve_adaptation`; the runtime's `set_preserve_adaptation`
receives the UI's adaptation switch before every request, so with adaptation
off the band returns to the target regardless of stale requests, and with
adaptation on but never run the result is the same because there is nothing to
copy. Coarsening in `amr.rs` orders collapses by lineage but does not gate them
on it, so a refilled band is ordinary mesh to the next pass: with a uniform
coarse field, a repaired adapted mesh went 3,421 → 1,671 triangles in three
generations.

Tests: the fourteen-pass drag holds within 5% of the fresh count and 95% of its
shortest edge; an adapted mesh dragged twelve passes stays within ±10% of its
count with over 95% of triangles carrying a request; kept triangles keep their
request and refill triangles over removed ground inherit theirs while the
uncovered crescent carries none; without preserving adaptation the refill is
coarser and carries no request; adaptation coarsens a repaired band it no longer
wants fine; the adaptation job stamps every triangle with exactly the field's
minimum over its seven samples; and the runtime test flips the switch between
two repairs. A browser check for the back-and-forth drag is listed, not run.

## 2026-09-15 — Carving documented

The architecture notes describe carving next to the full-rebuild baseline, the
atom identity rule, the junction and separated-curve growth rules, the frozen
import and the size field, plus the paired subdivision rule the flip test
forced into the mesher. The README's resolution bullet now says which edits
repair and which still rebuild, in the user's terms. The plan's statements that
coordinate edits take a full rebuild until local repair arrives are updated to
say it arrived. The browser checks gain a mesh-repair item covering a nudge, a
long drag, a baffle end, a junction, adding and deleting a curve, a material
change without carving, a repair through an adapted mesh, and the two edits
that still rebuild; they are listed, not yet run. The TODO item is closed and
its four follow-ups stand as their own items.

## 2026-09-15 — Every topology change is a repair

The classifier now returns a repair for every plan difference except a
changed outer domain, a resolution change and a requested rebuild. The former
full-rebuild reasons for face assignments, curve or span topology, span
behaviour and trace equivalence became `TopologyRepairReason` variants and
name the kind of change in the handoff line; the carve itself works from the
atoms whatever the edit was. Two changes in the carve make that true.

Atom identity is purely geometric: label, separated flag, parameter range and
endpoints. Faces and regions belong to the triangles and are relabeled from
the new topology, so a face that only changes its material or its id keeps
every atom and carves nothing; the test asserts zero removed and zero inserted
triangles for a reassignment. And kept atoms meeting at one point must agree
on its sectors, one old vertex per new trace and one new trace per old vertex;
where they do not, because a divider attached to the wall turned reflecting
and the wall junction gained a sector, every atom meeting there is rebuilt.
The selection loop grows by those points the way it grows by touched
separated curves.

Tests: a face reassignment relabels without carving; a new baffle carves only
its band and is cut as a slit; removing a divider merges the faces in place
with the absorbed side relabeled; a divider flipped from transmitting to
reflecting is rebuilt with two sides; a junction of two dividers moves with
both curves' spans beside it rebuilt and the far span and every wall atom
kept. A runtime test adds a baffle to the hole scene and gets a repair.

The flip test found two mesher gaps that predate carving, both in how the two
sides of a separated span between two active faces are subdivided. The chain
expansion computed interior pieces from each side's own direction, so pieces
of a span longer than the target differed in the last bits between the sides,
and refinement split a boundary edge on one side only. Both violate the
adaptation contract's identical-subdivision rule, so adaptation could not
import a fresh mesh of a straight reflecting divider between two subdomains.
Pieces are now computed from the lower parameter upwards for both sides, and
every midpoint split goes through `split_boundary_paired`, which splits the
partner edge at the same parameter with a coincident vertex. The flip test
checks the fresh mesh against the contract as the regression.

One consequence of the plan's atoms surfaced in the crossing test: an outer
wall is one atom per side, because coarsening merges arrangement segments but
never splits one, so moving a curve's attachment along a wall rebuilds the
whole wall band. Splitting long segments to the chord cap would keep those
bands local; it is on the backlog.

## 2026-09-15 — Coordinate edits repair the active mesh by carving

The classifier's `CoordinateRepairDeferred` full rebuild is gone. A plan whose
only difference from the active one is moved geometry now classifies as
`TopologyMeshUpdateAction::Repair(CurveOrJunctionMoved)`, and the preparation
job runs a `TopologyCarveJob` in a new Repairing phase in place of the mesher,
against whatever mesh is active, adapted or not. Assembly, the transfer and
the sources follow as for a rebuild; the operator is never reused across a
repair. A carve that fails is not the request's failure: the candidate falls
back to the full mesher under a new `RepairFailed` reason and carries the
message, and the prepared topology carries the carve report. The Performance
panel's handoff line shows the repair with its kept, removed and inserted
counts, a fallback with its message, and how many of the transferred nodes
were copied exactly.

Three runtime tests: a control nudge on a hole is a repair whose transfer
copies more than half the nodes exactly; a mesh the carve cannot read, one
boundary edge relabelled with a legacy label, falls back to the full rebuild
with the reason attached; and refinement an adaptation added far from the
hole comes through a repair unchanged, triangle for triangle.

Release timing on the Obstacle array example, a control moved by 0.02, Apple
M1 Max, taken from the preparation's own buckets:

| preset | triangles | mesh rebuild | mesh repair | kept | nodes copied exactly |
| --- | --- | --- | --- | --- | --- |
| Medium, 0.08 | 2,962 | 63 ms | 6.5 ms | 96.9% | 9,056 of 9,333 |
| Fine, 0.04 | 10,804 | 326 ms | 20.8 ms | 98.8% | 32,600 of 33,047 |

Assembly and the transfer are unchanged by the repair, 29 ms and 13 ms at the
Fine preset, so they are now the larger part of a handover. The obstacle is a
reflecting curve and is rebuilt whole, 66 atoms for one moved control; that
is the whole-curve rule for separated spans doing what it should.

## 2026-09-15 — The transfer copies untouched nodes exactly

After a carve most target nodes sit at exactly the point a source node had.
`QuadraticTransferWork` now indexes the source nodes by exact point during
validation, for topology meshes only, and a target node whose point is held by
exactly one source node becomes a unit-weight sample on it instead of a bin
search and a barycentric evaluation. Two source nodes at one point are the two
sides of a separated curve, and those keep the trace-aware location. The map
reports the count as `exact_nodes` for the handoff record. A test carves a
nudged hole, assembles both operators, and checks that every coincident node's
value comes back bit for bit, that over 80% of the nodes are exact, and that
the only exposed nodes lie in the area the hole uncovered.

## 2026-09-15 — Mesh repair by carving the changed band

The topology mesher had no incremental path: every curve or junction movement
classified as `CoordinateRepairDeferred` and took the full rebuild. The legacy
repair it was meant to replace moved existing vertices with a diffused
displacement and fenced off where that inverts elements, refusing any motion
over four target edges and any patch over a third of the mesh. The user asked
for different heuristics rather than a retune, and the agreed mechanism is
carving: the plan is a set of straight atoms, so only the atoms that differ
between two plans matter to the mesh.

`TopologyCarveJob` in `mesh/carve.rs` takes the previous mesh, both plans and
the compiled topology. Two atoms are the same when their source, separated
flag, face, region, parameter range and endpoints agree; trace ids are left
out because every compile reissues them. Triangles incident to a changed old
atom go, triangles a changed new atom passes through go, one more ring goes
so the rim does not hug the new boundary, and tiny kept islands go. The kept
remainder is flood-filled across unconstrained edges; each component lies in
one face of the new topology, so one face lookup on its largest triangle
relabels it or, for an excluded face, removes it. A separated curve that a
removed triangle touches is rebuilt whole, so slit recovery always sees
complete runs; the selection repeats until that set is stable. Kept atoms'
endpoints receive their new trace ids through the kept boundary edges, whose
labels and parameters are exact copies of the plan's, so a changed atom that
ends at a kept vertex reuses it. Changed atoms are expanded with the mesher's
own chain expansion; rim edges are oriented with the removed side on the left
and take the region of the kept triangle or of the plan's atom on that side;
walking those edges with the sharpest clockwise turn at each vertex gives the
cavity cycles, counter-clockwise ones being components and clockwise ones
holes of the smallest component containing them. From there the topology
meshing job resumes at its bridge state with the imported triangles frozen:
`MeshBuilder` gained `frozen_triangles`, and legalization, refinement and
splits skip anything that would change one, so the kept part comes out exactly
as it went in. A `LocalSizeField` over the removed triangles feeds the
refinement scoring and the chain subdivision, so a band cut through an adapted
mesh is refilled at the density it had. There is no motion cap and no patch
cap; the only fallback left is a genuine mesher error.

Two things broke on the first run. Slit recovery failed because the removed
band's old vertices stayed in the builder until compaction and the constraint
insertion snaps to any vertex within a tenth of the curve tolerance, so a
moved baffle grabbed an orphan at its old position; orphans are now not
imported at all. Three hole cases ended in a scale-degenerate triangle: beside
a frozen rim the circumcenter of a bad element often falls outside the cavity,
the refiner then split the element at its centroid, and nested centroids of
the child slivers are collinear along the median. With frozen triangles the
refiner now leaves such an element alone.

Nine tests cover a nudged hole with every kept triangle reappearing unchanged
and the region areas equal to a fresh mesh's, a hole dragged 1.1 across the
domain, a hole brought closer to the wall than an edge, a baffle rebuilt whole
with both sides paired, a moved separated junction keeping its three sector
traces, a nudge through an AMR-refined mesh keeping the band at the fine size,
a hole activated as a subdomain and filled in place, the identity carve, and
determinism across slice sizes. Every result also passes the adaptation
contract import, the strictest reader of a topology mesh.

Release timing on a single rounded hole in the unit domain, Apple M1 Max:

| target | triangles | full rebuild | carve, nudge 0.02 | carve, drag 0.5 |
| --- | --- | --- | --- | --- |
| 0.06 | 4,713 | 62 ms | 10 ms, 95% kept | 15 ms, 87% kept |
| 0.03 | 18,927 | 606 ms | 38 ms, 98% kept | 62 ms, 90% kept |

What remains is proportional to the whole mesh: indexing, import and the final
verification are about 2.5 µs per triangle together, and all of it is sliced
per item, so the frame budget holds. The kept-triangle assertions are
at-least rather than equal because a Delaunay refill of the band can reproduce
a removed triangle exactly. Verification also stopped searching the boundary
list for every edge of every triangle; it consults the key set first, which
the full mesher benefits from as well.

## 2026-09-15 — Mesh atoms merge to the meshing tolerance

The arrangement samples every curve to a chord deviation of about 5e-5 so it
can intersect curves robustly, and the mesh plan took each of those segments as
an immutable atom: one boundary edge per segment, 256 of them around a single
hole, each about 7e-3 long against a target edge of 0.15. The time step
followed the shortest edge, so every curved scene ran several times slower than
its interior mesh warranted.

`TopologyMeshPlan::coarsened(topology, AtomCoarsening)` now derives the plan the
mesh is built from. It groups arrangement segments by span or outer side, sorts
them by parameter, and merges greedy runs while every joint stays within the
chord tolerance of the run's chord and the chord stays under the cap; both come
from the meshing options, the curve tolerance and the target edge. A run never
crosses an authored vertex's traces, a span boundary, a change of the faces or
behaviour beside the segment, or any trace vertex shared with another source,
so junctions, T-junctions, crossings, knots and outer corners all keep their
atoms. Segmentation is decided once per span in parameter order, so the left
and right traces of a separated span merge over identical runs and stay paired.
Face cycles are re-derived by merging consecutive steps that fall in one run,
rotated so a run straddling the cycle's first step is not cut, and their
continuity is checked; boundary atoms and trace vertices are rebuilt to match,
and the merged step keeps the smallest segment index of its run as the key that
lets both faces of a transmitting divider share one chain.

The accepted bundle's plan is the coarsened one, derived with the request's
options, and re-derived from the same compiled scene when only the options
change; the editor's compiled plan stays fine. The reuse decision now checks
the options before comparing plans, since re-atomised boundaries would
otherwise read as moved geometry, and the geometry comparison requires equal
atom counts. The mesher was already splitting each atom into pieces no longer
than the target edge by linear subdivision, so coarse atoms cost no geometric
fidelity beyond the tolerance the mesher already promised.

Tests: on a rounded separated loop the coarse plan has a quarter of the curve
atoms, every atom's arrangement points lie within the tolerance of its chord,
both sides share the same parameter ranges, the cycles are continuous and the
atoms match the steps, the coarse mesh keeps every region's area to 2e-3 with
half the boundary edges, and the solver's time step is more than three times
larger. On the outer-to-outer divider with a T-junction the coarse plan loses
no authored trace the fine plan used and meshes to the same three regions.
Adaptation refines and coarsens on a coarsened plan with its coverage and trace
checks intact, and the runtime test shows coarser options yielding fewer atoms.

## 2026-09-15 — Assembly and the transfer map yield between frames

The measured handover tail, 22 to 50 ms of operator assembly and transfer-map
construction inside the slice that finished meshing, is now cooperative.
`assemble_with_provider` was restructured into `QuadraticAssemblyWork`, a state
machine with one step per triangle for node numbering and element assembly,
one step for the boundary laws, one step per row for CSR compression and one
finishing step. The one-shot `assemble_topology` drives it to completion in a
loop, and the new `QuadraticAssemblyJob`, which owns its mesh, plan and an
`OwnedTopologyWaveModel`, spreads the same steps across frames. The transfer
map got the same treatment: the P1 locator's grid became `SourceBins` with
one insertion per source triangle and one lookup per target node, and
`QuadraticTransferWork` runs validation, binning and location as phases behind
both `QuadraticTransferMap::build` and the new `QuadraticTransferJob`.

The preparation job holds both jobs and advances them under the same step
budget as meshing, with a new `Transferring` phase and a `transfer_ms` timing
bucket that the handoff record and Performance panel show. Volume-source
compilation starts once the transfer exists. Tests assert that the stepped
assembly passes through all five phases and equals the one-shot operator, that
the stepped transfer equals the one-shot map, and that a stepped runtime
preparation visits both phases across many slices with an operator and map
equal to their one-shot counterparts.

## 2026-09-15 — Where a handover's frame goes

The user sees frame hiccups at the handover after an edit and after an AMR
adaptation. A release-mode probe of the runtime on the Obstacle array, since
deleted, split one handover's CPU work:

| target edge | DOFs | meshing (sliced) | assembly | transfer map | f32 conversion |
| ---: | ---: | ---: | ---: | ---: | ---: |
| 0.08 | 17,465 | 305 ms | 18 ms | 9 ms | 0.6 ms |
| 0.05 | 29,105 | 555 ms | 31 ms | 14 ms | 0.9 ms |

Meshing is cooperative and now sliced by time, so it is latency but not a
hiccup. Assembly and the transfer map are synchronous, and they run inside the
same slice that finishes meshing, so the frame that completes a rebuild pays 22
to 50 ms at once. An AMR handoff has no meshing but the same tail, 28 ms and 50
ms here. That is two to three dropped frames per handover on this scene, which
matches the report. The GPU upload frame was not measured from the probe; the
Performance panel's handoff record shows it as `upload_ms`. Fixing this is
mechanical: both loops become resumable jobs under the existing frame budget,
with the timing buckets already in place. Recorded as a TODO above. The
second-order item moved into the isoparametric plan (§10) at the user's request.

## 2026-09-15 — The second-order instability reproduces, and it is the condition

The user could no longer reproduce the divergence of second-order outgoing
conditions on curved spans in the app. A deterministic test reproduces it on
the f64 CPU solver through the topology meshing path: a hole and a curved
baffle, both with second-order faces, in a reflecting cavity with a pulse
released beside them. The energy grows from 1.6 to 3e18 within 8,000 steps.
The test is in `wave_quadratic.rs` and stays ignored until the condition is
fixed; run it with `--ignored`.

A release-mode bisection then isolated the mechanism. Each row is the same
scene with one thing changed; "grows" means the energy reached 1e26 or more.

| change | outcome |
| --- | --- |
| hole only, baffle only | both grow |
| straight baffle, same law | stable through t = 54 |
| time step × 0.5, × 0.25, × 0.1, × 0.05 to the same physical time | grows, from the same physical time |
| interior target edge 0.3, 0.15, 0.08, 0.04, 0.02 | grows, earlier as the mesh refines |
| second order on the left face only, right only | both grow |
| arrangement sampling 10× and 100× coarser, boundary edges 0.027 and 0.053 | grows |
| first-order impedance on the same hole | stable |

So the growth is independent of the time step, worsens under spatial
refinement, does not depend on which face carries the law or on how finely
the curve is cut, and is absent on a straight span of the same length with the
same law. That is the signature of the formulation, not of the discretization:
the Engquist–Majda second-order condition with its tangential term is applied
on curved spans without the curvature terms a curved boundary needs, and on
those spans it injects energy. The tangential stiffness itself is intrinsic to
arc length and assembles identically on straight and curved polylines, which
is why nothing in the assembly separates the two cases.

The fix is a design choice recorded in the TODO list: restrict the law to
straight spans and outer edges, derive a curvature-corrected condition, or move
to a Higdon-type condition without tangential derivatives.

The bisection produced a second finding. Coarsening the arrangement sampling
from the default to the meshing tolerance changed nothing about the interior
mesh at target edge 0.15 but cut the curved boundary from 256 edges of 6.6e-3
to 32 edges of 0.053, and the recommended time step rose from 1.1e-3 to
1.0e-2. Every curved scene pays that factor today. It is logged as its own item
because coarser atoms interact with AMR's linear subdivision of atoms.

## 2026-09-15 — Mesh resolution is a control again

The switch to the unified topology runtime dropped the Coarse, Medium and Fine
presets and left a Target edge slider that did nothing: the runtime decides
reuse from the mesh plan alone, and a resolution change leaves the plan
unchanged, so every request came back as a reuse of the existing mesh.

`PreparedTopology` now records the options its mesh was built with, and a
request whose options differ from those is a full rebuild with its own reason,
"mesh resolution changed", even when the plan is identical. A rebuilt mesh also
takes a fresh operator and a transfer map, so the running field survives the
change. A second reason, "rebuild requested", backs a Remesh button through
`request_full_rebuild`, which the next request consumes; it is how the user
leaves an adapted mesh for the base resolution without editing geometry.

The Simulation panel has the "Mesh resolution" heading back with the three
presets and Custom, keeps the slider underneath it, and shows the active versus
requested target while they differ. The rebuild waits until the slider is
released rather than starting one per frame of the drag. Two runtime tests cover
the options change and the requested rebuild, including that the request is
consumed once and the following request reuses again.

## 2026-09-15 — Preparation is sliced by time, not by step count

The user reported "Connecting holes" taking about two seconds on a single-loop
scene. Profiling the topology mesher on such a scene at the app's options gave
the whole picture: the rebuild is 60 ms of compute for a hole and 105 ms for
two subdomains, but the frame loop advanced it by 256 cooperative steps per
frame, and those steps are tiny. The bridge search alone takes 26,000 steps,
which is 100 frames, and ear clipping 137,000 to 204,000, which is 500 to 800
frames. The whole rebuild needed 800 to 1,200 frames, so ten to twenty seconds
at 60 Hz, for a tenth of a second of work.

| phase | steps | compute | frames at 256 steps |
| --- | ---: | ---: | ---: |
| Connecting holes | 26,061 | 0.75 ms | 102 |
| Triangulating | 137,288 | 5.9 ms | 536 |
| Legalizing edges | 43,579 | 19.9 ms | 170 |
| Refining | 1,440 | 24.3 ms | 6 |
| Checking mesh | 3,592 | 11.5 ms | 14 |

The preparation job and the runtime gained `advance_for`, which runs slices of
256 steps until the job finishes or a wall-time budget has passed and counts
the call as one slice. The frame lends it 6 ms, so the same rebuild now takes
ten to twenty frames. The step-counted `advance` stays for tests. A test checks
that one unbounded call finishes a fresh preparation and reports one slice with
the same mesh the stepped path produces, and that a zero budget still makes one
slice of progress per call. The remaining open item on preparation latency is
the synchronous operator assembly and transfer tail, already logged above.

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

## 2026-09-23 — The batch ceiling was measuring the wrong thing

The frame-pacing controller shipped yesterday collapsed the solver batch to
exactly one step a frame: 2638 of 2670 frames, measured. Frame rate held at 120
but the simulation ran at a tenth of the speed asked for, which is how it was
reported — "simulation is just bolted to the framerate, fps==sps".

Two defects, both in how the display's cadence was estimated, and both invisible
to the test that guarded it.

The estimator tracked the fastest frame seen lately, on the reasoning that a
saturated solver must not be able to define its own slowness as normal. But the
fastest frame is not the display — a stall is followed by a very short frame
(99.9 ms then 2.56 ms in the trace), so the estimate latched at 2.56 ms against
a true 8.32 and every real frame read as a threefold overrun. The obvious
repair, a median of recent frames, fails the other way: once the batch is large
enough to slow every frame, the median is the solver's slowness, and a closed
loop over the recorded jitter settles it at 60 fps of a 120 Hz panel.

The fix is to choose which frames to measure rather than how to average them. A
frame carrying at most one step cannot have been slowed by the solver, so a
median over a window of those is the display and nothing else. Such frames are
plentiful: every paused frame, every frame a handoff withholds stepping on, and
every frame while the budget climbs from its floor — which is why the budget now
starts at the floor rather than the ceiling.

The tolerance band moved 1.05 → 1.5 and the recovery 0.5 → 0.05, both from
measurement rather than from taste. With the batch pinned at one step, 15.5 % of
frames still exceeded 1.05x the refresh period and 1.7 % exceeded 1.5x: at 1.05
the band sat inside the display's own jitter and the false accusations alone
pinned the batch. And since the controller can only find the edge by crossing
it, the ratio of backoff to recovery is the steady-state rate of dropped frames;
0.5 probed on a quarter of all frames.

Measured on the reported scene, before and after: 120 fps / 120 steps a second,
against 120 fps / 1178. The originally reported defect was 65 fps / 900.

What let both of these ship was the test. It modelled a frame as
`overhead + steps x cost` — smooth, no jitter, no presentation boundary — and
neither defect can exist in that model. The gate is now driven by 600 frame
deltas this app actually measured (`measured-frame-deltas.txt`), presented
through a model that pays for overrun a refresh period at a time with one frame
of pipeline slack. That model reproduces the reported linear operating point to
within 8 % without being fitted to it.
