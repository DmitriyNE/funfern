# The drawn cadence: what the eye sees, measured

Started on 2026-10-04 on `drawn-pacing-trace`, as the first of the two pacing
TODOs, and continued the same day on `gpu-bound-cuts`, where the first day's
premise turned out wrong and a defect in the readback pacing fell out. Until
this work every smoothness figure came from the main world's requested steps,
and the one series that could be read back, the completed count, is zero on
most frames and then jumps.

## What a frame draws

The first attempt's premise, and the 2026-09-23 TODO's, was that the drawing
samples the field buffer the solver pass has just written, so that the state a
frame shows is the step the render world encoded for it. It is not. The field
painter (`field_paint.rs`) uploads its vertex values from the host every
frame, and those values are the latest state readback (`display.current`).
So the picture is the read-back state, three frames behind the solver, and
what the eye sees is the readback's cadence. Nothing the frame draws depends
on the frame's own solver pass, which also means the cameras' passes run
beside the solver on the GPU, not after it.

The step the render world encodes is still recorded, as `drawn_step`; it is
what the probes and the readouts are paced by. The picture is the painted
readback, recorded as `picture` (the readback serial the main world painted),
`picture_step` (the step the control readback carried then) and
`picture_generation`.

When a frame is shown is not available: wgpu keeps the presented drawable
private. The render world's host time at encode stands in, exact up to the
vsync that follows, and the GPU's own clock around the solver pass.

## The trace

`FUNFERN_PACING_TRACE=<path>` makes the render world write one CSV row a
frame (`crates/funfern-app/src/drawn_pacing.rs`): the encoded generation and
step, the requested and completed counts, the solver pass's GPU times, the
painted readback, and the main world's pacing note for that frame (delta,
speed, step, batch ceiling, steps asked and admitted, running, withheld).
Without the variable nothing is created.

```sh
env HOME=<scratch> FUNFERN_PACING_TRACE=trace.csv ./target/release/funfern-app
cargo run --release -p funfern-app --example drawn_pacing_summary -- trace.csv
```

The summary leaves out the first two seconds and measures only intervals that
run on one generation at one step. Its wobble is the pacing harness's: the
relative spread of simulated seconds per wall second over sliding windows of
eight frames, on the encoded series, on the GPU clock, on the requested
series against the main world's deltas (the old proxy), on the completed
count and on the picture's step. "Away from handoffs" drops the windows a
handoff withheld stepping in. `picture: still` is the share of frames that
painted the same readback as the frame before: the frames on which the
picture did not change.

## The GPU frame timer

`gpu_frame_timer.rs` is always on where the device has timestamp queries
(Bevy enables every adapter feature, so on Metal it is). The solver pass
carries timestamp writes; a ring of sixteen slots reads them back a few
frames later and the main world can see the last 32 readings through the
request. The trace reads from the same timer.

Only the pass can be timed. A marker after the cameras was tried three ways,
to take the frame's end: an empty compute pass, a render pass loading and
storing the frame's output without drawing, and a compute pass reading a
texel of the drawn texture. Metal orders passes by the resources they touch;
the first two ran beside the solver pass, and the third began 34 µs after the
pass ended on every scene, which is Metal's compute encoders serialising, not
the drawing finishing, since the drawing does not depend on the pass. (A
render pass's start timestamp is its vertex stage's, which on a tile-based
GPU runs ahead of the attachment load; the end has to be sampled.) Bevy's own
pass diagnostics read 0.000 ms on this device.

## The readback gate was not gating

`PacedReadback` bounds how many copies of one readback may be in flight, so
that a slow solver or a backgrounded window cannot leave thousands of staging
copies outstanding. It works by removing the `Readback` component in the
render world on frames it refuses, before Bevy's allocator looks. Its only
ordering was `before(PrepareResources)`; nothing put it after
`ExtractCommands`, the set in which the extracted `Readback` lands. So the
scheduler was free to run it before the component existed, and then it
removed nothing and every refused frame still produced a copy. Which side it
fell on changed with unrelated systems being added to the schedule: the
build of the morning gated, and a readback landed every other frame; the
afternoon's builds did not, and one landed every frame. Instrumented, the
ungated build released 55 more copies than it had claimed over 480 frames,
exactly its refusals.

With the gate ordered after `ExtractCommands` the depth decides the picture.
The round trip of a copy is two frames on a light scene (encoded after the
frame's drawing, mapped at a later submission, delivered at the extraction
after that) and three to four where the solver fills the frame. Frames
between picture changes, 25 s runs on an M1 Max at 60 Hz:

| depth | light scene | adaptation, overlay, integrated field | solver fills the frame |
| --- | --- | --- | --- |
| 1 | 2: 97 %, 3: 3 % | 2: 95 %, 3: 3 %, 4: 2 % | 2: 14 %, 3: 86 % |
| 2 | 1: 92 %, 2: 8 % | 1: 93 %, 2: 6 % | 1: 50 %, 2: 46 %, 3: 4 % |
| 3 | 1: 97 %, 2: 3 % | 1: 98 %, 2: 1 % | 1: 92 %, 2: 8 % |

Depth 1 is a 30 Hz picture on a 60 Hz display, everywhere: what the morning's
build showed, and what the 2026-09-16 capture entry found for the screenshot
readback. Depth 3 is where it flattens, as it was there; the cost is a
staging buffer a copy. `IN_FLIGHT_DEPTH` is 3 and the gate's test holds one
claim per completion, so a completion landing in the frame of a claim cannot
put more in flight than the depth.

## What the first attempt showed, corrected

The default scene, window on this machine's 60 Hz path, 25 s runs, two each.
The "drawn" column of the first attempt's table was the encoded step, not the
picture; it is kept here under that name:

| scene | encoded wobble | away from handoffs | requested | completed | frozen |
| --- | --- | --- | --- | --- | --- |
| adaptation on | 9.1, 7.6 % | 6.9, 3.9 % | 9.2, 7.6 % | 11.2, 9.2 % | 2.2, 1.8 % |
| adaptation off | 10.7, 12.3 % | the same | 10.9, 12.4 % | 11.8, 13.2 % | 0 |
| off, edge 0.035 | 9.6, 9.0 % | the same | 9.7, 9.1 % | 11.4, 12.5 % | 0 |

- **Encoded equals requested.** On every run the render world encoded what
  was requested by the time it drew, so the main world's proxy tracks the
  solver to within 0.3 points. It does not track the picture, which on those
  runs changed on every other frame: the "completed still 51 %" of that
  attempt was the picture standing still.
- **Handoffs freeze the picture.** Each adaptation withholds stepping for
  three frames, 50 ms, and that time is not made up.
- **The rest is the ceiling's sawtooth.** With adaptation off the scene runs
  4 steps a frame on passes of about 5 ms in a 16.7 ms frame, and still dips:
  after a cut the batch runs `3,2,3,3,2,1,1,2,2,2,3,3,3,4,4,5,5`, a third of a
  second at about half speed.
- **Most cuts are not the solver's.** Over 60 s runs, 2.3 to 7.6 % of frames
  ran past 25 ms in the render world as in the main world, so the frames
  were really late. But in about 80 % of the cuts no solver pass within two
  frames ran past 12 ms (the median of the largest was 6 to 8 ms, the batch
  blamed 3 steps), where a frame is 16.7.
- **0.75 against 0.9 is still not resolved.** Three alternating 60 s runs of
  each backoff on the adaptation-off scene:

  | backoff | wobble | cuts a second | encoded speed |
  | --- | --- | --- | --- |
  | 0.75 | 18.3, 22.0, 26.7 % | 2.7, 3.5, 4.5 | 0.87, 0.84, 0.80 |
  | 0.9 | 30.0, 27.6, 12.0 % | 8.6, 7.8, 1.6 | 0.73, 0.78, 0.93 |

  Within either build the wobble follows the cut rate, and the cut rate
  follows how often a frame comes in late, which moved threefold between runs
  minutes apart.

## After the gate fix

The committed build, 25 s runs, the same machine and display:

| scene | fps | picture changes on | picture lag | picture wobble | cuts a second | solver pass p50 |
| --- | --- | --- | --- | --- | --- | --- |
| adaptation off | 59.9 | 98 % of frames | 3 frames | 6.0 % | 0.46 | 4.7 ms |
| adaptation, overlay, integrated | 59.9 | 97 % | 3 | 6.4 % | 0.18 | 6.8 ms |
| edge 0.025 at 2x speed | 55.2 | 85 % | 4 | 12.2 % | 4.8 | 16.6 ms |

## The false cuts

Done the same day on `solver-blame`. A late frame carrying steps used to cut
the batch whatever made it late. The pass measured in the late frame itself
cannot tell the solver's late frames from the display's: whatever stalls a
frame stalls the GPU with it, so a late frame on the light scene often
carried one slow pass, and the largest of the last four passes at the cuts
had a median of 0.53 of the interval against 0.34 elsewhere. The solver's
steady share can: the median of the latest six passes, as the ring delivers
them, three frames late.

| scene | steady share p50 / p99 | share at the cuts p10 – p90 | cuts |
| --- | --- | --- | --- |
| light, three 60 s runs | 0.23 – 0.26 / 0.46 – 0.55 | 0.16 – 0.33 | 887 |
| adaptation, overlay, integrated field | 0.30 – 0.41 / 0.46 – 0.54 | 0.22 – 0.55 | 10 |
| solver fills the frame, three runs | 1.02 – 1.04 / 1.20 – 1.27 | 1.02 – 1.26 | 285 |

Any threshold from 0.6 to 0.9 keeps every cut of the last row and drops all
but one of the 887 of the first. `SOLVER_BLAME_SHARE` is 0.75: a late frame
cuts only when the solver's steady share has reached three quarters of the
interval; below that the ceiling holds, neither cut nor grown. An unknown
share, no timestamp queries or no reading in the last quarter second, cuts
as before. The pacing harness models the share the same way, and its wobble
with room to spare fell from 18.3 % to 3.8 %, without from 16.6 % to 10.9 %;
the gates are ratcheted there.

Old and new builds alternated, 60 s a run, two to four rounds a scene:

| scene | cuts a second | picture wobble | fps | speed |
| --- | --- | --- | --- | --- |
| light | 0.52, 0.47 → 0.00, 0.00 | 7.2, 7.3 → 4.4, 4.4 % | 60 → 60 | 0.95 → 0.95 |
| adaptation, overlay, integrated | 0.14, 0.10, 0.05 → 0.03, 0.00, 0.09 | 6.1, 6.5, 6.6 → 8.6, 4.5, 6.8 % | 60 → 60 | 0.96 → 0.96 |
| edge 0.06 | 0.33, 2.30 → 0.00, 0.03 | 6.3, 17.5 → 4.7, 5.5 % | 60 → 60 | 0.95, 0.88 → 0.95, 0.95 |
| solver fills the frame | 4.95, 4.89, 3.37, 3.99 → 5.41, 4.91, 3.37, 3.36 | 12.3, 11.3, 9.5, 20.0 → 21.2, 11.6, 9.3, 9.3 % | 55 – 57 → 55 – 57 | 0.73, 0.73, 1.20, 0.98 → 0.66, 0.73, 1.21, 1.21 |

Where the solver has room the cuts are gone and the picture's wobble is down
to the display's own; the second edge run caught one of the cut storms the
display's late frames used to set off, 2.3 cuts a second and 17.5 % wobble,
which the new build did not have. Where the solver fills the frame three of
the four pairs match to the second decimal; the first new run had the GPU in
a slower state (pass share 0.89 against 1.00), the share dipped below the
threshold on 23 % of its frames after cuts had shrunk the batch, 49 late
frames were held, and it came out slower. The edge scene, meant to sit at
0.7 of the interval, landed at 0.33: a finer mesh at a tighter step ran
nearly as cheaply a frame as the coarse one. The GPU's state moved between
rounds, 0.73 to 1.2 of the set speed on the same scene and build, so only
pairs run back to back compare.

The same, with the window on the machine's 120 Hz panel, 60 s a run, two
rounds:

| scene | cuts a second | picture wobble | fps | speed | pass share |
| --- | --- | --- | --- | --- | --- |
| light | 1.08, 1.27 → 0.00, 0.00 | 6.1, 8.0 → 3.8, 4.2 % | 120 → 120 | 0.99 → 1.00 | 0.53, 0.25 → 0.14, 0.14 |
| edge 0.06 | 0.35, 0.70 → 0.00, 0.00 | 4.2, 6.5 → 3.9, 3.5 % | 120 → 120 | 0.99 → 1.00 | 0.33, 0.22 → 0.19, 0.19 |
| adaptation, overlay, integrated | 0.87, 11.6 → 2.2, 5.5 | 10.4, 21.9 → 12.3, 14.8 % | 118.6, 107.4 → 116.9, 113.6 | 0.96, 0.79 → 0.95, 0.91 | 0.42, 0.80 → 0.36, 0.73 |
| solver fills the frame | 19.7, 17.1 → 20.0, 21.1 | 22, 32 → 21, 28 % | 96, 78 → 93, 92 | 0.31, 0.25 → 0.29, 0.27 | 1.00, 1.15 → 1.03, 0.99 |

At 120 Hz a frame asks for half the steps, so the light scenes' passes are
0.14 to 0.33 of the interval and their cuts vanish as at 60. The overlay
scene is the borderline case here: its pass is 0.36 to 0.80 of the interval
depending on the state the GPU's clock is in, the share crossed the
threshold on 25 % of one new run's frames and 52 % of the other's, and the
result is inside the swing between the old runs (0.9 to 11.6 cuts a second
on the same build). Where the solver fills two frames both builds collapse
the same way. The picture changed on 99 %, 97 %, 85 % and 88 % of frames at
this refresh rate, so a depth of three still covers the round trip.

## The budget from the measured cost

Started 2026-10-05 on `feed-forward-budget`, after the submission spike
(`funfern-solver-submission.md`) left the controller's shape alone and
pointed at its knowledge. The controller found the batch that fits by
feedback alone: a quarter step of ceiling a frame while the batch pressed
it, a cut to three quarters when a frame came in late and the solver was
filling it. Every operating point was reached through a sawtooth and a
first late frame.

The GPU timer prices a step. Each reading now carries the steps its pass
ran and the generation it stepped; the main world takes the median of pass
time over steps across the fresh readings of the current generation
(`step_seconds`), and `StepBudget` sets the ceiling to what fits: a target
share of the interval over the cost, every frame, from the measurement.
The late-frame rule stays as the backstop and acts on the target: a late
frame the solver is to answer for (its steady share at or past
`SOLVER_BLAME_SHARE`, as before) lowers the target to nine tenths of the
share the pass measured; frames in time at the ceiling bring it back by
0.0025 a frame, to the nominal 0.8 and no further; it never drops below
0.4. While the share reads well under the target (by 0.15) with the batch
at the ceiling, the ceiling also probes a quarter step above the fit, for
the trap described below. Without a cost, no timestamp queries or a
generation too new for three readings, the old rule runs from where the
ceiling stands. The trace carries the cost and the target as two more
columns; the summary counts the target's drops as cuts where it has them.

**What a step costs depends on the load.** A fixed-batch sweep on the heavy
scene at 120 Hz priced a step at 1.6 ms in a batch of four, 1.2 in five,
1.0 in six and seven, and 1.3 in eight and nine. A lightly loaded GPU
clocks down, and a pass run while the queue is backed up shares the GPU
with the frame before it. Both inflate the cost and shrink the budget,
which lightens the load and brings the cost back; neither makes the budget
over-promise. The pass's fixed cost on top of its steps has the same sign,
so a ceiling priced in a small batch is short of the fit and the next
readings, at the larger batch, raise it: it converges from below.

**The first rule took the target to the floor.** It let the blame
threshold follow the target down, for the case where frames stay late at
a share below 0.75. Measured against the old build, alternating, 60 s a
run, it ran the heavy scene at 0.22 and 0.42 of the set speed against the
old build's 0.64 and 0.61, with 83 and 68 % picture wobble against 28 and
31. The machine was carrying the user's nine fitting jobs at the time,
load average 60 to 90, and a fifth of all frames were late for the host's
reasons on both builds and on the light scene too. Each of those frames,
blamed at a share of 0.8, cut the target by a tenth; once the target was
under 0.75 the threshold went with it and every further late frame cut
again, down to the floor, where batches of two steps on a clocked-down
GPU cost 1.6 ms each. With the threshold fixed, frames late for the
host's reasons are blamed only while the pass fills the interval, and the
target hovers just under the threshold instead: the solver keeps most of
its room, as it does under the old rule's balance of cuts and recovery.
The fixed-batch sweep ran under the same load, so its late frames say
nothing about the GPU; its costs are GPU timestamps and stand.

**The second cut the ceiling's floor out from under it.** With the
threshold fixed, the heavy scene ran at 0.52 and 0.56 of its set speed
against the old build's 0.65 and 0.63, wobble 46 and 38 % against 35 and
33. The target had hovered where meant, 0.59 to 0.78, dropping under three
times a second, but the ceiling had a tail: a tenth of the frames at 1.5
and 2.5 steps. Each late frame cut the target by a tenth of itself, and
the share that decides the blame is a median that trails by several
frames, so the late frames of one spell, all reading the same share,
compounded into two or three cuts; the fit at the lowered target met a
cost spike (p90 3.5 ms a step against a median of 1.1) and collapsed, and
a small batch keeps the GPU clocked down and the cost high. The cut now
goes to nine tenths of the share the pass measured, never under the
target again: a spell's late frames land on the same target. Measured, 60
s a run alternating, under the same host load:

| run | fps | late | speed of set | picture wobble | cuts a second | batch p10 / p50 / p90 |
| --- | --- | --- | --- | --- | --- | --- |
| old | 80.7 | 23.9 % | 0.577 | 36.4 % | 7.25 | 3 / 6 / 8 |
| new | 84.7 | 23.4 % | 0.584 | 29.6 % | 1.7 | 4 / 6 / 6 |
| old | 83.7 | 23.6 % | 0.611 | 35.8 % | 7.10 | 3 / 6 / 8 |
| new | 84.5 | 22.9 % | 0.669 | 25.3 % | 2.0 | 5 / 6 / 7 |

The old rule's cuts are the ceiling's, the new rule's the target's. The
ceiling's tail is gone (p10 at 4.3 and 5.2 steps), the batch spreads over
four to seven steps where the old rule's sawtooth spread it over three to
eight, and the light scene is unchanged (0.92 to 0.94 of set, wobble 18 to
17 %).

**The trap, and the probe.** A pass has a fixed cost on top of its steps
and a lightly loaded GPU clocks down, so a step priced in a small batch is
dear, and the integer floor on the steps can hold a fit on the same small
batch for good: a fit of 2.5 steps runs two, which price the step at what
gave 2.5. The first rule sat at a ceiling of one on half its frames for
exactly this reason once its target had collapsed. So while the share
reads 0.15 or more under the target with the batch at the ceiling, the
ceiling probes a quarter step above the fit, as the old rule always did;
the next readings, at the larger batch, price the step lower, and the fit
follows. At the fit the share reads the target and the ceiling stays put:
at a fixed batch the share spreads from 0.91 to 1.35 of its median, so a
share within 0.15 of the target is the measurement's noise, not room. With
the probe, the final rule, the same measurement (the host load easing from
nine fitting jobs to six over the runs, hence the higher numbers):

| run | fps | late | speed of set | picture wobble | cuts a second |
| --- | --- | --- | --- | --- | --- |
| old | 98.9 | 16.0 % | 0.764 | 25.9 % | 6.85 |
| new | 105.0 | 11.9 % | 0.749 | 19.1 % | 1.08 |
| old | 98.5 | 15.3 % | 0.766 | 26.9 % | 6.45 |
| new | 110.8 | 7.2 % | 0.800 | 16.3 % | 0.67 |
| old, light | 119.0 | 1.8 % | 0.995 | 6.0 % | 0 |
| new, light | 118.9 | 1.9 % | 0.994 | 6.3 % | 0 |

More frames, fewer late, the same speed or more, a third less picture
wobble, on both heavy pairs; the light scene unchanged. In the pacing
harness, whose display charges the draws on top of the pass, the run
without room to spare settles at seven steps a frame and its wobble fell
from 10.9 % to 4.9 %; with room to spare it reads 4.0 % against 3.8. The
ratchets are at 6 and 8 %.

Not measured: a quiet host, which the calibration run at the start of the
day had and nothing since (the fitting jobs ran from the fixed-batch sweep
on); the 60 Hz panel; a scene with adaptation, the overlay and the
integrated field; a GPU slowing under Low Power Mode, which is the case
the fit was built for. The threshold scene of 2026-10-04, blamed on some
frames and held on others, is what the adaptive target now does on
purpose: it hovers just under the threshold while the frames are not the
solver's.

## On a quiet host

5 October, 13:00, the fitting jobs gone (load 3): the same two binaries as
E4, the old rule and the budget, alternated, 60 s a run, two pairs a scene,
the window on the 120 Hz panel. The light scene is a tie (118 to 120 fps,
0.98 to 1.0 of its speed, 4.5 to 7.7 % wobble, within the run-to-run
spread). The heavy scene and the overlay scene (adaptation on, flow arrows,
integrated field) are not:

| scene | rule | fps | late | steps / s | still | wobble | cuts / s |
| --- | --- | --- | --- | --- | --- | --- | --- |
| heavy | old | 105.1 / 104.8 | 14.0 / 14.6 % | 451 / 424 | 16 % | 13.7 / 13.6 % | 14.6 / 15.0 |
| heavy | budget | 119.0 / 118.3 | 0.8 / 1.5 % | 328 / 337 | 19 / 20 % | 21.0 / 20.9 % | 0.14 / 0.23 |
| overlay | old | 105.5 / 105.2 | 12.8 / 13.5 % | 478 / 455 | 17 % | 21.7 / 22.6 % | 13.3 / 13.9 |
| overlay | budget | 115.8 / 116.6 | 2.2 / 2.4 % | 371 / 380 | 19 / 20 % | 25.1 / 23.4 % | 0.47 / 0.30 |

Steps a second are the completed count over the steady part of the run
(after 10 s); the overlay scene's adaptation chose different meshes for the
two builds (dt 2.62e-3 against 2.16e-3), so its speeds are not comparable
and its steps are. The budget keeps the cadence, as it is meant to, 14 %
late frames down to 1, and gives up a quarter of the speed for it; and the
wobble, which it halved under load, is worse here, 21 % against 14.

Both have one cause. A step costs 2.15 ms on the quiet host, against 1.07
ms the day before under the fitting load, on the same binary and scene, so
the budget fits three steps an interval where it fitted six, and four would
be late: three steps use 77 % of the interval and there is nothing between.
The old rule takes the rest by overrunning every seventh frame. A fixed
batch on the quiet host says the slow step is the machine's, not the
controller's: at six steps the step costs 1.87 ms, the pass 10.6 ms, 87 fps
at 37 % late; at eight, 2.06 ms and 65 fps. The chip clocks up under the
CPU load and the solver, memory-bound, rides on it; a quiet host gives the
GPU alone half the speed at any batch. Speeds compare within a day's pairs
only.

The wobble is the readback's. At 119 fps the picture stands still on a
fifth of the frames and then jumps (1.8 to 3 % of frames, against none
under the old rule), where at 105 fps it stands still less. The depth of
three in flight does not cover the round trip at 120 Hz, as the runs under
load already showed, and the budget, by reaching the cadence, exposes it.

## The readback's delivery

6 October, quiet host, 120 Hz panel, the budget. The depth was the
suspect, and it is not the cause. Three, four and five copies in flight,
alternated 3-4-5-5-4-3, 60 s a run:

| scene | depth 3 | depth 4 | depth 5 |
| --- | --- | --- | --- |
| heavy, picture still | 27.8 / 25.0 % | 24.9 / 25.9 % | 26.7 / 24.9 % |
| overlay, picture still | 25.7 / 25.2 % | 25.3 / 24.4 % | 24.0 / 26.1 % |

The solver and the copies are even: every frame encodes its steps and
takes a copy, and no copy is refused. The deliveries are not. The new
copies a frame received ran `1 0 2 1` over and over on both scenes: each
four frames got their four copies, one frame none and the next two. The
frame with none painted the old copy, the one with two painted only the
second and jumped two copies' worth. Most likely a copy finishes on the
device just after the frame polls for it and lands with the next; no
depth changes that.

So the picture plays its copies out (`picture_playout.rs`): every copy is
queued as it lands, the state's and the integrated field's, and the
painter takes one a frame. A lone copy waits a frame for a second, which
builds a reserve; the queue holds four at most and drops one when two or
more have stood after the release for 60 frames, so the lag a burst
builds does not last. Only the painter reads the played-out copy; the
snapshots, arrows and readouts keep the latest. The same pairs against
the main build:

| scene | build | still | serial a frame |
| --- | --- | --- | --- |
| heavy | main | 29.4 / 28.4 % | 0, 1, 2 and 3 |
| heavy | playout | 0.0 / 0.0 % | 1 |
| overlay | main | 23.4 / 25.5 % | 0, 1, 2 and 3 |
| overlay | playout | 1.5 / 1.5 % | 1, but at a handoff |

The overlay scene's remaining still frames are its adaptation's handoffs:
98 of 102 in one run fall within six frames of a new generation or a
frame that withheld stepping, where the queue starts again.

The cost is lag: the painted copy is 16 steps behind the newest control
readback at the median on the heavy scene, against 5 before, about two
frames more, 17 ms at 120 Hz. Against `1 0 2 1` that is the least a
playout can hold: one copy in reserve before the empty frame. The
summary's picture wobble barely moved (10.9 and 12.9 % against 13.1 and
14.0 on heavy) because it reads the copies' steps from the control
readback as each landed, which can be a copy off; the copies themselves
now advance one a frame, so the picture follows the encoded series, whose
own wobble is the budget's 4 or 5 steps a frame.

## Left for the pacing work
- The 60 Hz case and Low Power Mode, postponed (5 October): the machine
  has one panel and the mode is the user's to toggle.
- The integer step at a 2 ms step leaves a quarter of a 120 Hz interval
  unused; whether a second batch a frame, or a share above 0.8 where the
  draws leave room, could take it is a question for after the readback.
- 0.75 for the old rule's backoff survives only in the path without a cost
  of a step, a device without timestamp queries; on this machine the
  question is moot.
