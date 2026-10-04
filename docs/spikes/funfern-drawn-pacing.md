# The drawn cadence: what the eye sees, measured

Done on 2026-10-04 on `drawn-pacing-trace`, as the first of the two pacing
TODOs. Until now every smoothness figure came from the main world's requested
steps, and the one series that could be read back, the completed count, is
zero on most frames and then jumps.

## What a frame draws

A frame's solver pass and its drawing go into one command buffer, the drawing
sampling the field the pass has just written. So the state a frame draws is
the render world's encoded step count after that frame's encode, and the host
knows it exactly. It does not know when the frame was shown: wgpu 29 keeps the
Metal drawable private, so `presentedTime` is out of reach. Two clocks stand
in, each exact up to the vsync that follows: the render world's host time at
encode, and the GPU's at the start and end of the solver pass (timestamp
queries, which Bevy enables on every adapter that has them).

## The trace

`FUNFERN_PACING_TRACE=<path>` makes the render world write one CSV row a
frame (`crates/funfern-app/src/drawn_pacing.rs`): the drawn generation and
step, the requested and completed counts, the GPU pass times, and the main
world's pacing note for that frame (delta, speed, step, batch ceiling, steps
asked and admitted, running, withheld). Without the variable nothing is
created and the solver pass is encoded as before. Rows wait for their GPU
times, a ring of sixteen timestamp slots read back a few frames later.

```sh
env HOME=<scratch> FUNFERN_PACING_TRACE=trace.csv ./target/release/funfern-app
cargo run --release -p funfern-app --example drawn_pacing_summary -- trace.csv
```

The summary leaves out the first two seconds and measures only intervals that
run on one generation at one step. Its wobble is the pacing harness's: the
relative spread of simulated seconds per wall second over sliding windows of
eight frames, here on the drawn series, on the GPU clock, on the requested
series against the main world's deltas (the old proxy) and on the completed
count. "Away from handoffs" drops the windows a handoff withheld stepping in.

## What it showed

The default scene with the window on this machine's 60 Hz path (two 6K
displays and the built-in panel attached), 25 s runs, two each:

| scene | drawn wobble | away from handoffs | requested | completed | frozen |
| --- | --- | --- | --- | --- | --- |
| adaptation on | 9.1, 7.6 % | 6.9, 3.9 % | 9.2, 7.6 % | 11.2, 9.2 % | 2.2, 1.8 % |
| adaptation off | 10.7, 12.3 % | the same | 10.9, 12.4 % | 11.8, 13.2 % | 0 |
| off, edge 0.035 | 9.6, 9.0 % | the same | 9.7, 9.1 % | 11.4, 12.5 % | 0 |

- **The requested series was right.** On every run the render world encoded
  what was requested by the time it drew (requested ahead of drawn: 0 at p50
  and p90), so the proxy's wobble is the drawn one to within 0.3 points. The
  completed count is the misleading series: still on 51 % of frames.
- **Handoffs freeze the picture.** Each adaptation withholds stepping for
  three frames, 50 ms, and that time is not made up; it is all of the frozen
  share, and a quarter to a half of the wobble with adaptation on.
- **The rest is the ceiling's sawtooth.** With adaptation off the scene runs
  4 steps a frame on passes of about 5 ms in a 16.7 ms frame, and still dips:
  after a cut the batch runs `3,2,3,3,2,1,1,2,2,2,3,3,3,4,4,5,5`, a third of a
  second at about half speed.
- **Most cuts are not the solver's.** Over 60 s runs, 2.3 to 7.6 % of frames
  ran past 25 ms in the render world as in the main world, so the frames
  were really late. But in about 80 % of the cuts no solver pass within two
  frames ran past 12 ms (the median of the largest was 6 to 8 ms, the batch
  blamed 3 steps), where a frame is 16.7. The 2026-09-23 measurement with the
  batch pinned at one step found 1.7 % of frames past 1.5 times the refresh
  at 120 Hz; here late frames are more frequent, whatever makes them (the
  compositor driving three displays, other load: not measured), and each one
  cuts.
- **0.75 against 0.9 is still not resolved, and the reason is now visible.**
  Three alternating 60 s runs of each backoff on the adaptation-off scene:

  | backoff | wobble | cuts a second | drawn speed |
  | --- | --- | --- | --- |
  | 0.75 | 18.3, 22.0, 26.7 % | 2.7, 3.5, 4.5 | 0.87, 0.84, 0.80 |
  | 0.9 | 30.0, 27.6, 12.0 % | 8.6, 7.8, 1.6 | 0.73, 0.78, 0.93 |

  Within either build the wobble follows the cut rate, and the cut rate
  follows how often a frame comes in late, which moved threefold between runs
  minutes apart. It was the run-to-run variation the 2026-09-23 attempt
  could not see through, and it is not a measurement error: the eye sees it.
  The wobble at 60 s is also twice the 25 s runs', with the same build and
  scene; why was not measured.

## Left for the pacing work

The controller cuts on any late frame carrying steps. With the GPU's own pass
time now known a frame later, a late frame whose pass was a small share of the
refresh could be read as someone else's rather than the solver's. That is a
pacing change and was not made.
