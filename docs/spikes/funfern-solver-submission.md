# The solver's own submission

Started 2026-10-05, on `solver-submission`. The question the TODO in
`docs/engineering-log.md` has carried since 2026-09-23, corrected on
2026-10-04: whether the solver should submit its work to the GPU on its own,
outside Bevy's render graph, and what that would buy. The change is invasive,
so this spike measures the benefit before anything is designed.

## Where the frame stands

Every frame's steps are encoded by `compute_canonical_wave`, a system in the
`RenderGraph` schedule ordered before the cameras. Bevy flushes each graph
system's encoder into its own command buffer and submits the frame's buffers
in one `queue.submit`, so the solver's pass and the cameras' passes are
already separate command buffers, in one submission. The drawing reads
nothing the solver writes: the painter uploads the latest state readback
(`docs/spikes/funfern-drawn-pacing.md`, "What a frame draws"), and the
marker experiment there saw render passes run beside the solver pass.

And yet a frame whose pass outruns the display's interval is late. Where the
solver fills the frame, the picture draws at 55 to 57 fps with 10 to 20 %
wobble, and the whole batch-ceiling controller exists to keep the pass
inside the interval, in whole steps. So what ties the present to the pass is
not the data. The candidates are the queue's order on Metal, where command
buffers on one queue are committed in order, and the drawable's release,
which Metal may hold until every command buffer committed before its
presentation has completed.

## Questions

1. **Does a submission of the solver's own free the present from the pass?**
   The pass on its own encoder and its own `queue.submit`, placed after the
   frame's draws and present, and separately before the frame's submission.
   The batch forced past the interval so the GPU is saturated with solver
   work, on the heavy scene; the light scene as the control. If a placement
   keeps the frame on time while the pass spills, the architecture is worth
   designing: a 60 fps picture on heavy scenes while the solver runs as fast
   as the GPU allows. If neither does, the TODO closes.
2. **How many scenes would gain?** The gallery classified by pass share and
   speed reached at 60 and 120 Hz. Scenes whose solver fills the frame gain
   a smooth picture; scenes between 0.75 and 1 of the interval and short of
   their speed gain speed; the rest gain nothing.
3. **How much GPU time is idle?** From the timer's readings, the gap between
   one frame's pass end and the next's pass begin, which is what feeding the
   GPU continuously could claim, and whether a second phase with several
   submits a frame is worth anything.
4. **Chrome.** Only if 1 passes on Metal: the same in the browser build,
   read through the diagnostics with the Playwright harness.

## Method

The spike's build (on the `solver-submission` branch before the scaffolding
was taken out again) read `FUNFERN_SOLVER_SUBMIT=before|after|feeder` at
startup; unset was the graph, as on main. `before` and `after` put the pass
on its own encoder and its own `queue.submit`, in a `Render` system ordered
before `render_system` or after it, past the frame's draws and present,
retiring its own steps through `on_submitted_work_done`. `feeder` recorded
the frame's steps in chunks of `FUNFERN_SOLVER_CHUNK` steps (2 by default),
each its own command buffer, for a thread that submitted them
`FUNFERN_SOLVER_INFLIGHT` at a time (1 by default) and waited on each
submission's index with `Device::poll` before the next, so the frame's
draws never waited behind more than that many chunks. `FUNFERN_FIXED_BATCH=N`
asks N steps
every running frame in place of the controller's batch; the GPU
backpressure, 64 steps outstanding, still bounds what is admitted, so a batch
past what the GPU completes in an interval saturates it with the queue three
or so frames deep. The trace (`FUNFERN_PACING_TRACE`) and
`drawn_pacing_summary` measure as on 2026-10-04, with one more line: the
share of frames the main world measured late, past one and a half times its
median delta.

Scenes, from the default scene's autosave in an isolated HOME, adaptation
off:

- **heavy:** edge 0.025 at 2x speed, the "solver fills the frame" scene of
  2026-10-04 (pass p50 16.6 ms at 60 Hz).
- **light:** edge 0.08 at 1x, the "light" scene.

Runs of 60 s, the placements alternated, the window on the 60 Hz path unless
said otherwise.

## Results

### The placements

The window landed on the machine's 120 Hz panel, so an interval is 8.33 ms.
The heavy scene under the controller runs 6 to 7 steps a frame at 1.0 ms a
step on the GPU, 8.2 fit an interval; the fixed batch of 12 makes a pass of
13.9 ms. 60 s a run, the three placements alternated, two rounds and a
closing reference:

| run | placement | fps | late | picture still | picture wobble | pass p50 |
| --- | --- | --- | --- | --- | --- | --- |
| 1 | graph | 75.0 | 0.1 % | 0.1 % | 3.4 % | 13.91 ms |
| 2 | after | 75.3 | 0.0 % | 0.1 % | 3.6 % | 13.89 ms |
| 3 | before | 71.6 | 1.1 % | 3.2 % | 8.9 % | 13.95 ms |
| 4 | graph | 74.9 | 0.0 % | 0.1 % | 3.3 % | 13.92 ms |
| 5 | after | 75.0 | 0.0 % | 0.0 % | 3.3 % | 13.93 ms |
| 6 | before | 74.9 | 0.1 % | 0.2 % | 4.0 % | 13.89 ms |
| 7 | graph | 75.1 | 0.1 % | 0.2 % | 3.5 % | 13.91 ms |

The frame is the pass, whatever command buffer or submission the pass is
in: 13.3 ms a frame on a 13.9 ms pass, 75 fps on a 120 Hz panel, in every
placement. The presents land on the panel's 8.33 ms grid, so the main
world's deltas alternate one and two intervals (p50 16.4 ms, p90 16.9). The
light scene under the controller is 119.5 to 119.7 fps in all three, so a
submission of its own costs nothing either. So the queue executes its
command buffers in order: the frame's draws wait behind every piece of
solver work committed before them, and moving the pass to its own
submission moves nothing.

### The feeder

What is left to vary is how much solver work sits ahead of the draws. The
feeder bounds it to a chunk: the frame's steps recorded as command buffers
of a few steps each, a thread submitting them one at a time and waiting for
each to complete before the next.

The first attempt timed the chunks and ran into the timer's own protocol:
a slot is marked submitted as the pass is recorded and its staging buffer
mapped at the frame's cleanup, and a chunk the thread submitted after that
failed validation (`Buffer 'GPU frame timestamp staging' is still mapped`),
twelve times in the first stepping frame at a chunk of one step, after
which the run hung; the chunk of two escaped by timing. The feeder's passes
go untimed since, and a timer that fits a feeder would have to mark a slot
submitted from the submitting thread.

Untimed, the same scene and fixed batch, 60 s a run, the chunk in steps and
the chunks the thread keeps in flight varied; the controller's own run and
the graph with the fixed batch for comparison. Steps a second from the
encoded count over host time; the late share is of frames past one and a
half times the median delta:

| mode | fps | steps / s | late | picture still | picture wobble |
| --- | --- | --- | --- | --- | --- |
| controller, 6 to 7 steps a frame | 119.8 | 757 | 0.4 % | 28.7 % | 10.6 % |
| graph, fixed 12 | 75.0 | 900 | 0.1 % | 0.1 % | 3.4 % |
| feeder, chunk 1, one in flight | 117.8 | 259 | 13.2 % | 0.2 % | 12.3 % |
| feeder, chunk 2, one in flight | 108.3 | 440 | 12.5 % | 0.2 % | 9.0 % |
| feeder, chunk 4, one in flight | 66.8 | 591 | 10.5 % | 11.7 % | 9.8 % |
| feeder, chunk 2, two in flight | 84.2 | 610 | 22.9 % | 7.9 % | 8.7 % |
| feeder, chunk 4, two in flight | 69.8 | 722 | 16.5 % | 15.7 % | 11.5 % |

The feeder does free the frame from the pass: with a chunk of one step the
panel's rate is reached while the steps keep coming. But frames and steps
trade against each other along one line, since every frame's draws and
every chunk take their turn on the same queue, and the feeder's line lies
below the controller's point: the controller gets more frames and more
steps than any feeder setting, because its one pass a frame sits in the
frame's own submission with no handoff between the two, where the feeder
pays a round trip from completion to the next submit at every chunk, idles
the GPU across it, and runs small passes less efficiently (1.4 ms a step in
a two-step chunk against 1.0 in the batch). The late shares of 10 to 23 %
are the draws caught behind a chunk and chunks landing mid-frame. Keeping
two in flight recovers steps and loses frames, as the line says it should.

One more thing the table shows, for the readback TODO rather than this one:
under the controller at 120 fps the picture stood still on 29 % of frames,
where at 75 fps it changed on every one. A depth of three in flight covers
the readback's round trip at 60 Hz and on light scenes at 120, not here.

## Decision

The TODO closes. On a single in-order queue, which is what wgpu and WebGPU
give, the frame's draws wait behind whatever solver work is committed
before them, in whatever command buffer or submission; the solver's own
submission moves nothing, and bounding the work ahead of the draws to a
chunk buys frames only at a worse exchange rate for steps than the
controller already gets by sizing one pass a frame. The architecture
sketched in `docs/plan.md` (a submission module with its own fence, a
GPU-time budget in place of the step ceiling) is not worth building.

What the spike does leave for the pacing work is the controller's
knowledge, not its shape. It sizes the batch by feedback from late frames,
in whole steps, and the GPU timer now gives it the pass's time a step. A
feed-forward budget, the steps that fit the interval less the draws and a
margin at the measured time a step, with the late-frame cut kept as the
backstop, would reach the controller's operating point without the
sawtooth and the first late frame. That is a `frame_step_budget` change,
measurable with the trace as before.

Not run, since they were to size a gain the architecture does not have:
the gallery's classification by pass share (2), the GPU's idle share a
frame (3) and the browser (4). The idle share is worth measuring for the
feed-forward budget; it needs the draws' GPU time, which the marker
experiment of 2026-10-04 could not take.

The scaffolding (`FUNFERN_SOLVER_SUBMIT`, the feeder thread) is removed
after this document; `FUNFERN_FIXED_BATCH` stays, since a spill is what a
pacing experiment has to force, and so does the late share in the summary.

## Decision

_Pending._
