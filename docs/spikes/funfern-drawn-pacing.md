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

## Left for the pacing work

- The controller cuts on any late frame carrying steps. The solver pass's GPU
  time is now known a frame later; a late frame whose pass was a small share
  of the refresh could be read as someone else's rather than the solver's.
  The drawing cannot be timed from here, but it runs beside the solver and is
  small on this device. A pacing change, not made.
- The picture is three frames behind the solver. Shortening that is a
  readback-path change, not a pacing one.
