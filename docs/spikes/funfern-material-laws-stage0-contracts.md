# Material-law overhaul: Stage 0 implementation contracts

Date: 19 September 2026. Numerical baseline: `beca47e`. Architecture and
equations: [material-law plan](funfern-material-laws-plan.md). Stage breakdown:
[implementation review](funfern-material-laws-review.md#5-implementation-stages).

This file fixes the operational contracts needed by the linear implementation.
It does not enable new solver behavior. Later stages may tighten a tolerance; a
looser tolerance or changed physical meaning needs an explicit review and log
entry rather than a silent fixture edit.

## 1. Source and legacy-loss migration

### Source units and spatial normalization

`Q` is an integrated nodal primary flux, so every ordinary source contributes to
`Qdot`. New point and volume source waveforms are authored in primary-field-rate
units. Their carriers are dimensionless and the generation reference mass turns
the authored rate into integrated nodal forcing:

```text
point:   S_i = Mref_i G_i(x0,width,trace side) r(t)
volume:  S_i = sum_e |e| w_ei Pref_e'(0) profile_e(x_i) r_e(t)
```

`G` is the existing peak-one Gaussian. Across separated traces its distance and
membership follow the selected physical side. A volume profile is evaluated in
each contributing region frame; contributions remain separate until nodal
gather. `Mref` and `Pref'(0)` are immutable data of the accepted generation.
Changing material coefficients prepares a new generation but does not
renormalize an already accepted generation in place.

A weak Neumann-style boundary drive is edge integrated, not mass normalized:

```text
S_i,boundary = integral_Gamma N_i j_Gamma ds.
```

Thus `j_Gamma` has primary flux per boundary length per time. A prescribed
primary boundary retains field units and follows the explicit `Delta Q` exchange
contract. Pulses retain field-increment units and use the constitutive delta from
the main plan. These three source kinds are not interchangeable UI units.

### Converted version-22 waveform and edit anchor

A version-22 point, volume, or weak-boundary harmonic acceleration is decoded as
`LegacyIntegratedHarmonic`; prescribed-primary signals are not converted. At a
reset anchor `ta`, its direct rate is zero and thereafter uses the analytic
antiderivative in the main plan. Serialization of this changed meaning requires
the next document version; a version-22 file is never reinterpreted in place and
resaved as version 22.

At an edit committed at solver time `te`, store the old instantaneous direct
rate as `rate_anchor` and integrate the new acceleration from zero elapsed time:

```text
r(t) = rate_anchor + a0 (t-te)
     + A (t-te) sinc(omega (t-te)/2)
         sin(phase_at_te + omega (t-te)/2).
```

A frequency-only edit preserves `phase_at_te`; an explicit phase edit may jump
the acceleration but not the already integrated rate. Offset/amplitude edits
also keep rate continuous. Reset clears the anchor. A carrier edit immediately
uses the current carrier and retained rate; it creates no spatial source history.
The frozen run-start common envelope is not restarted by these edits.

### Legacy single loss rate

Version-22 `damping` is a normalized primary-field rate. On migration assign the
full symbolic field to exactly one physical channel:

| Stored scene skin | Migration target |
| --- | --- |
| Mechanical | magnetic loss `gamma_H` (the TE adapter's primary channel) |
| EM TE | magnetic loss `gamma_H` |
| EM TM | electric loss `gamma_E` |

The other channel is zero. Never copy the rate into both channels. A later skin
change preserves these physical channel names, so it need not preserve the old
primary-only damping trajectory. The Stage 1 schema may store optional electric
and magnetic channels, but the legacy scalar `damping` remains the old solver's
field until Stage 3 performs this versioned migration.

## 2. Trace histories and intersections

### Stable trace identity and orientation

A trace sample key is `(boundary kind, stable object/span id, physical side,
oriented span parameter)`. Outer sides use their stable side and increasing
boundary parameter. Separated curves include the side; transmitting traces do
not manufacture two keys. Coincident coordinates alone never establish history
identity.

Thin-gap jump history is oriented left minus right in the authored curve
orientation. Reversing the matched orientation negates the transferred history.
An unchanged trace copies exactly. A split restricts/interpolates the parent
history; a merge uses the unique side-compatible donors and rejects ambiguous
many-to-one matches. A genuinely new gap initializes to zero; deletion removes
its stored energy and reports that amount as edit exchange. Prescribed values
enter `zdot=T u`; they do not reset an existing gap history.

### Outgoing auxiliary memory

The accepted state stores energy-normalized modal `z`. For transfer, first form
raw pole state `x=H^(-1/2)z`. For each of the three poles form the physical
pole-current trace

```text
m_p = V diag(sqrt(d)) x_p.
```

This representation is invariant to modal signs, permutations, and rotations
inside a degenerate eigenspace. Transfer each `m_p` by the stable physical trace
map, then recover admitted new modes with the minimum-energy projection

```text
x'_p = diag(sqrt(d'))^+ V'^T m'_p,    z'=H^(1/2)x'.
```

Zero-`d` modes carry no auxiliary current. Components not representable by the
new operator and energy introduced/removed by the projection are explicit edit
exchange diagnostics. Unchanged operator/trace data must copy the physical
memory exactly within the f32 parity tolerance. A new outgoing trace starts
unexcited; deleting one accounts its remaining auxiliary energy.

At an outgoing/prescribed intersection, prescribed-primary ownership wins for
the shared DOF. Eliminate it from the outgoing unknown trace, use its held value
as known forcing, and attribute work to prescribed exchange. No auxiliary state
is stored on that eliminated DOF. Maximal outgoing trace components and corners
are assembled before modal decomposition; a geometric corner is not an implied
history reset.

## 3. Scalar/vector transfer policy

An exact `Q` copy requires the same stable DOF and unchanged geometric support.
Otherwise transfer density `Q_i/v_i`, integrate it over the new support, and
correct only the connected affected free component. Prescribed DOFs, different
baffle sides, and different material-interface sides are ineligible correction
support.

The correction scale is

```text
Qscale = sum_affected |Qraw_i| + q_floor sum_affected v_i,
q_floor = max(global RMS |Q_i|/v_i, 1e-12 in normalized units).
```

Reject if `|delta component total| > 0.05 Qscale`, if any corrected state leaves
its constitutive domain, or if the correction would need support outside the
affected component. Distribute an admitted correction by positive geometric
support and available domain margin. Exact component conservation tolerances are
`1e-11 Qscale` in f64 and `5e-6 Qscale` in f32.

Transfer `b` by a physical-coordinate quadratic reconstruction from the old six
quadrature samples, restricted to the same material side and baffle trace. An
unchanged element/sample is an exact copy. A connected new cell takes a bounded
constant-preserving extension from donors no farther than two element rings; a
new island is zero. There is no global harmonic relaxation.

On the irregular production fixtures, predeclared limits are:

- smooth one-handoff weighted relative L2 error: at most 3% for `Q` and `b`;
- twelve alternating refine/coarsen handoffs: at most 5%;
- stationary-force leakage relative to the transferred field scale: at most
  0.1%;
- unchanged-support scalar values and unchanged six-sample vectors: bit-exact
  before f64/f32 representation conversion.

Manufactured polynomials within the reconstruction order remain subject to the
much tighter existing exactness fixtures; the percentages above are not a license
to weaken them.

## 4. Event order, ownership, clock, and failures

At each accepted step boundary, consume one revision-ordered event batch in this
order:

1. finish validation of the previous candidate or honor its failure latch;
2. accept a prepared topology/material/skin generation atomically;
3. apply prescribed-primary exchange under the accepted maps;
4. commit source/drive edits and phase/rate anchors;
5. apply Switch gestures or ramp reversals;
6. apply pulses;
7. apply scheduled filter/maintenance;
8. start the next conservative step.

Events with the same class retain document/event serial order. A topology and a
pulse in one batch therefore apply the pulse to the new admitted map. Paused
simulations service steps 1–7 as a zero-duration commit and do not advance time.

Accepted state owns `Q,b`, all physical histories, accepted law/runtime tables,
clock, phase/rate anchors, caches, serials, invariant accounting and status.
Candidate state owns a complete separate set. Validation writes one ordered
global result with priority `non-finite > inverse domain > timestep/envelope >
layout/reference > success`; acceptance swaps the complete set. No candidate
writer mutates accepted state.

The canonical host clock is `(epoch: u64, epoch_origin_seconds: f64,
step_in_epoch: u32, dt: f64)`. GPU records pack the epoch into two `u32` lanes and
use bounded local elapsed time. Rebase at a commit boundary before
`step_in_epoch` reaches `2^16` or local elapsed time reaches 256 seconds. Phase
anchors are reduced at rebase. A rebase changes no physical state or event
ordering.

An invalid edit leaves accepted evolution running. A runtime failure rejects the
whole candidate step, latches the highest-priority reason, and pauses on the last
accepted state. Editing, preparation, reset and resume remain available. No
default clipping, partial commit, boundary-memory reset, or automatic state reset
is permitted.

## 5. Consumer and serialization inventory

| Consumer/state | Required migration owner |
| --- | --- |
| evolution and state buffers | `wave_gpu.rs`, `wave.wgsl`, shared layout tests |
| topology candidate/commit and pacing | `topology_runtime.rs`, `topology_editor.rs`, `document.rs` |
| latest-state transfer | `transfer.rs`, both transfer shaders, `wave_gpu.rs` |
| thin-gap/outgoing histories | canonical core, boundary preparation, transfer shaders |
| scalar/vector rendering | `wave_gpu.rs`, display extraction, `ui.rs` |
| point/curve/area probes | `probe.rs`, probe shaders and recorder histories |
| far field | `far_field.wgsl`, contour eligibility/history and handoff |
| AMR residuals | `indicator.rs`, topology AMR scheduling and snapshots |
| energy, flow and material overlays | canonical diagnostics, `material_overlay.rs`, UI labels |
| sources and pulses | `volume_source.rs`, forcing compilation, source UI/help |
| persistence/undo | `topology_persistence.rs`, document/editor transactions |

Probe history is retained only when its physical observable and units are
unchanged; otherwise start a labelled segment. Far-field history additionally
requires compatible exterior law, contour and wave speed. No consumer may infer
the new state from old displacement levels after the common-core cutover.

Version 22 remains readable with inert law defaults during Stage 1. The source
waveform and legacy-loss meaning changes above require a new version at their
implementation stage. Runtime switch state, phase anchors and candidate state are
not document data.

## 6. Capability and acceptance matrix

Initial authoring may persist structural laws, but the legacy solver accepts only
inert laws. Linear scalar/tensor maps are the first executable common-core
capability. Kerr and saturable laws are next after the nonlinear gates. Signed
`chi1`, general reciprocal nonlinear maps, nonlinear anisotropy, van der Pol and
all restoring/oscillator forms remain unavailable until their named gates close.
Validity of a formula is never equivalent to executable support.

Production f32 gates, in addition to the pinned spike thresholds, are:

- CPU/GPU `Q,b` weighted relative L2 difference at a matched accepted time:
  `<=3e-5` for 1,000 linear steps and `<=1e-4` for the longest release fixture;
- lossless relative energy drift: `<=2e-4` in f32 after subtracting declared
  source/boundary/edit work; no passive test may show growth above
  `5e-6 max(E0,1)` per accepted step;
- corrected planar boundary complex-reflection error versus the discrete oracle:
  `<=1.5e-2`, with fixed-CFL refinement reducing error until the spatial floor;
- basis sign/order/degenerate-rotation history tests: physical trace difference
  `<=2e-5` in f32 and `<=1e-11` in f64;
- global failure injection: zero accepted-state byte differences other than the
  latched status record.

## 7. Target-device performance budgets

The named target remains the Apple M1 Max / Metal machine used by the repository's
production enriched-quadratic measurements. The clean baseline is 9,326 DOFs,
`dt=0.00479230`, 128 steps in 36.86 ms (16.64 simulated seconds per wall second),
with 21.1–23.3 ms GPU transfer and 58–72 ms request-to-commit. These figures come
from [architecture.md](../architecture.md#initial-evolution-model) and the
[engineering log](../engineering-log.md#2026-09-09--production-enriched-quadratic-gpu-solver),
not from the dirty authoring branch.

At matched accuracy on that case, the linear common core must meet:

- reflecting/first-order steady throughput at least 75% of baseline;
- steady accepted-state memory at most 1.5x baseline and transaction peak at
  most 2.0x, with boundary dense factors reported separately;
- same-mesh handoff GPU commit at most 35 ms;
- ordinary request-to-commit at most 100 ms and no cooperative CPU slice above
  12 ms on the standard case.

For second-order outgoing boundaries, report `Nb`, transform/factor bytes,
preparation, trace-solve time and throughput separately. Initial acceptance is at
least 5 simulated seconds per wall second for the standard box and at most 250 ms
request-to-commit; exceeding either is a review point, not permission to silently
downgrade the boundary. Also measure the existing eight-obstacle case. Stages 2–6
record these real-core measurements incrementally; nonlinear additions report
their delta against the accepted linear core.
