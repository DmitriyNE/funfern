# funfern code review before milestone C

A read-only review of the whole codebase at 569e56b, made on 2026-10-19 and
2026-10-20 before milestone C began. Nothing in it has been fixed yet. The
finding IDs (RT-1, GPU-2 and so on) are the ones later commits and log
entries refer to.

## How it was made

- **Find.** The code was cut into 74 units of about 4,000 lines: every Rust,
  WGSL, CI, script and browser-test file, 176 in all. Each unit was read in
  full by one reviewer. Ten cross-cutting reviewers each followed one concern
  across the code:
  - the host and device protocol;
  - Rust and WGSL layouts;
  - time and precision;
  - AMR and transfer;
  - persistence;
  - concurrency;
  - errors that never reach the diagnostics warning;
  - core numerics;
  - the verification machinery;
  - interaction ordering.
- **Verify.** 253 raw findings came to 205 once merged. Each was handed to
  two independent skeptics who tried to refute it. One traced the code path;
  the other checked intent, the docs, the log and the "Worth checking
  sometime" list. A split went to a third. 199 survived and 6 were refuted.
- **Gaps.** A completeness critic then set eight targeted reviews, which
  added 15 confirmed findings.
- **Read-only.** Everything was checked by reading code. Nothing was built
  or run, so every finding is to be reproduced before it is fixed.

## Status at the time of writing

- **RT-1** is confirmed by hand. The candidate's clock starts at the running
  time, but its temporal state is built at zero, so `compile_temporal`
  refuses it.
- **VER-5** undercounts. The nightly of 2026-10-19 reports five mutation
  patches that no longer apply: `clock-forgotten-on-handoff`,
  `component-totals-dropped`, `curve-probe-flux-reversed`,
  `drive-phase-reset-on-handoff` and `far-field-frame-at-step-end`.
- **Not in the review.** The same nightly ran out of time on `observables`
  on SwiftShader, in both hostings.

## 1. Summary

The review confirmed 214 findings: 12 high, 88 medium and 114 low. Every finding was either confirmed or refuted, so none is left as plausible or unverified.

| Area | High | Medium | Low | Total |
|---|---|---|---|---|
| Runtime, handoff and protocol | 4 | 8 | 6 | 18 |
| GPU shaders and device | 2 | 5 | 7 | 14 |
| Probes, recorders and readouts | 2 | 8 | 7 | 17 |
| Core numerics and materials | 1 | 9 | 14 | 24 |
| Meshing, AMR and state transfer | 1 | 12 | 12 | 25 |
| Scene editor | 1 | 4 | 0 | 5 |
| UI, display and gallery | 1 | 12 | 22 | 35 |
| Persistence | 0 | 5 | 1 | 6 |
| Verification machinery | 0 | 25 | 45 | 70 |
| **Total** | **12** | **88** | **114** | **214** |

**Patterns across the findings:**
- **One recent commit.** ab17b11 started packing handoff candidates against the running clock's origin. That change is behind RT-1 and GPU-2, and it broke four mutation patches (VER-5).
- **Errors that skip the diagnostics warning.** Several failure paths that only a defect can trigger stop at the status line and never light the diagnostics warning, against the project rule: RT-6, RT-7, RT-8, RT-15, PR-3, PR-4, PR-11, UI-8 and UI-32.
- **Shared rules hide errors.** The host and the device often share one rule (NUM-1, NUM-13, PR-2). When the rule itself is wrong, the device-vs-reference fixtures cannot see it.
- **Races the model cannot see.** The protocol model treats handoff and live-event settlement as independent transitions, so it cannot reach the races in RT-2 and RT-3.
- **Weak checks.** 70 findings are in the checks themselves: assertions that cannot fail, oracles that restate the code under test, and gates that skip the modes they document.

**Fix first:**
1. **RT-1.** Once a run has advanced, every edit and every AMR adaptation on a driven or nonlinear scene (pump, time crystal, Kerr, Josephson, Switch) is refused. It is a regression from ab17b11 and the fix is local. GPU-2 comes from the same commit.
2. **GPU-1.** After any refused step on a Kerr or saturable scene with the default grid filter, the device commits phantom steps. A retry or a corrective edit then continues from a half-written state at the wrong time.
3. **RT-2, RT-3, RT-4.** A pulse or Switch pressed near a handoff causes spurious layout faults, events applied twice, or a stuck coordinator that only a scene reload clears. One guard in `begin_handoff`, plus re-arming events after `clear_failure`, removes most of it.
4. **ED-1.** Any deletion silently turns every switched-off (transmitting) free curve back into a reflecting wall. That changes the physics far from the edit.
5. **PR-1 and PR-2.** Boundary probes are the tool for measuring flux around obstacles. On the Right side they give scrambled profiles and integrals 2 to 3 times too large. On a separator they give flux whose sign flips from sample to sample.

## 2. Confirmed findings

### Runtime, handoff and protocol

#### RT-1 · High · Every handoff to a driven or nonlinear generation is refused once the run has advanced
`crates/funfern-app/src/ui/workers.rs:737`

**What goes wrong.** Take a scene with a time-driven or field-dependent medium: Parametric pump, Time crystal, Kerr slab, Josephson line or a Switch. Once simulated time is above zero, every material edit, geometry drag, Remesh and AMR adaptation is packed and then refused with "the temporal state, clock and operator must share one boundary". The edit never reaches the solver, and AMR keeps preparing candidates that are refused in turn. The refusal sets the status line. One skeptic found that it also reaches the diagnostics warning, so it is not silent, but nothing tells the user why.

**Why.** Since ab17b11, `compile_gpu_upload` packs a non-fresh candidate against `CanonicalGpuClock { epoch_origin_seconds: running_time, .. }` (workers.rs:643-650). `compile_generation_plan`, however, still builds the temporal state with `CanonicalTemporalWaveState::zero` (workers.rs:735-739), whose time is 0. `CanonicalGpuPlan::compile_temporal` (canonical_gpu.rs:1398-1405) refuses any state more than 16 eps away from `clock.time()`. The fixed path has no such check, so the late fixtures, which use fixed walls, still pass. Every unit test that packs a driven handoff uses `running_time = 0.0`, and the sequence harness takes the pack result as an input.

**Fix.** Build the candidate's temporal state at the running time, so the state and the clock share one boundary. Add a driven-handoff pack test with a running time above zero.

#### RT-2 · High · A pulse or Switch still pending when a handoff begins makes the device run the transfer twice or apply the event twice
`crates/funfern-app/src/canonical_gpu.rs:5230`

**What goes wrong.** With AMR on, or while editing a paused run, a pulse or Switch queued 2 to 3 frames before `begin_handoff` races the handoff receipt. Two outcomes are possible:
- **The event's acknowledgement arrives first.** The transfer is encoded again on the already-committed target, which latches STATUS_LAYOUT. After a perfectly good adaptation the app pauses with "Layout or transfer consistency check failed (failure code 1)".
- **The receipt settles first.** The event is encoded again on the target after the transfer has already carried its effect. On a remesh it uses the source mesh's node numbering, so the increments land on unrelated nodes, and a Switch's material index can name a different material.

**Why.** `queue_live_event` refuses while a handoff is pending. But `begin_handoff` (5230-5321) never looks at `self.live_event`, and the begin block in `refresh_runtime` (runtime.rs ~601) checks only `uploading`, `caught_up` and the pack result. The pulse and Switch blocks check only `uploading` and `source_commit`.
- In the first outcome, `settle_canonical_live_event` bumps the revision (6508). The handoff bind groups are rebuilt with `encoded: false`, and the transfer runs again against a map whose header the first run's `correct_primary` has already overwritten with the receipt. canonical_transfer_runtime.wgsl:277 rejects it, and canonical_wave.wgsl:2352 latches the rejection.
- In the second outcome, `settle_canonical_handoff` keeps `request.live_event` and installs stats with `processed_event = 0`. The new bind group therefore starts at `encoded_live_event = 0` (7127-7152).

The protocol model and the generations harness settle the handoff and the live event as independent transitions, so neither can produce this race.

**Fix.** Refuse `begin_handoff`, or hold the begin block, while a live event is still unacknowledged. Model that ordering in funfern-protocol so the proofs cover it.

#### RT-3 · High · A live event encoded after an unseen device failure is dropped but recorded as sent, and a source commit then blocks the runtime
`crates/funfern-app/src/canonical_gpu.rs:7143`

**What goes wrong.** Take a Kerr or saturable medium and drag the source amplitude up until a step fails with INVERSE_DOMAIN. A source patch queued after the failure, but before the status readback arrives, is encoded on the stopped device. After the user presses Run the field steps again, but `coordinator.source_commit` never clears:
- `request_runtime` returns early on every frame, so no edit takes effect.
- Reset is gated on `source_commit.is_none()`, so it never runs.
- Pulses and Switches wait forever until another scene is opened.

A pulse or Switch lost the same way leaves `live_event` pending, so every later event is refused as busy. The next handoff then encodes it again on the target (RT-2).

**Why.** The host learns of a failure only from the status readback, at least two frames late. Until then:
- `queue_live_event` (5098) sees no failure, and `compute_canonical_wave` encodes the event and records it as sent (7895).
- On the device, `live_event_stage` returns at once because `stopped()` is true. `commit_event` (canonical_wave.wgsl:2123-2133) only latches the failure again, so `processed_event` never reaches the serial.
- `clear_failure` does not re-arm the event. The cleared-failure branch of `prepare_canonical_bind_group` (7138-7144) carries `encoded_live_event` forward, even though its own comment says that anything encoded after the failure never committed.
- `settle_canonical_live_event` and `finish_source_commit` (ui/runtime.rs:340) wait for a serial that never arrives.

The model's `Step::Settle` settles a pending event whatever the device's failure state, so the lockstep harness cannot see this.

**Fix.** On `clear_failure`, rewind the encoded event serial to the processed one so the event is encoded again, or reject it explicitly so the coordinator can settle it. Make the model's settle step depend on the failure state.

#### RT-4 · High · A pending handoff is encoded a second time after any revision bump, and the rerun reads a transfer map that has already been consumed
`crates/funfern-app/src/canonical_gpu.rs:7279`

**What goes wrong.** A paused run takes a geometry edit through Route::Pack. While the pack finishes, the user clicks a pulse. When the pulse settles the revision is bumped, and the whole transfer is encoded again. If the first receipt is the one read, the target is published and immediately reads as failed with a layout failure. Otherwise the second receipt refuses the handoff and the edit is dropped. One skeptic rates this medium: it needs a click inside a window of a few frames, and the failure is visible and recoverable.

**Why.** `prepare_canonical_handoff_bind_groups` decides whether to rebuild from the revision alone (7277-7282) and always inserts the rebuilt groups with `encoded: false` (7409-7418). `compute_canonical_handoff` encodes whenever that flag is false and the revision matches (8293). It never reads `handoff.stats.submitted`, which is set at 8419 and otherwise unused. The rerun fails because:
- The first run's `correct_primary` has already overwritten the transfer header with HANDOFF_RECEIPT_MAGIC (canonical_transfer.wgsl:394-418).
- `transfer_runtime` and `transfer_clock` therefore reject the rerun (canonical_transfer_runtime.wgsl:277, 401).
- `handoff_commit` latches that rejection (canonical_wave.wgsl:2345-2354).

The solver's own bind group carries its encoded counters across a revision bump. The handoff groups do not.

**Fix.** Carry the encoded flag across a rebuild within one handoff, so the transfer is encoded exactly once. Fixing RT-2 also removes the usual trigger.

#### RT-5 · Medium · `accepted_material_runtime` decodes a snapshot taken before a rebase against the new epoch's origin
`crates/funfern-app/src/canonical_gpu.rs:5425`

**What goes wrong.** On a pumped or Switch medium, a frame's batch can end on the last step of an epoch while a full snapshot is copied. Until the next snapshot, about 250 ms later, the decoded drive phases are off by ω(L−1)dt and Switch start times are about 256 s early. Several consumers read that wrong runtime:
- `stored_energy_view`
- AMR indicators (ui/amr.rs:555), which can trigger an adaptation
- the Switch state readers

**Why.** A rebase is encoded at the start of the loop iteration that runs the next step (7988), so a frame can end at local step L−1 with the rebase still pending. After the rebase, `step_in_epoch` equals `behind`, so the check `clock.step_in_epoch < behind` passes. The snapshot metadata carries only the absolute step, so equality cannot tell the two cases apart. The unit test covers snapshots taken 50 and 5 steps into the epoch, but not 10.

**Fix.** Stamp each snapshot with its epoch and refuse to decode it against a different epoch.

#### RT-6 · Medium · Reset does nothing when its plan fails to compile, stays latched, and recompiles every frame
`crates/funfern-app/src/ui/runtime.rs:782`

**What goes wrong.** If the zero-state plan fails to compile, pressing Reset does nothing and nothing is reported. Frame time rises because the whole plan is recompiled on every frame. Later, an unrelated edit is prepared fresh and silently zeroes the field.

**Why.** The plan is installed only `if let Ok(plan) = reset`, with no else branch. An Err sets no message, no `last_error` and no `unseen_error`. `reset_requested` is cleared only by `reset_installed` or `requested()` (coordinator.rs:104-148), so the guard at 746-750 holds on every frame. The latched flag also feeds `starts_from_zero` (199).

**Fix.** On Err, clear the request and record the error through `last_error` or `unseen_error`.

#### RT-7 · Medium · A failure while creating the preparation job goes only to the status line and is retried every frame
`crates/funfern-app/src/ui/runtime.rs:240`

**What goes wrong.** Some consistency failures can only come from a defect: plan re-coarsening, face assignments, or "Compiled topology does not match its authored scene". When one happens:
- The status line is rewritten every frame, and the diagnostics marker never lights.
- Every frame clones the compiled scene and rebuilds the job, and the edit never prepares.
- A remesh or force_rebuild requested in that frame is lost.

**Why.** In `request_with_handoff`, the `?` on `TopologyPreparationJob::new` (topology_runtime.rs:438-447, AcceptedTopology::new 36-69) returns before `preparing`, `requested` and `last_error` are set. The caller only assigns `self.message`. `request_due` then finds the revision unaccounted for, which reopens the "retried every frame" defect its guard was added to close.

**Fix.** Record a job-creation error as a failed request for that revision, by setting both `last_error` and `requested_revision`.

#### RT-8 · Medium · A dead preparation or AMR worker is never noticed on native builds
`crates/funfern-app/src/ui/workers.rs:97`

**What goes wrong.** A panic in a preparation or AMR job leaves the worker's phase or kind set for good:
- `preparation_in_progress()` stays true, so the status line shows a preparation phase indefinitely.
- AMR keeps answering "geometry has priority", or shows "Adapting mesh" forever.
- No warning appears. Only a new edit, which runs the job on the main thread, gets things moving again.

**Why.** Both workers drain their channel with `try_recv().ok()` (line 97 for preparation, 477 for AMR), which treats Disconnected the same as Empty. Native builds unwind on panic, and the JoinHandles are discarded. The GPU pack path handles this case (runtime.rs:571), and the browser has BROWSER_BACKGROUND_FAILURE, but native preparation and AMR workers have neither.

**Fix.** Treat Disconnected as a reported failure that clears the worker state and respawns the worker.

#### RT-9 · Medium · A Reset during an in-flight GPU pack hands off with the discarded run's clock origin
`crates/funfern-app/src/ui/runtime.rs:746`

**What goes wrong.** A scene has run for 60 s. The user makes an edit and presses Reset while the candidate is still packing. Reset installs a zero field at t = 0, and the finished pack then hands off a plan packed against an origin of 60 s. The device's `preparation_delta` comes out at about −60 s, which shifts the anchors of drives, pulses, gates and Switches carried through the handoff. Both skeptics confirm the stale origin, but they disagree with the finding about which values end up wrong, and the stated new-pulse example may not differ from the reference. The impact is therefore narrower than first described.

**Why.** `dispatch_pack` captures `running_time` (253), and `compile_gpu_upload` packs the plan's origin from it (workers.rs:647). The Reset block (746) checks only `uploading` and `source_commit`, never `gpu_upload_preparation`, so the job survives. `begin_handoff` never compares the plan's origin with the running clock.

**Fix.** Drop or re-pack the in-flight pack when Reset installs, or have `begin_handoff` refuse a plan whose origin is ahead of the running clock.

#### RT-10 · Medium · The display cadence counts frames that backpressure throttled as quiet frames
`crates/funfern-app/src/ui/pacing.rs:441`

**What goes wrong.** On an overloaded scene, for example edge 0.04 at 2× speed under GPU contention, the 60-sample quiet window fills with frames that backpressure clamped. Their frame times are slowed by the solver. As a result, the late-frame test stops firing, the StepBudget fit roughly doubles, and the queue stays saturated. This is the "settles for 60 fps of a 120 Hz display" failure the cadence exists to prevent. The runtime effect was not measured.

**Why.** `DisplayCadence::observe` treats any frame whose recorded batch is at most 1 step as quiet. The runtime records the admitted count after `steps_with_gpu_backpressure` (runtime.rs:963), and that count is 0 or 1 exactly when 63 to 64 steps are still outstanding on the GPU.

**Fix.** Record the requested batch, or an outstanding-work flag, and exclude frames with a GPU backlog from the quiet window.

#### RT-11 · Medium · A speed retime re-requests a failed retimed candidate every frame
`crates/funfern-app/src/ui/probes.rs:592`

**What goes wrong.** The user changes speed by more than 10% and the retimed candidate fails in a way that repeats. The app then re-prepares, repacks and fails on every frame, without end. The CPU stays busy and each error replaces the last before it can be read.

**Why.** `retime_for_speed` clears `requested_revision` whenever the uploaded step is more than 10% off, and never checks `last_error`. A refused handoff keeps the old `uploaded_time_step` (runtime.rs:630-634), so the condition holds again on the next frame. The model mirrors this code (protocol lib.rs:493-500), but its fair suffix pins retime to false (lib.rs:877), so the proofs do not cover the loop.

**Fix.** Remember the step a retime was attempted for, and do not retry that step after a failure. Let the model express a refused retime.

#### RT-12 · Medium · The u32 total-step counter wraps into the u64 completed count, freezing the solver on the next edit
`crates/funfern-app/src/canonical_gpu.rs:6213`

**What goes wrong.** After 2^32 accepted steps without a Reset, which is about 6.5 days at full speed on a 120 Hz display (AMR handoffs carry the count across):
- AMR sits at "monitoring solution" for good.
- The first edit packs a candidate and then withholds all steps forever.
- `fire_times` schedules starts about 4.29e9·dt into the future.
- A snapshot is held forever.

Reset recovers.

**Why.** `clock_u32.w` is incremented without a carry (canonical_wave.wgsl:4527) and carried across handoffs (canonical_transfer_runtime.wgsl:409-410). `receive_canonical_control` stores it as-is into the u64 completed count, while the desired, encoded and retired counts stay u64. `caught_up()` (4802) is then false until another 2^32 steps have run.

**Fix.** Unwrap the device counter on the host against the last known u64 value, or add a carry word on the device. PR-10 needs its own shader-side fix as well.

#### RT-13 · Low · A Reset folded into a fresh candidate is lost when that candidate is superseded or fails
`crates/funfern-app/src/ui/coordinator.rs:108`

**What:** When a request is due in the same frame Reset is first seen, the Reset survives only as that candidate's fresh flag. A later edit made while the candidate prepares, or a failed preparation, drops it, and the run hands off the old field. The skeptics found that the upload-time scenario in the original finding does not happen, because the Reset branch runs in the publish frame. The narrower paths described here do happen.

**Why:** `Coordinator::requested()` clears `reset_requested`, and the superseding request is non-fresh.

**Fix:** Keep `reset_requested` set until a fresh generation actually installs.

#### RT-14 · Low · A rejected live event is reported again after every later handoff
`crates/funfern-app/src/ui/runtime.rs:986`

**What:** After one rejection, every AMR handoff shows the rejection message again and relights the marker.

**Why:** `transfer_clock` copies `event_result`, including the failure code (canonical_transfer_runtime.wgsl:417). The new stats start at `processed_event` 0, so the observed serial goes from N to 0 and back to N.

**Fix:** Do not carry the failure code across a handoff, or keep a record of serials already reported.

#### RT-15 · Low · A background worker's abort shows only on the status line
`crates/funfern-app/src/ui/diagnostics.rs:79`

**What:** In the threaded web build, a pool worker that panics or runs out of memory shows a status text styled as Idle. The marker never lights and the log stays empty, while every later edit waits forever.

**Why:** `browser_background_failure()` is read only by `status()` and `refresh_amr`, and the latter does not set `amr_error`.

**Fix:** Include the flag in `diagnostics_warning` and log an entry when it is first set.

#### RT-16 · Low · `ui::frame` runs once per egui pass, so a two-pass frame admits a second batch
`crates/funfern-app/src/ui.rs:2672`

**What:** Resizing a readout window triggers egui's discard-and-rerun. The rerun adds another frame delta, admits another batch of steps and releases another picture, which causes a visible hitch and false pacing readings. The skeptics found that the "Plots" grid trigger named in the original finding does not happen. Window resizing does.

**Why:** Multipass is enabled, and nothing guards the per-frame runtime work against running twice.

**Fix:** Run the runtime work on the first pass only.

#### RT-17 · Low · The snapshot's clock gate passes on the previous generation's clock after an install
`crates/funfern-app/src/canonical_gpu.rs:5955`

**What:** After a Reset, the new generation's first snapshot passes the gate against the old clock. That update's energy and AMR peak are then evaluated at t = 0 with the authored runtime. The skeptics found this almost invisible in ordinary use, but it is larger for clocks started late.

**Why:** `install` leaves `CanonicalGpuDisplay` alone, and the gate does not check which generation the clock belongs to.

**Fix:** Require the clock's generation to match the snapshot's before applying it.

#### RT-18 · Low · A Switch's remembered direction outlives a handoff that drops and re-creates its runtime record
`crates/funfern-app/src/ui/runtime.rs:1173`

**What:** The user throws material A's Switch, reassigns A's region and then assigns it back. The new generation starts A at base, but the button still reads "Switch ⏴", so the first press sends a no-op.

**Why:** `switch_targets` is cleared only on Reset or a fresh start, and it outranks the runtime state. A re-created runtime record starts at blend 0.

**Fix:** Reconcile `switch_targets` with each published generation's runtime.

### GPU shaders and device

#### GPU-1 · High · The resident filter clears a failed step's status under the latch, so later steps commit phantom steps
`crates/funfern-app/src/canonical_wave.wgsl:2180`

**What goes wrong.** Take a Kerr or saturable scene with the grid filter on, which is the default. A step fails with INVERSE_DOMAIN or INVERSE_CONVERGENCE. The next resident filter, which runs every 16 steps, clears the failed step's status. After that:
- Each later step flips the state lane and advances the clock without computing anything.
- The "accepted" lane alternates between the last good state and the failed step's half-written candidate.

The app still says "Paused at the last accepted step". But the displayed field, a corrective edit's handoff (documented as starting "from the accepted state, which the failure never touched") and Run-to-retry all continue from a mid-step candidate at the wrong time. Point probes record the phantom steps too.

**Why.** For field-law and restoring generations, `resident_filter_commit` treats a pending TIMESTEP, INVERSE_DOMAIN or INVERSE_CONVERGENCE code as "filter not taken" and clears `status.candidate`. It never checks whether the filter actually ran, i.e. whether the latch is clear or the event operation is 2. The filter is encoded with no failure check (canonical_gpu.rs:8156). `commit_step` (~4520) reads only `candidate`: later stages return early on `stopped()`, but the commit still flips the lane, advances the clocks and republishes the snapshot metadata. The next filter commit then finds the candidate cleared, flips again, and copies stale accounting into the ledger.

**Fix.** Take the "not taken" branch only when the filter itself ran, and leave an earlier step's code in place. Have `commit_step` check the latch as well.

#### GPU-2 · High · A drive started by a handoff has its carrier moved to the handoff instant but keeps its rate anchor at preparation time
`crates/funfern-app/src/canonical_transfer_runtime.wgsl:250`

**What goes wrong.** A pulsed source is switched to Continuous at time T, which takes a full handoff. Adding a harmonic Neumann side mid-run does the same. The new drive's integrated rate then carries a constant offset of up to its full amplitude:
- Behind reflecting walls the field level ramps without bound.
- Behind damping walls it settles into a permanent offset.

The f64 reference shows neither.

**Why.** `start_drive` advances only the carrier phase by `preparation_delta` and copies the rate anchor unchanged. Since ab17b11 the host packs that anchor as the drive's value at the preparation time, `drive.value(epoch_origin_seconds)` (canonical_gpu.rs:3697), where it used to be 0. The device's mean rate becomes (A/ω)(sin ωT_prep − sin ωT_h). That contradicts the stage-6 report and `start_drive`'s own comment, both of which say a new source starts with zero integrated rate at the handoff time.

**Fix.** Re-anchor the rate at the handoff instant together with the carrier, matching the reference's anchoring at t = 0. Add a fixture that switches pulse to continuous mid-run.

#### GPU-3 · Medium · After a handoff, a driven generation pins linear prescribed nodes at the authored mass, not the mass in force
`crates/funfern-app/src/canonical_wave.wgsl:2271`

**What goes wrong.** A Dirichlet-pinned edge sits on a material with a fired Switch (factor 4). After a handoff, the device pins the flux at m_a·g(0), while the reference pins it at 4·m_a·g(0).
- The handed state shows the pin at a quarter of its signal, and this stays on screen while the run is paused.
- The prescribed exchange is booked wrong.
- With a loss stage, the first step charges the wrong loss permanently.

The e2e handoff check leaves pins out (e2e.rs:1545-1556).

**Why.** The linear branch of `handoff_finalize` uses `mass_loss.x`, the authored mass from `primary_mass()` (canonical_gpu.rs:2099). The reference uses `primary_mass_at(time, runtime)` (canonical_temporal.rs:4578-4594). The device's own nonlinear branch already uses the temporal mass.

**Fix.** Use the temporal primary mass at local time 0 for linear pins on driven generations. Include pins in the handoff check.

#### GPU-4 · Medium · A softly rejected event makes the accounting ledger count the pre-event bank twice
`crates/funfern-app/src/canonical_wave.wgsl:4491`

**What goes wrong.** Take a Kerr or saturable scene with the filter on and a source, loss or wall. When a filter candidate is not taken, or a live event is rejected, the next `reduce_accounting` adds the pre-event contributions again, and the doubled amount stays in the ledger. One skeptic rates this low, because nothing in the shipped app reads the device ledger. Device-suite examples that compare the accounting would fail spuriously.

**Why.** `event_begin` copies the accepted total into the candidate and clears only the candidate bank (1720-1731). It relies on the commit flipping the lanes afterwards. The two soft-reject paths (2180-2184 and 2126-2130) restore the event slot but neither restore the base nor clear the accepted bank.

**Fix.** In both soft-reject paths, restore the pre-event base or clear the accepted bank.

#### GPU-5 · Medium · The acceptance pass of a linear law patch is under-dispatched
`crates/funfern-app/src/canonical_gpu.rs:7865`

**What goes wrong.** A fixed generation has a complementary loss of 0.3/s, and a `linear_loss_patch` sets it to 0. The event commits as accepted, but roughly the last third of the samples keep the old loss forever. The device runs that use this path do not catch it, because they patch in the values the operator already has.

**Why.** `event_accept_tables` op 3 indexes nodes and samples in one combined space, so it needs N+S threads. The host dispatches only `max(N, S)` worth of workgroups.

**Fix.** Dispatch over N+S, as the stage passes do. Add a device run that patches to values the operator does not already have.

#### GPU-6 · Medium · Each clock rebase re-forms an integrated source's rate anchor in f32, leaving a permanent DC rate error
`crates/funfern-app/src/canonical_wave.wgsl:2396`

**What goes wrong.** A harmonic point source runs for an hour or more with reflecting walls. Each rebase, every 65 to 256 s, adds a constant rate offset of about 1.5e-5·A, and the offsets random-walk. The cavity's mean field drifts linearly away from the reference. Unlike the late-epoch error the plan accepts, this error is not bounded by the epoch. No fixture runs an integrated source across a rebase.

**Why.** `rebase_clock_records` sets the anchor to `source_drive(i, elapsed)` in f32, with elapsed around 256 s, and reduces the phase separately. The two are rounded independently at the magnitude of ω·elapsed, so the DC part, anchor + (b/ω)cos φ, shifts at each rebase.

**Fix.** Preserve the DC part exactly across a rebase, for example by forming the anchor from the already-rounded new phase. Add a fixture with an integrated source that crosses a rebase. GPU-10 is the same defect at handoffs.

#### GPU-7 · Medium · Nothing compares buffer sizes with the device's limits
`crates/funfern-app/src/canonical_gpu.rs:3553`

**What goes wrong.** Two cases go over the WebGPU defaults:
- A fixed scene with second-order walls whose trace grows to about 4,100 nodes has a boundary buffer of about 134.5 MB, over the 128 MiB default `maxStorageBufferBindingSize`.
- The tables binding passes 128 MiB at about 175k DOFs.

At the handoff, `create_bind_group` raises a validation error and rendering stops for good (natively the app exits). The running generation is lost, although the scene could have been refused. The stop path does report it, so it is not silent.

**Why.** No code reads `RenderDevice::limits`, and every binding binds a whole buffer. The boundary buffer packs the T×T modal matrix plus a dense trace inverse, about 8T² bytes in all (3553-3567), and the AMR cap allows around 300k nodes. The handoff refusal outcome exists, but no size check feeds it.

**Fix.** Check packed buffer sizes against the device limits at pack time, and refuse the candidate through the existing rejection path.

#### GPU-8 · Low · A live source patch of a pulse train rounds its start at the absolute run time
`crates/funfern-app/src/canonical_wave.wgsl:1588`

Already listed as worth checking. The skeptic found it recorded in the engineering log rather than in plan.md's list.

**What:** After 10 h, editing a repeating pulse source shifts every later pulse by up to about 2 ms, and by 7.8 ms at 36 h, until a new generation installs. At 1 h the shift is already above the plan's 3e-5 s.

**Why:** `source_patch` packs the start against origin 0. The device then forms the start as a single f32 at the magnitude of the absolute time before reducing it by the repeat. The install path does this reduction in f64 on the host.

**Fix:** Pack live patches against the running clock's origin, as the install path does.

#### GPU-9 · Low · The trapezoid kick books a driven pin's work at the mean flux over the stage mass
`crates/funfern-app/src/canonical_wave.wgsl:2642`

**What:** On a pumped medium with a harmonic Dirichlet side, the device's prescribed-exchange and source lanes drift from the reference at first order. The trajectory is unaffected.

**Why:** `kick_trapezoid_node` uses 0.5(old+kicked)/M, where the other kick paths use the signal. `apply_wall_step` also sets the trapezoid flag even when the scene has no walls (canonical_gpu.rs:1339), and the reference does not.

**Fix:** Book a pin's work at its signal, and set the trapezoid flag only when the scene has walls.

#### GPU-10 · Low · Each handoff re-forms a retained integrated source's anchor in f32
`crates/funfern-app/src/canonical_transfer_runtime.wgsl:227`

**What:** With AMR handing off every few seconds, each handoff adds about 1e-5 of the rate amplitude as a DC rate error. Over N handoffs this grows like √N, and the mean level of a sealed region rises. The skeptics judged the original severity overstated.

**Why:** `retain_drive` and `replace_drive` rebuild the anchor and the phase separately in f32. The CPU twin never re-anchors.

**Fix:** Apply GPU-6's remedy through a helper that both functions share.

#### GPU-11 · Low · A live linear law patch can switch the force cache on without refreshing the cached force
`crates/funfern-app/src/canonical_wave.wgsl:2148`

**What:** A fixed scene with complementary loss runs N steps, and a patch then sets those rates to zero. From the next step every node feels a constant extra force, force(b0) − force(b_N). The shipped UI cannot reach this, but examples and tests can.

**Why:** The cached lanes are never advanced while the cache is off. Commit op 3 turns the cache flag on without recomputing them.

**Fix:** Recompute the cache when a patch turns it on, or refuse patches that change the flag.

#### GPU-12 · Low · `temporal_law_patch` admits a change from a linear law to a field law
`crates/funfern-app/src/canonical_gpu.rs:920`

**What:** A patch that adds a Kerr χ is accepted. Afterwards the samples use the Kerr secant while the nodes, filter and energy stay linear, so the generation is half nonlinear.

**Why:** The validator compares only words x, y and z of each record, not the field bits or parameters. The queue refuses such a patch only when the current generation already has field laws.

**Fix:** Refuse a patch whose field-law flags or parameters differ from the current generation's.

#### GPU-13 · Low · Readback staging buffers of a retired size are dropped without `destroy()`
`crates/funfern-app/src/paced_readback.rs:222`

**What:** In the web build under AMR churn at about 137k DOFs, each handoff leaves up to 3 staging buffers of 2.2 MB each for the JS garbage collector, around 2 MB/s. Native builds free them on drop.

**Why:** `retire_unused` and `discard` drop the buffers through `retain`. On wasm, wgpu 29's buffer Drop does nothing, and the pool is keyed by byte size, which changes with every handoff.

**Fix:** Call `destroy()` on buffers that leave the pool.

#### GPU-14 · Low · The kick form's first rate pass reads a first-order wall node's flux before the short-wave half
`crates/funfern-app/src/canonical_wave.wgsl:4060`

**What:** In the fourth-order kick form with a first-order absorbing wall and short-wave loss next to it, the corrected force at the wall's neighbours differs from the reference by an h²/12 term on every step.

**Why:** The reference applies the short-wave half before `wall_rate_at`. The device's rate pass (pipeline 42) runs before `kick_first`, which is where the half is applied.

**Fix:** Apply the half before the rate pass on the device.

### Probes, recorders and readouts

#### PR-1 · High · Right-side boundary probe samples run backwards inside every plan atom
`crates/funfern-app/src/topology_runtime.rs:1868`

**What goes wrong.** A boundary probe around a hole must use the Right side, because the Left side has no active trace. Take a straight Right-side probe 1.6 long at edge 0.08:
- The sample positions come out as 0.08, 0.055, 0.029, 0.16, …, so the profile is scrambled at the scale of one mesh edge.
- The probe length, summed over sample distances (probe_hit.rs 322-326), comes out about 2.4 times the true length.
- `curve_probe_integral` multiplies by that length (readouts.rs:420). The integrated flux or energy, for example the power leaving an obstacle, therefore reads 2 to 3 times too high. On curved holes at the High preset the error is about 2 times.

**Why.** `compile_boundary_probe` sorts pieces by `parameter[0]` (1827) and interpolates from `parameter[0]` to `parameter[1]` (1868-1872). The plan stores every Right-side atom reversed (topology_plan.rs 2568-2570 and 2848-2849). The pieces therefore come out ordered by their end parameter, but each one is walked from its end back to its start. Atoms are capped at the target edge length, so any span longer than one edge has several.

**Fix.** Walk each piece from its lower to its higher parameter whatever its side. Add a test with a multi-atom Right-side probe.

#### PR-2 · High · The boundary stencil ignores the requested trace side on a transmitting span with the same region on both sides
`crates/funfern-core/src/probe.rs:984`

**What goes wrong.** On a free separator, Left and Right probes get identical stencils. Along either probe the outward normal follows whichever adjacent triangle has the lower index on each edge, so the directed flux flips sign from sample to sample and its integral partly cancels. After a carve or AMR renumbers the triangles, the sign can also flip between generations. This contradicts the architecture ("select one explicit physical trace") and the engineering log ("exactly opposite outward normals").

**Why.** `build_topology` rewrites every transmitting label to Left (908-920), and `contains_boundary_target` ignores the side for transmitting atoms. `build_with_model` takes the first triangle, in index order, that has the region and contains both edge vertices (984-990), and takes the normal from its centroid (1024-1028). A free separator, or a separator that closes off a face (the new face inherits the region it was cut from), has the same region on both sides.

**Fix.** Pick the adjacent triangle from the geometric side of the edge relative to the curve direction and the requested side, not from the region and the index order.

#### PR-3 · Medium · A recorder setup failure goes only to the status line, and the chain leaves the other recorders on old stencils that still count as current
`crates/funfern-app/src/ui/probes.rs:470`

**What goes wrong.** On an adaptation handoff to a driven generation, the point-stencil update fails. Point probes are cleared, and curve and area setup is skipped. The line recorder keeps running the old mesh's node indices against the new state and writes wrong samples, stamped with correct times, into the kept ring. The status line is the only report.

**Why.** `configure_probes` chains the three updates with `and_then` and reports errors through `self.message`, and Status events are not errors (events.rs:29-31). The bind group is rebuilt from the old stencils, and the stored upload tag lets their readbacks pass as current.

**Fix.** Configure each recorder independently. Clear any recorder whose setup failed, and report the failure through the diagnostics warning.

#### PR-4 · Medium · A failed stencil build leaves the previous mesh's recorder bound to the new generation
`crates/funfern-app/src/wave_gpu.rs:352`

**What goes wrong.** This is the recorder-side half of PR-3. When reconstruction fails, the old stencils stay installed, the probes sample unrelated nodes of the new mesh, and those samples are ingested for the whole generation.

**Why.** Every validation rejection clears the recorder's buffers first. The fallible reconstructions return through `?` with the old stencils still in place: `source.build` (352, 666), `gpu_canonical_area_contribution` (861-863) and `TemporalTableIndex::build` (304, 605, 799). The 2026-09-16 rule that "each rejection clears explicitly" was written before these fallible builds existed.

**Fix.** Clear the recorder on every error path in all three update functions.

#### PR-5 · Medium · Probe traces keep the discarded source trajectory for each handoff's admission backlog
`crates/funfern-app/src/ui/probes.rs:671`

**What goes wrong.** A point probe records at 120 Hz while the user disables a source. Admission takes about 80 ms. During that time the unedited source's samples are ingested. The target's samples for the same window are then dropped, because their times are not later than the trace's last time. The trace shows pre-edit physics followed by a jump, and spectra and transfer estimates built from it are biased. Curve and area traces behave the same way (698, 719).

**Why.** The source keeps stepping, and its recorders keep writing, while admission is pending. The target restarts at the snapshot step and writes into the kept ring, but ingestion keeps only records later than the last time, and nothing trims the trace when the handoff publishes.

**Fix.** When a handoff publishes, truncate the kept traces back to the snapshot time.

#### PR-6 · Medium · When the far-field recorder refuses a too-long delay window, the readout freezes on the old contour's frames
`crates/funfern-app/src/ui/probes.rs:497`

**What goes wrong.** With a vacuum exterior, the user drags the domain's right edge from x = 1 to 5. The delay window grows to about 10 s, past the 8.5 s limit, and the recorder refuses. The readout keeps drawing the old pattern frozen at its last time, without a red status, and the status line flashes the error once per handoff.

**Why.** `update_canonical_far_field` refuses a history longer than the ring holds, but neither the core compile nor `set_far_field` checks that limit. The error branch clears only `far_field_recording_from`. The window's status reads the compile result, which is still Ok. The architecture asks for an explicit error in place of truncated output.

**Fix.** Report the ring-capacity refusal through the far-field status the window already draws in red, or check the limit at compile time, and clear the trace.

#### PR-7 · Medium · The parameter wrap moves an end sample of a single-span open-curve probe onto the opposite tip
`crates/funfern-core/src/probe.rs:971`

**What goes wrong.** A boundary probe on a baffle drawn with two clicks reads one of its end samples at the other tip. The plotted profile's end point and the integral are wrong, and this happens where the gradient is singular.

**Why.** `build_with_model` tries `parameter ± period` and accepts the first edge that fits. `compile_boundary_probe` passes the open curve's total parameter length as its period (topology_runtime.rs:1844), and on a single-span curve the first and last edges share one label.

**Fix.** Wrap only for closed curves, as `node_vertex_at` already does.

#### PR-8 · Medium · The far-field enclosure check tests only the point source's centre
`crates/funfern-core/src/probe.rs:1136`

**What goes wrong.** A source of width 0.06 at (−0.85, 0), with the contour at |x| = 0.88, is accepted. About 31% of its Gaussian lies outside the contour. The far-field pattern then misses that share of the source, so it is too weak and has the wrong directivity, and nothing reports it.

**Why.** The check tests only the centre, with a margin of tolerance·0.25. The source drives out to 9 widths.

**Fix.** Require the source's reach to lie inside the contour.

#### PR-9 · Medium · After a handoff, the recorders' new stencil buffers can reach the render world a frame late
`crates/funfern-app/src/canonical_gpu.rs:7594`

**What goes wrong.** With a 120 Hz probe and AMR on, a handoff admitted with a backlog lets the target's first 48 steps run with no recorder bound. Each adaptation leaves a gap of about 2 samples in the trace, and nothing reports it.

**Why.** The new stencil assets are uploaded only when their AssetEvent runs. That system has no ordering against the egui pass that creates the assets. `steps_wait_for_recorders` holds steps only while a group for the current generation is compiling, not while it is missing. The PH-18 log accepted this lag only for a probe being dragged.

**Fix.** After a generation change, hold steps until the recorders for the new generation have bind groups, or order the asset systems before the egui pass.

#### PR-10 · Medium · After the step counter wraps, the line-probe shader's stride test disagrees with the host's schedule
`crates/funfern-app/src/canonical_curve_probe.wgsl:512`

**What goes wrong.** A line probe with stride 36 that runs past 2^32 total steps stops recording, and nothing reports it. Point and area recorders carry on.

**Why.** The host chooses the dispatch steps from a u64 count (canonical_gpu.rs:8155, 8194-8197). The shader then tests again against the wrapped u32 counter. Because 2^32 mod 36 = 4, every step the host chose fails that test. Unwrapping the counter on the host (RT-12) does not fix this.

**Fix.** Remove the shader's own stride test, since the host already chose the step, or pass the host's decision to the shader.

#### PR-11 · Low · A recorder pipeline that fails to compile holds every solver step forever
`crates/funfern-app/src/canonical_gpu.rs:7719`

**What:** With a probe or the far field on, a recorder shader error freezes the solver at step 0, showing 0 steps/s and no notice. One skeptic dissented: browsers do not report backend compile failures as `CachedPipelineState::Err`, so only shader-module creation errors reach this path.

**Why:** Since 569e56b any state other than Ok counts as "compiling", and Bevy keeps permanent errors in Err.

**Fix:** Skip a recorder whose pipeline is in Err, and report it through the diagnostics warning.

#### PR-12 · Low · The far-field readout's Clear button refills from the device ring at the next readback
`crates/funfern-app/src/ui/readouts.rs:1677`

**What:** Clear empties the plots for one frame, and the next readback brings back all 512 ring frames.

**Why:** Clear resets the trace's last time to −1, and each readback carries the whole ring. Commit dcf0b32 fixed this for point, line and area probes but missed the far field.

**Fix:** On Clear, resume the trace from the current simulated time, as `clear_probe_trace` does.

#### PR-13 · Low · A line probe's integral history is rescaled by the probe's current length after an edit
`crates/funfern-app/src/ui/readouts.rs:919`

**What:** Dragging a segment from length 1 to length 2 doubles the whole recorded integral history.

**Why:** One length, refreshed from the document, is applied to every frame, and records do not carry the length they were sampled over.

**Fix:** Store the length, or the integral, with each frame, or clear the trace on a geometry edit.

#### PR-14 · Low · Waterfalls give every frame an equal row, so the time axis is non-linear
`crates/funfern-app/src/ui/readouts.rs:605`

**What:** A handoff that changes the recording stride, or a dropped readback, bends space-time tracks, so a wave speed read off the diagram is wrong on one side.

**Why:** Rows are placed by frame index (605-612, 713-720), while the axis labels and the drag treat the axis as linear in time.

**Fix:** Place rows by their record times.

#### PR-15 · Low · After a Reset, the far-field "Recording the delay window" label uses the previous run's step count
`crates/funfern-app/src/ui/probes.rs:493`

**What:** After a 60 s run and a Reset, the label reads 0% for about 60 s while the plots already draw.

**Why:** `configure_probes` runs before `completed_steps` is refreshed (runtime.rs:981).

**Fix:** Take the recording start from the installed clock.

#### PR-16 · Low · Line-probe normals come from the document, but the stencils come from the active generation
`crates/funfern-app/src/ui/probes.rs:431`

**What:** Pressing "Swap ends" during an in-flight upload records flux with its sign flipped relative to the sample order. Those frames stay in the trace.

**Why:** `configure_probes` looks the target up in the editor document instead of in `active.probe_definitions`.

**Fix:** Use `active.probe_definitions`.

#### PR-17 · Low · An exterior with a named loss channel is refused as "non-uniform" rather than as lossy
`crates/funfern-core/src/probe.rs:1167`

**What:** A constant electric loss on the ambient medium produces "requires a uniform exterior medium", which sends the user after the wrong property.

**Why:** `Material::uniform` returns None for any material with laws, and the LossyExterior check looks only at legacy damping.

**Fix:** Check named loss channels for LossyExterior first.

### Core numerics and materials

#### NUM-1 · High · A handoff takes a lane's carrier from the first lane with the same kind and phase
`crates/funfern-core/src/canonical_temporal.rs:507`

**What goes wrong.** Pump a material's mass row at 1 Hz and its stiffness row at 1.5 Hz, both at the default phase 0. At the first handoff at time T, from AMR or any edit, the stiffness carrier is re-anchored to the mass carrier. The stiffness modulation jumps by 2π·0.5·T mod 2π, a full inversion at T = 1 s, and the jump repeats at every handoff. The reflectionless preset does the same after one row's frequency is edited. The CPU twin and the device share the rule, so the fixtures cannot see it.

**Why.** `carries_to` maps each lane to the first source lane with a matching signature. The signature is the drive kind and its authored phase, and leaves out the frequency on purpose. `carried_onto` (257-264) and the shader (canonical_transfer_runtime.wgsl:366-371) then re-anchor from that lane. The comment's premise, that lanes with one signature "cannot be told apart by phase", holds only when their frequencies are equal.

**Fix.** Prefer the same lane index when its signature matches, and fall back to the first match only for real moves. Apply the same rule in the shader.

#### NUM-2 · Medium · A driven CPU handoff checks the handed fluxes against the target's authored runtime
`crates/funfern-core/src/canonical_temporal.rs:4152`

**What goes wrong.** A defocusing Kerr law with a Switch alternate is refused as OutsideDomain at a state that is valid under the switched coefficient. This affects only the CPU reference used by tests and examples. The device checks against the right runtime.

**Why.** `from_handed` calls `new_at`, which inverts under `initial_runtime()` (3857). The carried runtime is installed only afterwards (4161).

**Fix.** Validate the fluxes under the carried runtime.

#### NUM-3 · Medium · The temporal primary rate uses u = Q/m in the van der Pol loss beside a field law
`crates/funfern-core/src/canonical_temporal.rs:1664`

**What goes wrong.** On a van der Pol medium with Kerr on the mass row, the AMR snapshot's velocity at self-oscillating nodes is off by about 20 to 70% of the gain term. That skews the field-law envelope and the wall residual the estimator reads.

**Why.** The rate is written as (β + α/m²·Q²)·Q. Since 36f570f the step uses α(u/Q)², with u taken from the node's own inverse, and that field is already computed at line 1640. The rate test has no case with both van der Pol and Kerr.

**Fix.** Use α·u² with the node's own field, and add a case with both laws to the rate test.

#### NUM-4 · Medium · A Switch blend can round just past [0,1], after which every evaluation errors
`crates/funfern-core/src/material_law.rs:1114`

**What goes wrong.** On the host, `begin(true,0.1,0.2).begin(false,0.3,0.2)` stores a start of 1.0000000000000013, and every later evaluation returns InvalidValue. In the app, reversing a Switch in the last 0.4% of its ramp stores an f32 start just past 1. `restored()` then fails, so until the next Switch:
- the indicator shows the authored state;
- the energy view uses the authored runtime;
- AMR skips its estimate.

Nothing reaches diagnostics.

**Why.** `begin` stores the evaluated blend without clamping it. Smootherstep can round above 1 near the end of the ramp. `valid()` and `restored()` (1045) require [0,1] exactly. The device has the same arithmetic (canonical_wave.wgsl:529-541, 1651).

**Fix.** Clamp the start blend to [0,1] on the host and on the device.

#### NUM-5 · Medium · A skin switch rescales legacy damping by 1/ρ, but the solver reads legacy damping as a rate
`crates/funfern-core/src/wave.rs:422`

**What goes wrong.** A v22 Mechanical material with ρ = 4 and damping 0.4 becomes, in TM, ε = 1, μ = 4 and damping 0.1, and decays 4 times more slowly. The round trip back restores 0.4, so the round-trip test passes. No warning appears.

**Why.** `convert_material` uses α = d/ρ and d = αμ (422, 428), which kept the rate under the old scalar operator. The canonical solver treats damping as the primary-field rate in every skin (canonical_wave.rs:5029-5031). The unit test at 2003 encodes the old meaning.

**Fix.** Copy legacy damping unchanged across skins and update the test, or settle what v22 damping means and make the solver and the conversion agree.

#### NUM-6 · Medium · Switching medium refuses with ParameterLimit because the incoming parameters are added first
`crates/funfern-core/src/law_presets.rs:826`

**What goes wrong.** A van der Pol medium plus one user parameter is switched to Travelling modulation. The fifth push hits the 8-parameter limit, although the finished material would hold 6.

**Why.** The three parts are applied in sequence, and each retires only its own outgoing parameters before checking the limit.

**Fix.** Retire all outgoing parameters before adding any incoming ones.

#### NUM-7 · Medium · A van der Pol gain on the non-primary channel becomes a constant loss when a medium is chosen
`crates/funfern-core/src/law_presets.rs:839`

**What goes wrong.** Switching the TM emitter scene to TE leaves the disk reading Custom, and its preparation fails. Re-picking a medium:
- turns the E-row gain into a passive loss at the gain's rate;
- orphans the threshold parameter;
- adds more parameters on each TM/TE toggle, until ParameterLimit.

**Why.** `apply_self_oscillation` recognises van der Pol only on the skin's primary loss, and TM↔TE conversion clones the material unchanged. The final loop then makes the channel constant and keeps its base rate, which for van der Pol is a gain.

**Fix.** Recognise and remove a van der Pol channel on either loss row together with its parameters, or convert the channel on TM↔TE.

#### NUM-8 · Medium · The trace eigensolve's seed overflows u64 and panics in debug builds
`crates/funfern-core/src/trace_eigen.rs:1041`

**What goes wrong.** The README's debug commands (`cargo run`, `scripts/trunk serve`) and a non-release `cargo test -p funfern-core` all panic while preparing the default scene, which has second-order outgoing walls. On wasm the app aborts. CI and the gate run only release builds.

**Why.** The seed multiplies by a constant above 2^63 with a plain `*`. wall_sweep.rs:414 uses `wrapping_mul` with the same constant.

**Fix.** Use `wrapping_mul`, and consider adding one debug-profile test run.

#### NUM-9 · Medium · The CPU grid filter's contraction claim fails beside a nonzero pin
`crates/funfern-core/src/canonical_wave.rs:2696`

**What goes wrong.** With a driven Dirichlet side next to quiet free nodes, `apply_grid_filter` raises the energy and returns InvalidState, and `grid_filter_calibration` panics on the unwrap. This is latent, because the calibration scenes have no pins, and it affects only the CPU reference.

**Why.** Only the free rows are updated, but their second difference reads the pinned field. The energy change therefore has a pin-coupling term with no fixed sign, and the check at 2709 has no allowance for it.

**Fix.** Filter only the free subspace, or book the pin coupling as a prescribed exchange.

#### NUM-10 · Medium · Kerr and saturable presets are not recognised after a Mechanical↔EM skin change
`crates/funfern-core/src/law_presets.rs:1779`

**What goes wrong.** A TM Kerr medium switched to Mechanical reads Custom, and its χ slider can only be reached in Advanced.

**Why.** The conversion correctly swaps the rows, but the Kerr and saturable presets exist only on the mass row. The skin-change test checks conversion and the round trip, never identification.

**Fix.** Add stiffness-row field presets or match field presets on either row, and assert identification in the test.

#### NUM-11 · Low · `estimated_bytes` leaves out the dense trace inverse
`crates/funfern-core/src/canonical_wave.rs:4390`

**What:** The factor's memory is under-reported by O(T²): about 8.7 MB at 1,042 trace nodes reads as kilobytes. The "skipped the inverse" assertion at 7955 compares two values that are always equal.

**Fix:** Count `direct` in the estimate, and rely on the `direct_inverse.is_none()` check for the skip.

#### NUM-12 · Low · `stable_difference` drops real gradients below 1.8e-15 whenever the field is below 1
`crates/funfern-core/src/canonical_wave.rs:6160`

**What:** In a scene authored at small amplitude, the CPU reference stays frozen while the f32 device propagates normally.

**Why:** The threshold has a `.max(1.0)` floor, but its comment asks only for removing roundoff. The copy at canonical_temporal.rs:6909 has the same floor.

**Fix:** Make the threshold relative.

#### NUM-13 · Low · The runtime layout ignores the two loss lanes, so a frequency edit on a loss drive jumps its carrier
`crates/funfern-core/src/canonical_temporal.rs:417`

**What:** Editing an electric-loss pump's frequency mid-run jumps its carrier, while coefficient drives stay continuous.

**Why:** `CanonicalRuntimeLayout::of` visits only coefficient samples, so lanes 2 and 3 are never carried.

**Fix:** Include the loss samples' drives in the layout, on the host and in the device packing.

#### NUM-14 · Low · `bernoulli_map` drops the linear rate β for a negative duration
`crates/funfern-core/src/canonical_temporal.rs:5944`

**What:** A backward step of a generation whose only loss is van der Pol returns Ok with a state that is not the inverse.

**Why:** The small-rate guard is true for any negative duration.

**Fix:** Take the absolute value of β·τ in the guard.

#### NUM-15 · Low · Scene-path coefficient sampling bypasses `Material::evaluate_static`
`crates/funfern-core/src/wave_quadratic.rs:1175`

**What:** Legacy `assemble_scene` drops constant loss channels and assembles materials with laws as static media, so it disagrees with the topology path.

**Fix:** Route the Scene provider through the shared library evaluator.

#### NUM-16 · Low · The Scene-path outer boundary samples the background material where another region meets the side
`crates/funfern-core/src/wave_quadratic.rs:2083`

**What:** A legacy scene with region 2 against an outgoing side absorbs with the background's impedance. One skeptic dissented, judging the impact nil because the app uses the topology path.

**Fix:** Look up the region of the triangle that owns each outer edge.

#### NUM-17 · Low · A spatially varying φ⁴ λ or van der Pol threshold is never checked at the nodes
`crates/funfern-core/src/material_law.rs:775`

**What:** λ = "16*x" is accepted. Nodes with λ < 0 run out to the amplitude bound, and a threshold of 0 at a node gives an infinite α.

**Why:** `passes()` defers the check "to the node points during assembly", but the compiler and `pack_restoring` never make it.

**Fix:** Check the sign of the evaluated λ and threshold at each node during compile.

#### NUM-18 · Low · `stiffness_speed_ratio_range` uses √ĥ where the stiffness row gives 1/√ĥ
`crates/funfern-core/src/material_law.rs:1967`

**What:** A pump of depth 0.3 reports (0.837, 1.140) where the medium runs at (0.877, 1.195). The function has no callers yet.

**Fix:** Return (1/√high, 1/√low).

#### NUM-19 · Low · `SplinePatch::project` leaves the density out of the right-hand side
`crates/funfern-core/src/spline_patch.rs:1592`

**What:** With ρ = 4, projecting the constant 1 returns 0.25.

**Fix:** Weight the load by the density, as `project_parametric` does.

#### NUM-20 · Low · `frequency_bounds` drops cross terms for condensed corner DOFs
`crates/funfern-core/src/spline_patch.rs:964`

**What:** Running with `--condense 3 --scale 4` over-scales the condensed corners.

**Fix:** Sum the gradients for each condensed node before squaring.

#### NUM-21 · Low · `refined()` silently drops a C0 knot
`crates/funfern-core/src/spline_patch.rs:1254`

**What:** On patches with a C0 knot, `refine_field` is no longer exact, and nothing signals it.

**Fix:** Carry interior knot multiplicities into the refined basis, or refuse to refine such a patch.

#### NUM-22 · Low · The time-crystal "Edge sharpness" slider offers 0, which the validator rejects
`crates/funfern-core/src/law_presets.rs:120`

**What:** Dragging the slider to its end makes Apply fail with a generic error.

**Fix:** Set the minimum to 1e-6, as the Advanced editor does.

#### NUM-23 · Low · A parameter rename rewrites a number's exponent suffix
`crates/funfern-core/src/material.rs:1179`

**What:** Renaming `e1` to `e3` turns the literal 1e1 into 1e3 without a word.

**Why:** `replace_identifier` starts an identifier at a letter even when it directly follows a digit.

**Fix:** Scan numeric literals, exponent included, as single tokens.

#### NUM-24 · Low · A point source leaks through a hole along a chord that runs through two rim vertices
`crates/funfern-core/src/source_reach.rs:134`

**What:** A node in the hole's shadow is driven at about 1.8% of the peak. This needs exact collinearity, and the CPU and the device share the code.

**Why:** A crossing counts only with strict signs, and a rim vertex is a single node, not a split point.

**Fix:** Treat a segment that enters the excluded side through a rim vertex as blocked.

### Meshing, AMR and state transfer

#### MESH-1 · High · Renumbered trace IDs can point a separated node's candidates at the wrong place, and the node then takes the other side's value
`crates/funfern-core/src/transfer.rs:255`

**What goes wrong.** A hole H is drawn first and a closed reflecting wall W later, with a cavity mode ringing inside W. Reshaping H so its outline gains m ≥ 2 vertices, or switching H's behaviour, shifts every trace ID on W by 2m. At each sample vertex of W, one of the two coincident copies takes the other side's value. Cavity values appear outside a reflecting wall, or zeros inside it, at every such handoff.

**Why.** `preferred_trace_triangles` intersects the node's curve-side candidates with the source triangles that carry the target's trace ID. Trace IDs are local to a snapshot and handed out in sequence, and a carve or retraced reuse gives the target new IDs. Only an empty intersection falls back. A non-empty intersection that does not contain the node sends `locate` (930) straight to the unrestricted pass, skipping the curve-side set that the comment, the log and the architecture all name as the fallback.

**Fix.** When the narrowed set does not contain the node, retry with the full curve-side set before the unrestricted pass.

#### MESH-2 · Medium · A paired collapse closes a separated span whose two sides share both trace vertices
`crates/funfern-core/src/mesh/amr.rs:1261`

**What goes wrong.** A straight free baffle 0.1 long, at the default maximum edge, gets a coarsening transaction. Verification then finds a non-manifold edge, and the whole transaction is thrown away, including every other collapse in the mesh. This repeats on every cycle, so coarsening never succeeds for that scene.

**Why.** The two sides' plans are built and link-checked separately (1534). After both apply, edges [A,B] and [B,A] close the slit. The mesher forces two pieces on such spans, but AMR does not.

**Fix.** Refuse a paired collapse that would leave a single piece per side when the two sides share both end traces.

#### MESH-3 · Medium · Placeholder slots in the primary map's rows give a newly prescribed node source node 0 as its donor
`crates/funfern-app/src/canonical_gpu.rs:529`

**What goes wrong.** In TM with an electric-wall side, which makes node 0 prescribed, the user switches another side to a harmonic Dirichlet drive. The new carriers inherit node 0's phase, so the driven side runs ω·t_handoff out of phase with the reference.

**Why.** Single-donor rows report 7 sources, six of them zero-weight placeholders at node 0 (canonical_transfer.rs:196-222). `from_primary_transfer` keeps any prescribed source slot, the placeholders included.

**Fix.** Skip zero-weight or placeholder slots when choosing a prescribed donor.

#### MESH-4 · Medium · The coarsening confirmation streak survives a mesh commit
`crates/funfern-app/src/ui/amr.rs:310`

**What goes wrong.** After the first coarsening, every new mesh is coarsened on a single estimate taken right after a transfer. A streak left from the old mesh carries across edits too. The result is extra handoffs, each with interpolation error.

**Why.** The commit handler (runtime.rs:712-743) no longer resets `amr_coarsen_streak`. The reset was dropped in eddad77.

**Fix.** Reset the streak when a mesh is committed.

#### MESH-5 · Medium · A retraced mesh keeps its `mesh_revision`, so a stale adaptation fails at the handoff as an AMR error
`crates/funfern-app/src/topology_runtime.rs:315`

**What goes wrong.** The user switches a wall from Dirichlet to Neumann while an adaptation is running. The adaptation passes the stale check and then fails with "Adapted mesh does not match the active topology". `amr_error` lights the diagnostics warning, and the cycle is lost.

**Why.** `retraced()` changes `geometry_revision` but keeps `mesh_revision`, and the stale check (amr.rs:163-178) compares only `mesh_revision`.

**Fix.** Include `geometry_revision` in the stale check.

#### MESH-6 · Medium · For separated loops that touch at one vertex, the cycle's starting half-edge decides whether the sectors merge or meshing stalls
`crates/funfern-core/src/mesh/topology_plan.rs:674`

**What goes wrong.** Two closed curves with reflecting spans share one authored vertex J. Which of two failures occurs depends on the drawing direction:
- **Merge:** the two sectors become one node, coupling wedges across the walls. AMR later rejects that mesh (amr.rs:389).
- **Stall:** ear clipping stalls and the mesh fails.

**Why.** The same-point merge at 674-683 runs on every step. Only `split_slit_trace_vertices` undoes it, and that function seeds only slit steps (1445).

**Fix.** Limit the merge to stretches that contain a slit, or split merged sectors whether or not a slit is present.

#### MESH-7 · Medium · Coarsened atoms can move a boundary onto or across a neighbour the arrangement kept apart
`crates/funfern-core/src/mesh/topology_plan.rs:2325`

**What goes wrong.** Two cases are reachable:
- A disc 3.5e-4 inside a ring wall: the ring's merged chord can cross into the disc, giving an overlapping or stalled mesh.
- A thin cap cut off a circle: the cap face can collapse to a two-trace cycle.

**Why.** Merging accepts a run within the curve tolerance (5e-4) of its own chord, but the arrangement only guarantees 2e-4 separation. `coarsened` never re-checks that each cycle has at least 3 traces, and nothing falls back to the fine plan.

**Fix.** Bound merges by the arrangement's separation or check merged chords against nearby boundaries, keep at least 3 traces per cycle, and fall back to the fine plan if meshing fails.

#### MESH-8 · Medium · Slit recovery can move frozen rim vertices during a carve
`crates/funfern-core/src/mesh.rs:1591`

**What goes wrong.** A baffle added or dragged inside a finer adapted region can snap or push a rim vertex. Kept triangles change shape, possibly into slivers, while the carve reports them as unchanged.

**Why.** Neither `relocatable_onto` nor `repair_constraint_edge_slivers` (1863/1895) checks the frozen-vertex and frozen-triangle bounds.

**Fix.** Exclude frozen vertices, and vertices whose fan contains frozen triangles, from both moves.

#### MESH-9 · Medium · A transmitting crossing near a hidden sample point is refused as a "touch with no junction"
`crates/funfern-core/src/topology.rs:1073`

**What goes wrong.** A straight transmitting curve crossing a circle is refused at about 7% of positions per crossing, so the draft flickers between valid and invalid while the user drags. Resizing the domain changes the sampling and can flip a valid scene to invalid.

**Why.** The crossed curve's neighbouring sample segment fails the endpoint-distance check. The graph builder also merges a crossing that lies within 0.25·tol of a sample.

**Fix.** Exempt near contacts explained by an accepted proper crossing of the adjacent segment, snapping the crossing to the sample.

#### MESH-10 · Medium · The barycentric tolerance is absolute, so small elements reject points on their own edge
`crates/funfern-core/src/transfer.rs:1020`

**What goes wrong.** With AMR's minimum edge at 0.005 near walls, separated midpoints take the other side's value, and new hole-rim points are filled by extension instead of being interpolated.

**Why.** The weight tolerance is 64ε·max(1, |coord|), while the weight error from rounding grows as 1/h.

**Fix.** Scale the tolerance with the element's size.

#### MESH-11 · Medium · Separated-side pairing is decided per label, not per atom
`crates/funfern-core/src/mesh/amr.rs:1337`

**What goes wrong.** A drum is half filled by a hole. Refinement on the one-sided arc fails the whole transaction. Coarsening there either fails or, on the Right side, silently never happens. It repeats on every cycle.

**Why.** `paired_label` returns a partner whenever any atom on the span has the opposite side.

**Fix.** Decide pairing per atom.

#### MESH-12 · Medium · The capacity guard allows one vertex too many for a paired split
`crates/funfern-core/src/mesh/amr.rs:1711`

**What goes wrong.** At max − 1 vertices, a split on a separated trace fails the whole transaction instead of committing a partial mesh, and refinement stops committing from then on.

**Why.** The guard allows one more vertex, but a paired split adds two.

**Fix.** Check for room for two vertices before a paired split.

#### MESH-13 · Medium · A face that becomes active behind a separated wall copies the other side's field onto its wall nodes
`crates/funfern-core/src/transfer.rs:561`

**What goes wrong.** One sequence that reaches it: turn a hole into a reflecting-walled subdomain, turn it back into a hole, then press undo, which is a handoff. The new face's wall nodes take the outer field while its interior stays zero. The resulting ring radiates into the face, and no total correction applies.

**Why.** The exact-copy shortcut assumes a lone source node is never one side of a separated wall. The fallback then takes any containing triangle.

**Fix.** Refuse donors from the other side of a separated curve in both paths.

#### MESH-14 · Low · The refill quality floor follows the previous mesh's worst angle
`crates/funfern-core/src/mesh/carve.rs:1558`

**What:** Each accepted sub-minimum element halves the next carve's floor, so slivers build up over a drag session and the one-ring-wider retry never fires.

**Fix:** Base the floor on the minimum angle, or on the worst angle among kept triangles only.

#### MESH-15 · Low · The driven indicator supplement leaves the outgoing wall's pole store out of `element_energy`
`crates/funfern-core/src/canonical_temporal.rs:1573`

**What:** Wall elements read less energy than the fixed path gives the same state, so the global indicator jumps when a drive switches on.

**Fix:** Add the same Σ½z² share as the fixed supplement (indicator.rs:639-647), and compare `element_energy` in the parity test.

#### MESH-16 · Low · Local-repair hull checks use the right-sided tangent at C0 corners
`crates/funfern-core/src/mesh/adaptation.rs:1068`

**What:** A local repair touching a polygon hole fails with FixedPatchBoundary on every attempt and falls back to a full rebuild. This is the legacy path; the app never reaches it.

**Fix:** Use `derivative_left` where an edge approaches a knot from the left, as the samplers have since 4aaacb3.

#### MESH-17 · Low · A resolution change after a non-geometric edit keeps the old topology revision
`crates/funfern-app/src/topology_runtime.rs:436`

**What:** A remesh of unchanged geometry skips the component-total correction.

**Why:** The plan is re-coarsened from the newer compile while the token keeps the old revision.

**Fix:** Keep the plan's and the token's revisions consistent.

#### MESH-18 · Low · The scalar estimator rejects a topology thin gap between two materials as InvalidMesh
`crates/funfern-core/src/indicator.rs:2258`

**What:** The two sides compute different springs, so the partner check fails. The app is unaffected, because it always attaches a canonical supplement.

**Fix:** Compute the pair's spring from both sides' materials, as the operator does.

#### MESH-19 · Low · Constraint-edge recovery refuses every flip whose new diagonal still crosses the segment
`crates/funfern-core/src/mesh.rs:1982`

**What:** A valid baffle scene fails to mesh with "could not recover an internal-boundary segment".

**Fix:** Allow flips that keep crossing on convex quads, Sloan style, with a progress bound.

#### MESH-20 · Low · A junction at an outer-domain corner is always refused
`crates/funfern-core/src/topology.rs:1108`

**What:** The editor clamps a click to a corner, and the result is refused as "touch with no junction".

**Fix:** Accept a corner vertex for both adjacent sides, or keep the editor's clamp off exact corners.

#### MESH-21 · Low · Legacy validation picks a nested loop's container by smallest perimeter
`crates/funfern-core/src/geometry.rs:2059`

**What:** With a star inside a circle, a valid scene is rejected, or a wrong exterior is accepted.

**Fix:** Pick the innermost container from the containment matrix.

#### MESH-22 · Low · The carve's size field is installed after the changed atoms are expanded
`crates/funfern-core/src/mesh/carve.rs:691`

**What:** A straight boundary moved through an adapted band is expanded at the coarse target.

**Fix:** Install the size field before the Expand phase.

#### MESH-23 · Low · Tiny-island deletion removes small components anywhere in the mesh
`crates/funfern-core/src/mesh/carve.rs:932`

**What:** A small cell far from the edit is carved on every repair, and its separated curves are banded too.

**Fix:** Delete only islands that touch the removed band.

#### MESH-24 · Low · The local-repair compatibility check ignores `material_interfaces`
`crates/funfern-core/src/mesh/adaptation.rs:366`

**What:** Adding the first open interface returns the stale mesh as a successful local repair. This is the legacy path.

**Fix:** Compare interfaces, junctions and regions, and fall back to a full mesh when they differ.

#### MESH-25 · Low · "boundary repair left the local patch" is classified as terminal
`crates/funfern-core/src/mesh/adaptation.rs:194`

**What:** The repair never retries with a larger patch, and the fallback is labelled "other".

**Fix:** Classify this failure as a patch-boundary hit, preferably with a typed error rather than a substring match.

### Scene editor

#### ED-1 · High · Any deletion re-walls every free transmitting curve in the scene
`crates/funfern-app/src/topology_editor.rs:1855`

**What goes wrong.** A free baffle switched to Transmit is a supported way to hold a wall switched off without deleting it. Deleting an unrelated hole demotes that baffle to REFLECTING, and it scatters the field again. The only report is "1 attached curve promoted to baffles", which also misdescribes the curve. A transmitting separator with a free end becomes a wall the same way.

**Why.** `plan_removal` runs `promote_freed_curves` over every curve in the scene (4661). Its premise, that the compiler rejects free transmitting ends, was removed by 31c0d7e, and core and editor tests now assert that state is valid. `weld_endpoint` (1410) and the always-reflecting cut pieces (1779) rely on the same stale premise.

**Fix.** Drop the forced promotion, or limit it to curves the edit actually cut where a rule still requires it. Update the comments at 1666, 3948 and 4657.

#### ED-2 · Medium · The breakpoint end attachment uses a node index that the start's span split has already shifted
`crates/funfern-app/src/topology_editor.rs:1124`

**What goes wrong.** On a circle, a curve drawn from the rim between dots 0 and 1 to dot 3 ends on the old dot 2. If the start lies in the span just before the clicked dot, the result is a loop the user never drew.

**Why.** Splitting the span at the start shifts every later node index, and the Breakpoint index is not remapped. LooseEnd already gets this remap (d96f812).

**Fix.** Remap Breakpoint indices past the split, as for LooseEnd.

#### ED-3 · Medium · Moving an outer junction along its side does not carry Outer face anchors with it
`crates/funfern-app/src/topology_editor.rs:2549`

**What goes wrong.** A separator moved from bottom fraction 0.3 to 0.7, past the background anchor at 0.5, gives DuplicateFace and an invalid draft.

**Why.** The vertex's fraction is recomputed, but Outer anchors keep their old fraction. `resize_outer_domain` remaps anchors, but drags and rigid moves do not.

**Fix.** Remap Outer anchors on that side whenever an outer vertex moves, reusing the resize logic.

#### ED-4 · Medium · An open curve that starts and ends on the same loose end reshapes a curve or is refused misleadingly
`crates/funfern-app/src/topology_editor.rs:1144`

**What goes wrong.** Either the user sees "Both curve ends must be free to close the loop", or a closed loop is committed whose shape differs from both the drawing and the original curve.

**Why.** The second join takes the closing branch without checking that the two endpoints differ. `weld_endpoint` does check this.

**Fix.** Refuse, or close on the drawn curve itself, when both attachments name the same loose end.

#### ED-5 · Medium · An exterior merge that keeps a subdomain's material resets region 1's frame to world
`crates/funfern-app/src/topology_editor.rs:4870`

**What goes wrong.** A region with a frame centred at (0.5, 0) is merged with the exterior and its material kept. Its source moves to the domain centre and its anisotropy rotates back to world, without notice.

**Why.** `settle_merge` copies the material and the source but sets the frame to world. The background only has to have a World attachment.

**Fix.** Keep the chosen region's origin and angle, with a World attachment.

### UI, display and gallery

#### UI-1 · High · The profile-placement angle round trip writes 1-ulp edits with no user input
`crates/funfern-app/src/ui/materials.rs:358`

**What goes wrong.** On the gallery's Rotated crystal, dragging the frame gizmo through 64 to 115° does the following:
- A mid-drag commit closes the gizmo's transaction, so Escape can no longer cancel the drag.
- Later frames push undo steps.
- After release, Undo restores a non-fixed angle, the panel commits again and the redo history is cleared. The drag can never be undone while the region stays selected.

Each spurious commit also revalidates the scene and hands off.

**Why.** The panel converts radians to degrees and back (350, 358), which is not the identity: at least 10.5% of angles in several bands are not fixed points. The exact comparison at 378 then calls `set_region_frame`. `history_shortcuts` runs before the side panel, so an Undo is immediately undone.

**Fix.** Write the angle back only when the DragValue reports a change, or compare in degrees.

#### UI-2 · Medium · Neumann "Fire now" and pulse edits fire at the live time, but every Neumann edit takes a full handoff
`crates/funfern-app/src/ui/signals.rs:52`

**What goes wrong.** On a 34k-DOF scene, the handoff takes 0.4 to 0.8 s. The new generation joins 0.15 to 0.55 s into the burst, so the pulse is truncated.

**Why.** `takes_handoff` is true only for Field. A Neumann signal is stored in the boundary data, and changing it makes `operator_scene_eq` false, which forces a FullHandoff.

**Fix.** Time Flux edits at the handoff time, or make Neumann signal edits a live patch.

#### UI-3 · Medium · The painted field and the energy readout use the authored runtime after handoffs and rebases
`crates/funfern-app/src/ui.rs:2445`

**What goes wrong.** Take a thrown Switch (ε from 1 to 4) with AMR on:
- After each handoff, the painted E_z is 4 times too large for 0.25 to 0.5 s.
- After each rebase, the same happens until the next snapshot.
- A straddling snapshot makes the energy readout 4 times too high. `amr_energy_peak` latches that value, which raises the dormant threshold for good.

**Why.** `temporal_view` falls back to the authored runtime whenever `accepted_material_runtime` is None, which happens after every generation change and across a rebase. The device runs the carried runtime. AMR waits in this case; the display does not.

**Fix.** Seed the display's runtime from the handoff, hold readouts until the runtime and clock align, and never feed the peak from an unaligned reading.

#### UI-4 · Medium · Profile-placement and volume-source DragValues commit an undo step and a revalidation on every frame of a drag
`crates/funfern-app/src/ui/materials.rs:378`

**What goes wrong.** About two seconds of scrubbing evicts the user's earlier undo history.

**Why.** Each frame runs begin, changed and commit. The colour button was fixed by staging its value until release.

**Fix.** Stage the value until the pointer is released, as the colour button does.

#### UI-5 · Medium · The nonlinear-strength readout shows "+0.00%" for defocusing laws
`crates/funfern-app/src/ui/materials.rs:1353`

**What goes wrong.** With χ₂ = −2, the density falls by 50% while the readout says +0.00%. Inverted (Divide) rows show the wrong sign.

**Why.** The strength is seeded at 0 and only ever raised with max (canonical_temporal.rs:2740-2771).

**Fix.** Track the signed departure with the largest magnitude.

#### UI-6 · Medium · Streamline lattice filter keys ignore the lattice spacing
`crates/funfern-app/src/ui/paint.rs:849`

**What goes wrong.** After one zoom step, the inner cells continue other locations' averages, so the old flow pattern appears scaled by √2. While the run is paused this never relaxes.

**Why.** The key encodes only the column and row, and the filter's owner does not include the spacing.

**Fix.** Include the spacing in the key, or clear the filter when the spacing changes.

#### UI-7 · Medium · "Fire now" on the non-primary loss row of a legacy-damping material drops the primary loss
`crates/funfern-app/src/ui/law_editor.rs:123`

**What goes wrong.** For a v22 material with damping 0.2, firing a stiffness-row loss drive commits damping 0 with no primary channel. The density loss disappears from the solver, and Revert loses it permanently.

**Why.** `with_fired_drive` copies the zeroed damping but not the primary channel that `adopt_legacy_damping` created.

**Fix.** Commit the adopted primary channel together with the fired one.

#### UI-8 · Medium · The diagnostics log and its marker stop recording during a video recording or snapshot
`crates/funfern-app/src/ui/diagnostics.rs:157`

**What goes wrong.** An adaptation error that appears and clears during a recording leaves no log entry and no marker.

**Why.** `record_events` runs only inside `diagnostics_window`, which is skipped while capturing (ui.rs:959-962).

**Fix.** Record events on every frame, and hide only the window.

#### UI-9 · Medium · The step-ceiling line and the nonlinear-strength readout name the wrong coefficient in TE
`crates/funfern-app/src/ui/diagnostics.rs:1028`

**What goes wrong.** In TE, an ε pump reads "Permeability μ falls to 0.80×", and an ε Kerr law reads +0.00%.

**Why.** The primary floor is always paired with the mass row, but TE swaps the rows (canonical_temporal.rs:6626-6654). materials.rs:1339-1342 makes the same assumption.

**Fix.** Map primary and complementary to rows through the skin's role table.

#### UI-10 · Medium · Switching a source or Neumann side to Pulse fires at the live time, but the switch always takes a handoff
`crates/funfern-app/src/ui/signals.rs:236`

**What goes wrong.** Switching a continuous source to Pulse cuts the start of the first burst.

**Why.** `forcing_sparse_layout_eq` requires the same pulsed flag on both sides. The editor test asserts the wrong premise.

**Fix.** Use the handoff time when the pulsed flag changes, and fix the test.

#### UI-11 · Medium · `fire_times` takes its handoff lead from the last commit of any kind
`crates/funfern-app/src/ui/signals.rs:101`

**What goes wrong.** After a live source patch or a probe toggle, the next gate or Dirichlet fire starts 0.1 to 0.5 s early.

**Why.** `commit_in_place` also calls `record_handoff` (runtime.rs:329), with near-zero pack and upload times.

**Fix.** Record handoff durations only for commits that install a new generation.

#### UI-12 · Medium · After a scene change, the previous scene's arrows are drawn and re-seed the exposure peak
`crates/funfern-app/src/ui/viewport.rs:239`

**What goes wrong.** The old arrows flash over the new scene. If the new scene's flow is quieter, its arrows stay faded for 16 s or more, or vanish entirely beyond a ratio of about 500.

**Why.** The two overlay layouts and the overlay display are not cleared on a scene change, and the layout match never checks which scene it belongs to.

**Fix.** Clear both layouts on a scene replacement.

#### UI-13 · Medium · Reset restarts the exposure while the outgoing field is still on screen, and the test meant to guard this never runs Reset
`crates/funfern-app/src/ui/exposure.rs:456`

**What goes wrong.** A field at 6% of its peak is repainted at about 76% for roughly 0.1 s, a bright flash of a field that was nearly dark.

**Why.** The Reset path restarts the scale at request time (runtime.rs:797), and the old picture keeps painting until the new generation arrives. The test calls `set_document` and never processes the Reset flag.

**Fix.** Restart the scale when the zeroed picture arrives, and make the test go through the Reset path.

#### UI-14 · Low · The ring resonator description claims "ten times its field", but the measured quantity is squared
`crates/funfern-app/src/topology_examples.rs:338`

**What:** The measured field ratio is about 3.3. **Fix:** Say "ten times the energy".

#### UI-15 · Low · The disordered-crystal description cites the photonic crystal's "fifty rods" and an 80% figure
`crates/funfern-app/src/topology_examples.rs:270`

**What:** The gallery's photonic crystal now has 25 rods and gives 0.82 field transmission. **Fix:** Reword the description for the current scene.

#### UI-16 · Low · The biconvex lens uses the platform's atan2
`crates/funfern-app/src/topology_examples.rs:1467`

**What:** This breaks the rule that gallery geometry is portable. It is not proven that the browser's result differs for this input. **Fix:** Use `portable_acos`.

#### UI-17 · Low · A Formula→Constant→Formula round trip panics when the constant prints longer than 256 bytes
`crates/funfern-app/src/ui.rs:1270`

**What:** Typing 1e-300 and switching back to Formula crashes the app. **Fix:** Format the constant with an exponent, and handle the parse error instead of unwrapping.

#### UI-18 · Low · The constant editor silently rewrites valid out-of-range values as soon as it is drawn
`crates/funfern-app/src/ui.rs:1296`

**What:** A loaded source profile of −1 is clamped to 0 when its region is selected. **Fix:** Turn off clamping of existing values, and check the ceiling on the Formula→Constant switch.

#### UI-19 · Low · The painted field silently falls back to linear Q/M when the field-law map fails
`crates/funfern-app/src/ui.rs:2615`

**What:** The fallback paints a different field from the one the solver steps, and nothing is reported. One skeptic dissented: the stated Kerr and saturable trigger cannot happen. The fallback remains for bounded laws. **Fix:** Report the failure through the diagnostics warning.

#### UI-20 · Low · Weld target dots leave out the dragged curve's own breakpoints, which the weld still snaps to
`crates/funfern-app/src/ui/weld.rs:92`

**Fix:** Draw the targets `weld_hit` accepts, and update its stale doc comment.

#### UI-21 · Low · Range clamps rewrite simple-view preset values and the volume-source profile when drawn
`crates/funfern-app/src/ui/materials.rs:1285`

**What:** A φ⁴ bound of 0.8 becomes 1.0 when the user leaves Advanced. Apply and Revert light up, region selection is refused, and the next Apply commits 1.0. **Fix:** Turn off clamping of existing values.

#### UI-22 · Low · The low-pass advances a returning key's stale history
`crates/funfern-app/src/ui/paint.rs:871`

**What:** After a pan away and back 2 s later, a band of arrows flashes at about 95% of the raw flow. **Fix:** Treat stale keys as new and seed them from live neighbours.

#### UI-23 · Low · "Fire now" commits the drive without the row's Divide flag
`crates/funfern-app/src/ui/law_editor.rs:118`

**Fix:** Copy `inverted` along with the drive and the gate.

#### UI-24 · Low · A repeated error merged into the previous log entry does not relight the marker
`crates/funfern-app/src/ui/diagnostics.rs:121`

**Fix:** Set `unseen_error` before the merge returns.

#### UI-25 · Low · Delete during a control drag removes the control mid-gesture
`crates/funfern-app/src/ui/draw_tools.rs:325`

**What:** The drag then moves the neighbouring control. **Fix:** Guard `delete_selection` with `interaction_in_progress`, as `deletion_offer` does.

#### UI-26 · Low · A pulse click with no active generation is dropped without a word
`crates/funfern-app/src/ui/draw_tools.rs:474`

**What:** It happens when the tool is armed while a new scene prepares. One skeptic dissented: a scene replacement disarms the tool, so the path is narrower than first described. **Fix:** Set a status message, as the other refusal does.

#### UI-27 · Low · A typed probe name is lost when the inspector tab is switched or closed
`crates/funfern-app/src/ui/probes.rs:224`

**Fix:** Commit when the field was not drawn this frame, not when the selection changes. The region name field needs the same fix.

#### UI-28 · Low · Pulse→Continuous carries the pulse's offset over as an integrated rate
`crates/funfern-app/src/ui/signals.rs:156`

**What:** A flash with offset 0.4 becomes a flux of 24 at T = 60 s, growing without bound. **Fix:** Zero the offset for integrating uses and tell the user.

#### UI-29 · Low · The pulse-shape spectrum aliases for long pulses
`crates/funfern-app/src/ui/signals.rs:443`

**What:** A 1,000 s pulse on a 10 Hz carrier shows its peak at about 6.4 Hz. **Fix:** When the sample cap binds, cap the plotted range at the true Nyquist frequency or shorten the window.

#### UI-30 · Low · Disabled probes are drawn in the failure red
`crates/funfern-app/src/ui/probe_hit.rs:333`

**Fix:** Check `enabled` before `probe_status`.

#### UI-31 · Low · Touch taps use the desktop hit radii
`crates/funfern-app/src/ui/input.rs:30`

**What:** On the release frame no touch is active any more, so a tap uses the mouse radii. A tap and a drag at the same spot then pick different things. **Fix:** Keep the touch state for the release frame.

#### UI-32 · Low · A failed vector-overlay upload is reported only on the status line and retried every frame
`crates/funfern-app/src/ui/viewport.rs:333`

**Fix:** Report it through the diagnostics warning, and stop rebuilding every frame.

#### UI-33 · Low · Diagnostics log times can read "m:60.0"
`crates/funfern-app/src/ui/events.rs:46`

**Fix:** Round to tenths before splitting off the minutes.

#### UI-34 · Low · A probe name over 64 bytes is silently discarded
`crates/funfern-app/src/ui/probes.rs:307`

**Fix:** Add a byte limit to the field, trim before measuring, and report a refusal.

#### UI-35 · Low · After the 30 s startup timeout fires on a slow start, every later stop is worded as "Unable to start WebGPU"
`index.html:246`

**Fix:** Reset the failed state when the app hides the startup overlay.

### Persistence

#### PER-1 · Medium · The stop message says "The document is saved" even when the autosave failed
`crates/funfern-app/src/stopped.rs:105`

**What goes wrong.** In a browser that blocks storage, a device loss shows the "saved" line. The user reloads, trusting it, and the session is gone. Natively, a full disk does the same before the process exits.

**Why.** `autosave_now` returns nothing, and the message is hard-coded. The failure notice is an egui window that never draws once rendering has stopped. A write skipped after an earlier failure also counts as saved.

**Fix.** Have `autosave_now` return whether the document is safely written, and word the message from that.

#### PER-2 · Medium · Load requires the accepted scene to pass today's compile rules
`crates/funfern-app/src/topology_persistence.rs:1234`

**What goes wrong.** A version 22 file written before e6ffc06 can hold a driven Dirichlet face meeting a pinned side. That file is now refused. An autosave of it is replaced by an example and then overwritten, and shared links to it cannot be opened.

**Why.** `validate_document` compiles the accepted scene under the current rules, with no fallback and no migration. The frozen v22 scene set has no such scene.

**Fix.** Open a scene that fails today's rules as an invalid draft instead of refusing the file, and add such a scene to the frozen set.

#### PER-3 · Medium · Undo across New or an opened example restores the scene but keeps the incoming settings and readouts
`crates/funfern-app/src/topology_editor.rs:281`

**What goes wrong.** The outgoing scene's adaptation setting, mesh edge and readouts are lost for good, and the autosave writes the mix. A second New wipes settings with no history step at all.

**Why.** History steps store only the model.

**Fix.** Store presentation and readouts in steps that replace the whole scene, and restore them with the model.

#### PER-4 · Medium · The unbounded overlay Min/Max fields accept infinity, after which every save fails
`crates/funfern-app/src/ui/panels.rs:235`

**What goes wrong.** Typing `inf` or `1e400` makes every later autosave, save and link fail with a generic message.

**Why.** The DragValues have no range, and `valid()` requires finite values.

**Fix.** Give both fields a finite range.

#### PER-5 · Medium · On the web, Save… reports "Scene saved" when the user presses Cancel or Ok in rfd's overlay
`crates/funfern-app/src/files.rs:135`

**What goes wrong.** Neither Ok nor Cancel downloads anything, yet the status says "Scene saved". One skeptic found the claim false for SVG export.

**Why.** On wasm, rfd 0.17.2 returns a writable handle at once, and `write` always returns Ok.

**Fix.** Use a direct Blob download, as PNG snapshots already do, and word the status as a download.

#### PER-6 · Low · Quit, window close and page reload do not flush the autosave
`crates/funfern-app/src/ui/session.rs:322`

**What:** Edits made inside the 0.8 s debounce are lost. **Fix:** Call `autosave_now` on AppExit and window close natively, and on pagehide or visibilitychange on the web, refreshing the link as well.

### Verification machinery

#### VER-1 · Medium · The nightly pages-blocked recorders cell runs each fixture twice and reaches the 6-hour limit
`.github/workflows/ci.yml:910`

**What goes wrong.** GitHub cancels the cell near 360 minutes, and a `full` dispatch then blocks the deploy.

**Why.** In pages-blocked hosting every fixture runs cold and then warm. The comment's "about three hours" counts each page only once.

**Fix.** Split the recorders across cells, or run the warm pass only for a subset, and set an explicit job timeout.

#### VER-2 · Medium · The nightly wall sweep's "coarsest reach" runs at 0.2, but the slider goes to 0.25
`.github/workflows/ci.yml:164`

**Why.** The slider range is 0.02 to 0.25 (panels.rs:367-370), and mesher failures increase toward the coarse end.

**Fix.** Sweep at 0.25.

#### VER-3 · Medium · The device suite never runs the nonlinear, gated or oscillator probe variants, or the resident filter beside a field law
`scripts/device-suite.sh:100`

**Fix.** Add the CONSUMER_* runs and the NONLINEAR_FILTER and NONLINEAR_GAP runs.

#### VER-4 · Medium · The device suite links each binary only when its own run starts
`scripts/device-suite.sh:178`

**What goes wrong.** A build made beside the suite changes what the later runs execute. The suite still reports its verdict on the original tree.

**Fix.** Link every binary right after the build, and run only from those links.

#### VER-5 · Medium · Four mutation patches no longer apply to HEAD
`tests/mutations/curve-probe-flux-reversed/patch.diff:8`

**What goes wrong.** The nightly reports curve-probe-flux-reversed, far-field-frame-at-step-end, clock-forgotten-on-handoff and drive-phase-reset-on-handoff as "not detected" and goes red. The checks they guard are untested meanwhile.

**Why.** e0ce86f and ab17b11 rewrote the shader lines these patches touch.

**Fix.** Rewrite the patches against HEAD, and add a `git apply --check` step to the gate.

#### VER-6 · Medium · The temporal consumer gate never puts a travelling drive on the stiffness row
`crates/funfern-app/examples/canonical_gpu_temporal_consumer.rs:94`

**What goes wrong.** The gate cannot catch a regression in the stiffness-row spatial phase, which the log records as having already happened once. No Switch is ever begun and no row is inverted, so those probe paths go unchecked too.

**Fix.** Add a mode with a travelling stiffness drive, a begun Switch and an inverted row.

#### VER-7 · Medium · The timing example divides by `steps` but times a different number of steps
`crates/funfern-app/examples/canonical_gpu_temporal_timing.rs:364`

**What goes wrong.** Figures come out at 0.77, 0.90 or 1.02 times the true per-step cost. The documented "bimodal" 2,000-step results are this error.

**Fix.** Divide by the difference between the read-back counts at the start and end of the timed window.

#### VER-8 · Medium · `far_field::judge` never checks that the records cover the whole run
`crates/funfern-app/src/ui/e2e/far_field.rs:88`

**What goes wrong.** A recorder that drops the head or the tail of its trace still passes.

**Fix.** Check the first record against the twin's first projectable frame, and the last record against the endpoint.

#### VER-9 · Medium · The rejection fixture passes if nothing ever refuses
`crates/funfern-app/src/ui/e2e.rs:314`

**Fix.** Fail when the run reaches step 4096 without a refusal.

#### VER-10 · Medium · The irregular-mesh handoff test's Q bounds accept a transfer that flattens Q to its mean
`crates/funfern-core/src/canonical_transfer.rs:3239`

**What goes wrong.** A transfer that returns the mean density everywhere scores about 0.021, below both bounds. The skeptics found other tests catch two of the three example regressions.

**Fix.** Measure the error on the field minus its mean, with bounds specific to this fixture.

#### VER-11 · Medium · The "edits the field cannot tell" fixture applies the component-total correction that the app skips on the disc edits
`crates/funfern-app/src/topology_examples.rs:12006`

**Why.** Every generation compiles at revision 0, so the fixture's check always reads "same geometry". The app compiles geometry edits at a new revision and skips the correction.

**Fix.** Compile the fixture's generations with the editor's revision sequence.

#### VER-12 · Medium · The component-total test only restates the transfer's own postcondition
`crates/funfern-app/src/topology_runtime.rs:2874`

**Fix.** Compare the arrived totals with the departing totals the test computes.

#### VER-13 · Medium · The material-frame test cannot detect a wrong or ignored region frame
`crates/funfern-core/src/canonical_temporal.rs:9961`

**Fix.** Compute the expected coordinates independently from the region's frame.

#### VER-14 · Medium · The pin-offset check passes even with the drift-form pin correction removed
`crates/funfern-core/src/canonical_temporal.rs:16762`

**Why.** Without the correction the drift form already scores 7.2 times better than leapfrog, against a threshold of 5.

**Fix.** Compare against a run that uses the plain signal and require the measured improvement.

#### VER-15 · Medium · The nonlinear area-probe test claims junction nodes, but its scene has one material
`crates/funfern-core/src/canonical_temporal.rs:9658`

**Fix.** Add a second region with a different field law.

#### VER-16 · Medium · `temporal_tensor_scaling_keeps_the_base_anisotropy_shape` never calls production code
`crates/funfern-core/src/canonical_temporal.rs:10660`

**Fix.** Exercise the scaling through a driven anisotropic operator, or delete the test.

#### VER-17 · Medium · The stationary-leakage check runs through the identity map
`crates/funfern-core/src/canonical_transfer.rs:3282`

**Fix.** Transfer the stationary field through `b_to_fine` and `b_to_coarse`.

#### VER-18 · Medium · The round-trip energy assertion compares `edit_exchange` with its own definition
`crates/funfern-core/src/canonical_transfer.rs:3718`

**Fix.** Assert the corner nodes' values and bound the energy they add.

#### VER-19 · Medium · The coarsening test on the T-junction merges nothing
`crates/funfern-core/src/mesh/topology_plan.rs:3219`

**Fix.** Use curved or multi-sample spans, so that some runs can merge up to a junction.

#### VER-20 · Medium · The line-probe stride test checks its own copy of the formula
`crates/funfern-app/src/wave_gpu.rs:3081`

**Fix.** Read the strides the recorder stores.

#### VER-21 · Medium · The weld-survivor test cannot fail, because both regions carry the same material
`crates/funfern-app/src/topology_editor.rs:6917`

**Fix.** Give the two regions distinct materials, and assert that the weld happened.

#### VER-22 · Medium · The "a refused command must not spend a region id" check uses a refusal that returns before allocation
`crates/funfern-app/src/topology_editor.rs:8934`

**Fix.** Use a refusal that reaches the compile.

#### VER-23 · Medium · The boundary-probe deletion test never makes a second piece
`crates/funfern-app/src/topology_editor.rs:8572`

**Fix.** Delete an interior span of a probed curve with four spans.

#### VER-24 · Medium · The guided-tour assertion "the frequency alone" cannot fail
`crates/funfern-app/src/ui/guide.rs:1318`

**Why.** The edit leaves acceptance Pending, which alone makes the check false.

**Fix.** Call `settle()` before the check.

#### VER-25 · Medium · The scale-relative clearance test never changes the domain's scale
`crates/funfern-core/tests/geometry.rs:614`

**Fix.** Validate on a domain about 100 wide, with clearances on both sides of the scaled margin.

#### VER-26 · Low · The Pages deploy is serialized only at the deploy job
`.github/workflows/ci.yml:1007`

**What:** A slower, older run can publish over a newer one. **Fix:** Add workflow-level concurrency, or check that the commit being deployed is the head.

#### VER-27 · Low · The documented proptest sweep command uses `--bins`, which has run zero tests since 92fa492
`docs/checks.md:311`

**Fix:** Use `--lib`.

#### VER-28 · Low · The Safari runner's stale-build check is swallowed by its own retry loop, and a page without a build stamp is accepted
`browser-tests/safari-e2e.mjs:100`

**Fix:** Throw the mismatch outside `until()`, and require the stamp to be present.

#### VER-29 · Low · frame-rotation-dropped's expected pattern matches the test's own diagnostic line
`tests/mutations/frame-rotation-dropped/manifest:3`

**Fix:** Anchor the pattern to the assertion message.

#### VER-30 · Low · component-totals-dropped expects only the bare word "beyond"
`tests/mutations/component-totals-dropped/manifest:3`

**What:** The pattern would accept a failure at another check as "detected". One skeptic dissented: with the current patch this does not produce a false "detected". **Fix:** Pin the pattern to the Remesh Q check.

#### VER-31 · Low · The device suite runs the oscillator handoff's remesh mode twice under two spellings
`scripts/device-suite.sh:95`

**Fix:** Replace the duplicate with one of the unrun modes.

#### VER-32 · Low · `mutations.sh` with a pattern that matches nothing reports "0 of 0" and exits 0
`scripts/mutations.sh:84`

**Fix:** Exit with an error when nothing matches, as device-suite.sh does.

#### VER-33 · Low · The nonlinear failure gate panics when the device refuses one step later than the oracle
`crates/funfern-app/examples/canonical_gpu_nonlinear_failure.rs:284`

**What:** The gate's own comment accepts a refusal one step late, but it then indexes past the end of `history`. **Fix:** Advance the oracle one more step, or bound the index.

#### VER-34 · Low · The post-settle state-lane bound of 1e-3 rests on a stale 2e-4 from a since-fixed oracle defect
`crates/funfern-app/examples/canonical_gpu_temporal_amr.rs:787`

**Fix:** Tighten the bound toward the measured 2.5e-7.

#### VER-35 · Low · `--checksum` hashes before the full snapshot lands, so b and r are never in it
`crates/funfern-app/examples/canonical_gpu_temporal_timing.rs:336`

**Fix:** Wait for the complementary lane, as the other gates do.

#### VER-36 · Low · `table_differences` reads source rows with the slice stride, but source rows are packed contiguously
`crates/funfern-app/src/canonical_gpu.rs:1212`

**Fix:** Step through source rows one entry at a time.

#### VER-37 · Low · The calibration's "true error" skips the energy weighting it claims
`crates/funfern-core/examples/temporal_amr_calibration.rs:946`

**Fix:** Weight the error by the energy density it already gathers.

#### VER-38 · Low · iga_curved_patch runs the affine and bulged-box geometries on the wrong domain
`crates/funfern-core/examples/iga_curved_patch.rs:239`

**What:** The spike's recorded figures for the bulged box describe a nearly flat patch. **Fix:** Pass size (2, 2), and re-measure.

#### VER-39 · Low · iga_patch_edits rounds the edited run's step counts separately
`crates/funfern-core/examples/iga_patch_edits.rs:171`

**Fix:** Use integer step counts, as the refinement section does.

#### VER-40 · Low · cavity-batches' 128- and 256-step batches are capped at 64 per frame
`crates/funfern-app/src/ui/e2e.rs:266`

**What:** Two of the four batchings run the same schedule. **Fix:** Lift the caps for the fixture, or document the real batchings.

#### VER-41 · Low · The rejection fixture's wrong-code message reports the reference's verdict on step 1
`crates/funfern-app/src/ui/e2e.rs:1744`

**Fix:** Advance the reference to the refused step before stepping it.

#### VER-42 · Low · The line-integral check uses the twin's length, so a wrong readout length passes
`crates/funfern-app/src/ui/e2e/observables.rs:510`

**Fix:** Integrate with `state.probe_metrics`.

#### VER-43 · Low · The readouts' transfer check bypasses the readout's transfer path
`crates/funfern-app/src/ui/e2e/observables.rs:638`

**What:** It also measures a single tone through a pure delay, which reads |H| = 1 under any alignment. **Fix:** Go through `TransferView`, with a signal whose phase matters.

#### VER-44 · Low · The withdrawn-handoff test's "nothing was admitted" check cannot fail
`crates/funfern-app/src/ui/sequences/generations.rs:1161`

**Fix:** Decide admission by the handles, as the device does.

#### VER-45 · Low · The stamp-precision test splits the clock origin differently from the device
`crates/funfern-app/src/wave_gpu.rs:2905`

**Fix:** Model the device's whole-seconds-plus-fraction split, and its 3e-5 bound.

#### VER-46 · Low · The emitter's seed check mostly reads leakage from a rectangular window
`crates/funfern-app/src/topology_examples.rs:7553`

**Fix:** Window the DFT, or tie the seed bound to the measured tone frequency.

#### VER-47 · Low · `core_mode`'s bisection returns NaN for small V
`crates/funfern-app/src/topology_examples.rs:7955`

**Fix:** Bracket the root on [0, min(V, π/2)].

#### VER-48 · Low · The PH-01 box modes cannot detect a TM/TE slot swap
`crates/funfern-app/src/topology_examples.rs:11254`

**Fix:** Add an impedance-sensitive check, such as a two-medium interface with unequal impedances.

#### VER-49 · Low · The assertions that stepped frequencies "miss ω_h by their law" are algebraic identities
`crates/funfern-app/src/topology_examples.rs:11096`

**Fix:** Drop them, or compare against ω_h measured independently.

#### VER-50 · Low · The slow near-field fixture drives the wall's even tangential modes, not the third mode
`crates/funfern-app/src/topology_examples.rs:14208`

**Fix:** Use the matching odd profile, sin(6.732y).

#### VER-51 · Low · PH-12's per-order filter scales each order by |H_n| instead of the field it met
`crates/funfern-app/src/topology_examples.rs:17709`

**Fix:** Scale by |α_n J_n(kr0)|.

#### VER-52 · Low · The superseded-candidate check commits a token that was never issued
`crates/funfern-app/src/topology_runtime.rs:5091`

**Fix:** Commit request 3's real token.

#### VER-53 · Low · "Undo takes the frame back with the curve" compares two identical values
`crates/funfern-app/src/topology_editor.rs:9516`

**Fix:** Compare against the frame before the drag, and check the curve.

#### VER-54 · Low · The Klein-Gordon oracle uses (m+n) where it should use (m²+n²)
`crates/funfern-core/src/canonical_temporal.rs:13808`

**Fix:** Use m²+n².

#### VER-55 · Low · The fourth-order tests say they run at the app's step but run at 0.9, while the app runs at 1.2
`crates/funfern-core/src/canonical_temporal.rs:16668`

**Fix:** Test at the production share.

#### VER-56 · Low · The support-aware cycle test's 5% bound accepts a flat Q
`crates/funfern-core/src/canonical_transfer.rs:2984`

**Fix:** Tighten the bound to near roundoff, since this field is exact.

#### VER-57 · Low · The identity normalized-matrix check compares a clone with itself
`crates/funfern-core/src/canonical_transfer.rs:3385`

**Fix:** Apply `normalized_map`.

#### VER-58 · Low · The thin-gap history test named for "new" samples never builds one
`crates/funfern-core/src/canonical_transfer.rs:3298`

**Fix:** Add a lengthened target.

#### VER-59 · Low · The filter test's exchange-accounting assertions cannot fail, and its fixture makes the exchange zero
`crates/funfern-core/src/canonical_wave.rs:7838`

**What:** The test would also miss a filter that counts the pin's exchange twice. **Fix:** Start with the pin off its signal, and assert the books balance.

#### VER-60 · Low · The handoff-surplus test compares energies at two different end times
`crates/funfern-core/src/canonical_wave.rs:9205`

**Fix:** Compare both runs at the same instant.

#### VER-61 · Low · outer_resize_and_junction_trace_membership_survive_projection resizes nothing
`crates/funfern-core/src/mesh/topology_plan.rs:4146`

**Fix:** Resize the domain, and assert the junction's membership.

#### VER-62 · Low · The constant-field grid-filter test filters an all-zero vector
`crates/funfern-core/src/wave_quadratic.rs:4333`

**Fix:** Give the two levels different constants.

#### VER-63 · Low · The second-order energy oracle adds a term in energy×time
`crates/funfern-core/src/wave_quadratic.rs:3521`

**What:** The ignored reproducer for the known instability depends on this functional. **Fix:** Use a proper energy for the Engquist-Majda law.

#### VER-64 · Low · The mixed-sides test's final `any != 0` check cannot fail
`crates/funfern-core/src/wave_quadratic.rs:3366`

**Fix:** Check the Neumann side's effect after stepping.

#### VER-65 · Low · The `span_context` side test cannot detect a left/right swap
`crates/funfern-app/src/topology_viewport.rs:1373`

**Fix:** Assert which face is on the left of increasing parameter.

#### VER-66 · Low · `directional_skin_keeps_ratio_and_geometric_mean` never checks the geometric mean
`crates/funfern-core/src/wave.rs:2201`

**What:** One skeptic dissented, because another test catches the Mechanical regression. **Fix:** Assert the determinant equals the square of the skin stiffness.

#### VER-67 · Low · The energy-breakdown test "including auxiliaries" has no auxiliaries
`crates/funfern-core/src/canonical_consumer.rs:711`

**Fix:** Use a fixture with a thin gap and outgoing walls.

#### VER-68 · Low · The sentence test covers 2 of the 6 error types it claims to guard
`crates/funfern-core/tests/geometry.rs:1137`

**Fix:** Add `TopologySceneIssue`, `TopologyMeshPlanError`, `FaceAnchorIssue` and `SeparatorAttachmentIssue`.

#### VER-69 · Low · The interface-trace check after local repair passes when no interface edges remain
`crates/funfern-core/tests/mesh.rs:1649`

**Fix:** Assert that the filtered set is not empty.

#### VER-70 · Low · The "source mesh must remain immutable" check compares an Arc with its own clone
`crates/funfern-core/tests/mesh.rs:1605`

**What:** One skeptic dissented: no defect can slip through, because Arc forbids mutation. **Fix:** Deep-copy before the job, or drop the assertion.

## 3. Plausible and unverified findings

**Plausible:** none. No finding ended with the skeptics split or undecided.

Seven confirmed findings had one skeptic refute them out of three verdicts. The majority confirmed them, and each entry above notes the dissent: PR-11, NUM-16, UI-19, UI-26, VER-30, VER-66 and VER-70. Two more were confirmed with one skeptic proposing a different severity: RT-4 (medium) and GPU-4 (low).

**Unverified:** none. Every skeptic returned.

## 4. Coverage

- **Reviews:** 74 units, 10 cross-cutting reviews and 8 gap reviews. Every reviewer returned a result.
- **Outcome:** 214 confirmed, 0 plausible, 0 unverified, 6 refuted.
- **Already-known items seen again (2):**
  - Deleting the wall between a hole and a subdomain silently deletes the subdomain when its anchor was on the deleted curve (crates/funfern-app/src/topology_editor.rs:4836).
  - Temporal AMR transfer-alone check compares b against an empty lane on some runs, so it cannot fail (crates/funfern-app/examples/canonical_gpu_temporal_amr.rs:728).
- **Confirmed but already known:** GPU-8, the pulse-train start rounding on a live patch, is marked already listed as worth checking.