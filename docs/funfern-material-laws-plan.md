# funfern: shared field core and material laws

**Specification and implementation plan — 19 September 2026**

**Code baseline:** [DmitriyNE/funfern, beca47e0d05c9b24945ef632b00c5cc16f80c8a0](https://github.com/DmitriyNE/funfern/tree/beca47e0d05c9b24945ef632b00c5cc16f80c8a0).

This specification defines a shared time-domain field solver, nonlinear and time-driven material laws, dynamic editing, GPU state transfer, material authoring, and the implementation and verification work required to deliver them.

**Status and terminology:** this is a design specification. Requirements state the intended behavior; passages marked recommended describe implementation choices. A **design gate** identifies a remaining derivation or engineering decision that must be completed before the dependent feature is enabled. The final phase completes the named oscillator and active-medium catalogue.

**Adopted architecture — implementation in progress:** use integrated nodal primary flux `Q`, independent two-component complementary flux `b` at the six element quadrature points, and explicitly physical trace/oscillator memory. There is no production bulk potential, potential gauge rebasing, or seam-offset solve. Authoritative mechanical `s₀`, named electric/magnetic loss channels, analytic legacy-source migration, phase anchors, frozen-rate passive loss, value-continuous ramp reversal, and recoverable global acceptance are agreed. The [core spike](funfern-material-laws-spike-report.md) justifies direct state over potential-plus-loss memory. Stages 0–4 are complete: contracts and inert authoring are frozen; the f64 direct-state CPU reference composes linear sources, physical losses, thin gaps, both outgoing orders, the paired filter, and bounded scalar/vector/physical-history transfer; and the production-intended f32 GPU core closes the linear parity, clock, event, rollback and target-device performance gates recorded in the [Stage 4 report](funfern-material-laws-stage4-report.md). The old production solver remains the application path until transfer and consumer migration close, and authored non-inert laws remain explicitly non-executable.

**Adopted outgoing boundary:** retain first order and implement the passive three-state-per-mode rational candidate as the new second-order option. The [auxiliary report](funfern-boundary-auxiliary-spike.md) supplies its equations and energy proof. Its original boundary-only time split is superseded by the force-coupled kicks below. No blanket nonlinear-domain ban is adopted. The candidate is spatially nonlocal, not exact curved DtN; its accuracy/cost tradeoff is accepted for implementation, not a claim that production performance or all boundary geometries have passed.

**Single-boundary scattering follow-up:** [248 checks now pass](funfern-boundary-scattering-spike.md), confirming the candidate's predicted planar reflection and grazing improvement over first order. Legacy second order remains more accurate in the isolated lossless planar comparison, but retains its known robustness failures. The follow-up exposed a fixed-CFL reflection floor in the previous separate boundary split (21 retained failures). Use the corrected **force-coupled boundary midpoint kicks**, with explicit interior drift, as the implementation reference. Do not copy the superseded split from the original auxiliary prototype. Production performance, f32, full nonlinear composition and boundary-history transfer remain implementation gates.

This specification is authoritative for the selected design; the [review and detailed stages](funfern-material-laws-review.md) define work packages. Spike reports preserve historical findings, including failed alternatives, and do not override this consolidated decision. Stages 0–4 are complete; Stage 5 is the next implementation boundary. No further broad spike is prerequisite; numerical gates close before their dependent feature is enabled, not after production cutover.

## 1. Product contract

funfern is an interactive time-domain toy. Visual continuity, responsive editing, and bounded numerical behavior take priority over constructing a physical interpretation for an instantaneous geometry or skin change.

The implementation must:

- Use one evolving field formulation in Mechanical, EM TM, and EM TE. Mechanical is a presentation of the same core, with mechanical names and no requirement to expose EM vector controls.
- Keep ordinary edits and skin changes dynamic. Preserve live canonical state; do not reset the field or reconstruct an EM state when an overlay or skin is enabled.
- Prepare geometry, operators, transfer stencils, and law metadata while the accepted generation continues evolving.
- Apply a prepared transition to the latest GPU state at a complete timestep boundary. Field and physical-history transfer must not introduce a GPU → CPU → GPU field-calculation round trip.
- Preserve the continuous solver clock and source phase across handoff.
- Remove filtered inverse-derivative reconstruction and display-only component mean subtraction.
- Treat amplitude bounds as validity/stability contracts, not as permission to clip fields silently.
- Keep the portable limit of eight storage buffers per shader stage.
- Deliver the material laws, loss laws, oscillator media, presets, and editor controls specified below. Unsupported combinations are rejected explicitly; their named features remain scheduled behind the relevant design gates.

Frequency-domain analysis, an eigensolver product feature, IGA, full 3D Maxwell fields, and a general vector-field painting tool are outside this implementation. Independent responses to gradients of the primary scalar observable are also outside scope; evaluating the actual complementary physical field is part of the core. Small eigenproblems used to verify the solver are ordinary tests, not product features.

## 2. Canonical state and constitutive equations

### 2.0 Selected direct state and two-sided loss

Electric loss remains attached to the electric field/material response and magnetic loss to the magnetic response when changing skins. TM/TE assign these channels to primary/complementary roles; Mechanical uses the TE adapter. Do not implement different production solvers per skin or relabel a primary-only loss as polarization-independent absorption.

With spatially varying complementary loss, a flux represented solely as `±Cψ` is generally insufficient: its local dissipative evolution need not remain in that operator's image. Store nodal integrated primary flux `Q` and independently evolved complementary flux `b`, with two components at each of six quadrature points per element. With `η=+1` for TM and `η=-1` for TE/Mechanical, the interior equations are

\[
u=U(Q,t),\qquad v=\mathcal C^{-1}(b,t),
\]
\[
\dot Q=-\eta C^\mathsf T Wv-\mathcal L_{\rm primary}+S,
\qquad \dot b=\eta Cu-\mathcal L_{\rm complementary}.
\]

Here `b=B⊥, v=H⊥` in TM and `b=D⊥, v=E⊥` in TE; `𝒞` is the corresponding constitutive map. With complementary loss absent and `b=ηCψ` initially, this reproduces the lossless potential reference's interior evolution. That relation is a comparison fixture, not a constraint on evolving production state. The energy is defined in section 6.3.

The [core spike, sections 1–3](funfern-material-laws-spike-report.md) establishes compatible linear equivalence and demonstrates legitimate loss-generated nonpotential flux. Extra stationary modes also admit persistent discretization/remap artifacts. Test both; do not project all nonpotential state away or claim arbitrary Maxwell constraints are automatically enforced. Thin gaps and outgoing conditions retain genuine physical history (section 4.3). General vector painting/background-field authoring is not part of this rollout.

The potential-plus-loss-memory fallback is not selected: it inherits the old boundary instability and adds cancellation/rebasing cost without demonstrated benefit. Keep potential notation only in analytical compatibility fixtures or explicitly derived future oscillator models, never as a second production state path.

### 2.1 State and discrete operators

Authoritative state is synchronized `(Q,b)` plus physical auxiliaries and runtime/event records. Cached `u`, `v`, forces, display samples, and intermediate stages are derived or scratch data. Accepted and candidate copies have separate ownership until global validation.

Let:

- `G` be the enriched quadratic finite-element gradient at quadrature points.
- `R(x,y)=(-y,x)` and `C=RG`.
- `W` contain element/quadrature integration weights.
- `P_i(u,t)` be the assembled nodal constitutive map.
- `U_i(Q,t)=P_i^{-1}(Q,t)`.
- `𝒟:E↦D` and `ℬ:H↦B` are the electric and magnetic constitutive maps; their inverses recover field observables from fluxes.

`Q_i` is an integrated nodal quantity. It is not interchangeable with a pointwise `D` or `B` value.

For isotropic media:

| Skin | Scalar observable `u` | Nodal map `Q=P(u,t)` | Complementary flux | Complementary observable |
| --- | --- | --- | --- | --- |
| EM TM | `E_z` | Lumped `D_z(E_z,t)` | Independent `B⊥=b` | `H⊥=ℬ⁻¹(b,t)` |
| EM TE | `H_z` | Lumped `B_z(H_z,t)` | Independent `D⊥=b` | `E⊥=𝒟⁻¹(b,t)` |
| Mechanical | Same scalar variable under the mechanical presentation | Same canonical construction | Maintained internally | Presentation-dependent |

The mechanical adapter uses authoritative `s₀=ε=1/k₀` and `μ=ρ`, which maps its fixed linear scalar equation to the TE core. Legacy mechanical damping `σ` supplies a reference rate `σ/ρ`; loss migration must explicitly assign legacy single-rate data to the new named channels, preserving the saved skin's intended linear behavior where supported. Do not silently apply the full legacy rate to both channels. Switching to TM can change the scalar dynamics through the polarization's parameter interpretation, but still transfers the live state.

For a node shared by several element/material instances,

\[
P_i(u,t)=\sum_{e\ni i}|e|\,w_{e,i}\,\mathcal P_e(u,x_i,t).
\]

Here `|e|` is the triangle area and `w_{e,i}` its positive nodal lumping weight. The discretization is a seven-node quadratic triangle enriched with a cubic bubble: weights are `1/20` at each vertex, `2/15` at each edge midpoint, and `9/20` at the bubble node. They sum to one. Stiffness uses the six-point element quadrature.

Keep each contribution needed to evaluate this sum correctly. A shared `MaterialId` is insufficient grounds to combine contributions if their region frames, sampled coefficients, or travelling-drive phases differ. Combine only algebraically equivalent evaluated contributions.

For linear static media, `P(u)=Mu` with positive diagonal `M`.

### 2.2 Lossless evolution

\[
\dot b=\eta C U(Q,t),\qquad \dot Q=-F(b,t),\qquad
F(b,t)=\eta C^\mathsf T W\mathcal C^{-1}(b,t).
\]

Use matching discrete operators and signs in the solver, probes, vector overlays, and energy diagnostics.

For a fixed linear complementary inverse tensor `J`, define `K=CᵀWJC`. Compatible reference data must reproduce the existing stiffness matrix:

\[
F(\eta C\psi)=K\psi.
\]

Keep the current enriched quadratic basis and quadrature. Existing linear anisotropy must reproduce the current tensor stiffness, including its frame rotation; do not replace it with an isotropic approximation. Nonlinear anisotropy needs the constitutive contract in section 5.3.

Direct state also represents flux outside `image(C)`. In fixed linear media, `ker(CᵀWJ)` is stationary under unforced lossless evolution. Its behavior under losses, remaps, and material changes is part of validation, not a reason to reconstruct or discard it.

### 2.3 Scalar wave equation and zero modes

For fixed positive `M`, fixed symmetric positive-semidefinite `K`, and no sources or losses:

\[
M\ddot u+Ku=0.
\]

The direct formulation reproduces the scalar trajectories whose initial data satisfy

\[
\eta C^\mathsf T WJb_0=-M\dot u_0,\qquad Q_0=Mu_0.
\]

On each free connected component, solvability requires

\[
\sum_{i\in c}m_i\dot u_i(0)=0.
\]

Every nonzero spatial eigenmode retains both phase-space freedoms; a compatible initializer can solve `Kψ₀=-M u̇₀` and set `b₀=ηCψ₀`. The scalar second-order equation admits free uniform motion `a+bt`; unforced direct state represents the bounded branch `a`. A static uniform `u` remains. Components with invertible stiffness lose no scalar degree of freedom. Direct `b` additionally carries stationary complementary state absent from the scalar description.

This equivalence concerns scalar evolution. The canonical energy is defined in section 6.3; source, loss, temporal-material, and nonlinear-restoring behavior follows their own equations.

With uniform linear damping, the scalar second-order equation admits `a+b exp(-γt)`, while direct first-order loss retains the decaying branch. The excluded combination therefore depends on the loss model.

## 3. Time integration and event ordering

### 3.1 Reference conservative step

Use the following second-order kick–drift–kick scheme as the CPU reference for the smooth, lossless equations:

\[
Q_{n+1/2}=Q_n-\frac{\Delta t}{2}F(b_n,t_n),
\]

\[
b_{n+1}=b_n+\eta\Delta t\,C U(Q_{n+1/2},t_n+\Delta t/2),
\]

\[
Q_{n+1}=Q_{n+1/2}
-\frac{\Delta t}{2}F(b_{n+1},t_{n+1}).
\]

Display `u` at an explicitly documented synchronized stage; an endpoint display uses `U(Q_{n+1},t_{n+1})`. Do not mix half-step and endpoint fields when constructing Poynting vectors or diagnostics.

A cached endpoint force can supply the next step's first kick when its state/law generation still matches. The direct-state implementation uses quadrature evaluation and deterministic nodal gather; it does not inherit a one-CSR-per-step cost claim. Any incremental linear force-cache optimization must preserve independent `b`, include loss/filter/event invalidation, and earn its place through parity and actual-core measurements.

At a linear outgoing trace, combine the held interior force with the boundary midpoint kick. For boundary state `X=(QΓ,z)` and its linear generator `GΓ`, each kick of duration `τ=Δt/2` solves

```text
[I-(τ/2)GΓ] Xnew = [I+(τ/2)GΓ] Xold - τ [FΓ,0aux].
```

Use the updated half-stage primary field in the explicit full `b` drift, then kick with the new force. Include prescribed/source inputs at their declared stages and test their normalization. Eliminate auxiliaries to a reduced trace solve; the interior is not globally implicit. The [scattering derivation, sections 3–4](funfern-boundary-scattering-spike.md) explains why a separate boundary-only split has a fixed-CFL reflection floor. For nonlinear primary traces, derive the force-coupled discrete-gradient counterpart and test the full composition before enabling that combination; the earlier standalone nonlinear boundary proof alone is insufficient.

Invalidate/recompute force caches after topology changes, constitutive edits, discontinuous drive events, skin changes, complementary-loss updates, or a filter operation that changes the force input.

The method is second order for smooth coefficients. It does not exactly conserve energy, and static CFL compliance is not a proof of stability under arbitrary modulation.

### 3.2 Discontinuities and switches

A hard event occurs between complete steps:

1. Finish the old step using the old constitutive law, including its endpoint force.
2. Keep free `Q`, independent `b`, and unchanged physical histories intact.
3. Install the new law/event state.
4. Re-evaluate constitutive observables and the force under the new law.
5. Start the next step.

On an unchanged mesh this preserves integrated nodal primary flux `Q` on unconstrained nodes and complementary flux `b`. Constitutive observables may jump. At material junctions, conservation applies to the assembled nodal flux; it does not assert separate pointwise flux continuity in every incident material. It does not require `Q̇` to remain continuous. A changed boundary operator additionally follows the physical-history mapping/admission rules in section 8.

Prescribed-primary nodes are an explicit exception: after installing the law, enforce `Q=P_new(g,t)` and account the accepted change from the pre-event `Q` as external boundary exchange. This also applies when an edit introduces a prescribed node. Preserve `b` at the instantaneous event, then use prescribed `u=g` in the drift and physical-history evolution. An event whose prescribed field is outside the new law's domain is rejected transactionally. Flux-continuity and identity-transfer tests must distinguish these constrained nodes from unconstrained ones.

A user Switch is stamped by the GPU at its actual application boundary. While paused, service it at a zero-duration complete-step commit boundary without advancing the solver clock. It must not estimate the GPU time from the host.

Hard periodic edges must also have an explicit boundary-placement policy. Smooth square drives need adequate temporal resolution. Sampling a discontinuity inconsistently at the two kicks is not an acceptable substitute for event ordering.

### 3.3 Clocks and runtime law state

All stage-time consumers use one GPU-owned clock contract. Avoid repeated reinterpretation of historical levels under newly evaluated laws.

Use integer step/epoch bookkeeping with a bounded conversion interval and phase reduction for periodic drives. Before implementation, specify the actual integer/float layout, epoch rollover, and stage-time calculation; test long runs and timestep changes. Do not rely on an indefinitely increasing f32 integer counter remaining exact.

Per-material runtime records include stable identity/serial mapping, switch target, ramp start state, start time, and duration. They are separate from authored material parameters.

Sources and material drives carry a temporal phase/time anchor, `θ(t)=θ_a+ω(t-t_a)`, evaluated through the bounded clock and phase-reduction contract. At a frequency edit's GPU commit boundary, evaluate the old instantaneous phase, make it the new anchor, and install the new frequency. This is scalar event work per edited drive, not per-node field work or a host-time estimate. Preserve anchors through handoff/retiming. An explicit phase edit intentionally changes phase; a travelling drive's spatial phase retains its documented frame rule. Carrier-phase continuity does not assert unchanged amplitude after another parameter changes.

For a ramp of duration `τ>0` starting at `t_s`, use the C² smootherstep

\[
z=\operatorname{clamp}((t-t_s)/\tau,0,1),\qquad
s(z)=6z^5-15z^4+10z^3,
\]

\[
h_{\rm switch}(t)=h_{\rm start}
+(h_{\rm target}-h_{\rm start})s(z).
\]

Handle `τ≤0` as a hard event without dividing by `τ`. The factors before and after an event are defined at every stage, including when a serial has not yet been applied.

Ramp policy:

- Transfer an active ramp through handoff at its current phase.
- If switched again mid-ramp, start the new ramp from the currently evaluated factor. Preserve value continuity and accept the derivative discontinuity; the C² smootherstep property applies to an uninterrupted segment joined to constant endpoints, not arbitrary mid-ramp reversals. Do not add derivative-history state solely to smooth a reversal.
- Keep runtime state when any source/pulse edit replaces the forcing buffer.
- Map records through stable material IDs, never editor-local array positions.
- Reset runtime switch state on document load and explicit Reset.
- Keep Switch out of document history and document dirtiness.

An in-place law update is allowed only when its compiled topology/data layout remains valid and the active timestep covers the new admissible range. Before activation, validate the latest GPU `Q` and complementary flux against the candidate law's inverse domains. Keep the accepted law and state available until validation succeeds; on failure reject the patch through the normal status path. Distinguish a state outside the new inverse domain from a valid state requiring a smaller timestep: retiming cannot fix a nonexistent inverse. The same admissibility check applies to topology candidates. This adds no CPU field calculation. If a new timestep/layout is needed, prepare a new generation and transfer the same canonical state.

An edit to alternate factors during a ramp must validate the current factor and the full proposed ramp trajectory, not only the new base/alternate endpoints. Preserve the underlying factor trajectory through reciprocal views; a reciprocal of a ramp is generally different from a new ramp between reciprocal endpoints.

## 4. Sources, losses, boundaries, and initialization

### 4.1 Direct first-order sources

The new source slot is

\[
\dot Q=-F(b,t)+S(t,\text{position},\text{authored state})-\mathcal L.
\]

Evaluate sources from their current authored carrier and GPU time. Do not introduce a persistent spatial antiderivative of a moving source merely to reproduce a legacy acceleration-source API: that would reintroduce stationary footprints after carrier edits.

For fixed `M`, an integrated scalar force `f_int` in `Mü+Ku=f_int` has the same driven scalar evolution when `Ṡ=f_int`. A per-mass acceleration `f_acc` instead requires `Ṡ=M f_acc`. Reusing the acceleration waveform directly as `S` changes both the source contract and its units.

**Accepted source migration; design gate S — normalization and verification.** New point/volume sources use direct primary-field-rate units, compiled to integrated `S` using immutable generation reference masses and the current spatial carrier. For a legacy per-mass acceleration

\[
a(t)=a_0+A\sin(\omega t+\phi),
\]

store an explicitly versioned converted direct-rate waveform

\[
r(t)=a_0t+\frac A\omega\bigl[\cos\phi-\cos(\omega t+\phi)\bigr],\quad\omega>0.
\]

It obeys `r(0)=0` and `ṙ=a`. At zero frequency use the exact limit `r(t)=(a₀+A sin φ)t`. Evaluate small frequencies without cancellation, for example using the equivalent oscillatory term `A t sinc(ωt/2) sin(φ+ωt/2)`, where `sinc(z)=sin(z)/z` has a stable zero limit. This is an analytic temporal waveform evaluated on the current carrier; it introduces no persistent spatial antiderivative field. Define its runtime anchor behavior under source edits alongside phase anchoring.

For a fixed linear reference mass, use `S_i=(M_ref)ᵢᵢ carrier_i r(t)`. This preserves the legacy forcing relationship outside the explicitly changed startup/carrier-edit behavior. Legacy DC acceleration legitimately produces a growing direct source; do not silently replace it with constant forcing or clip it. New sources use direct-drive semantics. Prescribed-primary boundaries retain field units and do not undergo this acceleration-waveform conversion; weak boundary loads require their own edge normalization.

Before the common core replaces the production solver:

- Specify source amplitude units and normalization for point, volume, and boundary sources.
- Implement and round-trip the converted waveform, including offset, zero/near-zero frequency, and its source-edit anchor policy, in explicit schema/version handling.
- Update source help, presets, tests, and CPU/GPU envelopes together.
- Preserve absolute phase and current carrier at handoff.

Use a direct first-order drive with a shared smooth startup envelope for continuous point, volume, and driven-boundary sources. At run initialization/Reset choose `τ_source=4/f_min` for the slowest active oscillating source (`SOURCE_RAMP_PERIODS=4`); when none oscillates, use no startup envelope. Keep that envelope schedule through ordinary source/material edits and handoff: adding a slower source must not re-attenuate an established run. For `τ_source>0`, use `z=clamp(t/τ_source,0,1)` and `e(z)=z²(3-2z)`. A shared envelope preserves relative source phases; do not give different members of a phased array independent startup delays.

Source-envelope conventions must agree in CPU evolution, GPU evolution, initialization/transfer, and diagnostics, including startup and carrier edits. Exact reproduction of legacy acceleration-source startup transients is not a release requirement.

A pulse authored as a scalar increment `δu` becomes

\[
Q_i\leftarrow P_i(U_i(Q_i,t)+\delta u_i,t),
\]

with complementary `b` and physical histories unchanged unless a future pulse tool explicitly excites them. Numerically, preserve the authoritative `Q` despite finite inversion tolerance: evaluate `û=U(Q,t)` and apply

\[
\Delta Q_i=P_i(\hat u_i+\delta u_i,t)-P_i(\hat u_i,t),
\qquad Q_i\leftarrow Q_i+\Delta Q_i.
\]

A zero pulse must be an identity operation. Validate the requested field and resulting state against the constitutive domain and compiled timestep envelope before committing the pulse. Account only the accepted `ΔQ`.

### 4.2 Loss/current laws

Use one direct-state formulation with independently authored electric and magnetic loss channels. Electric loss uses the electric observable and magnetic loss the magnetic observable in every skin. TM/TE swap which channel is nodal and which is complementary; Mechanical follows the TE adapter. The primary-only convention is superseded; the [core spike](funfern-material-laws-spike-report.md) supports passive two-sided loss and documents the frozen-rate accuracy tradeoff.

For admitted isotropic passive rate laws, use loss currents `J_loss=γ_E(E,t)D` and `M_loss=γ_H(H,t)B`, with nonnegative rates and positive constitutive energy response. These are toy flux-rate laws, not a claim that γ is an Ohmic conductivity in all nonlinear media. State the arguments and rate units in the editor. The primary-channel convention is

\[
\mathcal L_i=\gamma_i(u,t)\,Q_i.
\]

With only the primary channel active in fixed linear media, this matches the existing damping-ratio convention. At mixed nodes, assemble the loss from all material contributions,

\[
\mathcal L_i=\sum_{e\ni i}|e|w_{e,i}\,
\gamma_e(u_i,x_i,t)\,\mathcal P_e(u_i,x_i,t).
\]

A single effective nodal rate, if used, must reproduce this sum; do not average nonlinear rate laws independently of their constitutive weights. For nonnegative rates and passive monotone constitutive contributions through the origin, the loss contribution to canonical energy is dissipative.

Use a frozen-rate exponential loss substep: evaluate the assembled effective passive rate at the input state/time, then update the relevant flux by `exp(-γ_eff Δt)`. For a mixed scalar node, derive the rate from the assembled loss divided by authoritative Q, with a finite constitutive zero-field limit rather than a `0/0`; complementary isotropic loss uses the physical vector field at its sample. This adds no implicit loss solve beyond the constitutive evaluation already required.

The exponential is exact only when the effective rate is constant during that substep. Different constant material rates at a nonlinear junction can produce a field-dependent effective rate. Accept generally first-order accuracy for varying nonlinear/driven losses; retain the second-order lossless conservative step. The core spike measures this distinction; repeat accuracy/dissipation checks for the actual source/loss/boundary composition. Do not require an expensive higher-order loss solve unless measured behavior warrants it.

Field-dependent or driven loss follows this first-order equation and the specified local integrator. Its rate controls and time-drive composition must have explicit units and field arguments.

Passive saturable absorption and polynomial rate laws remain in scope. Negative-rate/active laws need their own amplitude, growth-rate, and integration bounds. A generic negative damping law must not be advertised as a van der Pol oscillator without its derivation.

### 4.3 Boundaries and physical auxiliary state

Preserve the product's boundary types, with explicit first-order equations:

- Prescribed primary `u=g(t)` sets `Q=P(g,t)` at declared kick/drift/output stages, uses `g` in the complementary drift, and accounts the applied boundary exchange. Homogeneous `g=0` requires no hidden potential reference.
- Natural conditions and weak boundary drives supply the appropriate signed edge-integrated term in `Q̇`; derive normalization with the physical power pairing.
- PEC sets `E_z=0` in TM and zero tangential complementary E in TE; PMC sets zero tangential complementary H in TM and `H_z=0` in TE. Mechanical fixed/free map to prescribed-primary/natural conditions, including the supported tensor interpretation.
- Thin gaps retain a physical trace-jump memory `z_gap`: with trace-difference operator T and positive surface stiffness Kg, `ż_gap=T u`, `Q̇_gap=-TᵀKg z_gap`, and `E_gap=½z_gapᵀKg z_gap`. This reproduces the compatible linear gap spring without a bulk potential. Include gap stiffness in CFL bounds, diagnostics and AMR residuals; port initialization and history transfer explicitly.

**Adopted outgoing law.** Use a power-conjugate boundary interface `uᵀQ̇Γ=-wᵀj`. For a homogeneous isotropic linear lossless exterior with primary/complementary coefficients α,β, `c=1/√(αβ)` and `Y₀=√(α/β)`. First order is the local impedance response. Second order uses the normalized rational response

```text
d = √(7/8)c|kτ|
Y(s)/Y₀ = 1 + (6d/7)/s - (8d/7)/(s+d) + (2d/7)/(s+2d).
```

For each normalized boundary mode:

```text
ẋ = -d diag(0,1,2)x + √d [1,1,1]ᵀ w
j = w + √d [6/7,-8/7,2/7]x.
```

Use the positive storage matrix H and energy-normalized `z=H½x` from the [auxiliary derivation, section 3](funfern-boundary-auxiliary-spike.md); auxiliary energy is `½zᵀz`. For FEM trace damping D and tangential stiffness A, prepare

```text
D⁻½ A D⁻½ = V diag(λ)Vᵀ
d_j = √(7λ_j/4),  TΓ = VᵀD½Rtrace
w = TΓu,  Q̇Γ = -TΓᵀj.
```

Treat disconnected trace components separately. At d=0 the law is exactly first order; omit inactive auxiliaries or preserve their harmless zero coupling, rather than inventing a small pole. Fixed passive interior maps have a semidiscrete bulk-plus-boundary energy balance; sources and temporal material work are separate. The [scattering report](funfern-boundary-scattering-spike.md) supplies the required force-coupled kick, which supersedes the auxiliary prototype's separate boundary split.

The initial exterior contract is linear, homogeneous, isotropic, lossless and initially unexcited. Interior loss/nonlinearity is not an exterior matching claim. Do not prohibit all nonlinear boundary-touching domains: enable each supported interior/boundary composition after its own tests. In particular, the corrected full nonlinear primary-trace step is still to be derived/verified. Any temporary fast-path limitation must be stated by actual trace/map capability, not by a blanket domain rule.

For curved/corner traces the assembled graph construction is passive but not exact exterior DtN. Annular energy tests and planar harmonic reflection are evidence, not curved radiation-accuracy acceptance. Retain the existing curved/topology reproducer as a production cutover gate; no silent downgrade to first order if it fails. More accurate rational/CRBC or shape-specific nonlocal DtN variants remain future work behind the same physical interface.

**Gate B.** Before each boundary capability is enabled, specify stage equations, stiffness/rate bounds, initialization, histories/transfer and prescribed intersections, then pass passivity, reflection, transient and fixed-CFL refinement tests. Before linear production cutover retain both outgoing orders and supported existing curved scenes. Nonlinear full-system composition closes before enabling nonlinear traces. Frozen reference impedance under modulation is allowed only as a documented, tested approximation. Measure nonlocal cost on the actual core.

### 4.4 Initial conditions and skin changes

New zero scenes start at `Q=0,b=0` and zero unexcited physical histories. Scalar-only initial data with no velocity use `Q=P(u₀,t₀),b=0` in the lossless homogeneous-boundary case. Nonzero gap or outgoing history, if requested, needs an explicit physical initializer; zero bulk fields alone do not specify it.

For a legacy fixed-linear API accepting arbitrary `u₀,u̇₀`, remove the unsupported free-component mean velocity, solve `Kψ₀=-M u̇₀` on the supported subspace, set `b₀=ηCψ₀,Q₀=Mu₀`, and discard ψ. This optional one-time compatibility solve is not part of live skin switching or remeshing.

All running skins carry the same `Q,b`. A skin/material toggle preserves those values and maps physical histories through the normal admission/event path, including prescribed-primary exchange. Mechanical↔TE preserves supported compiled maps under the named-field adapter; TM↔TE changes physical roles and need not preserve the same trajectories or separately named E/H values. Document conversion must preserve formulas/domains where supported, without claiming that reciprocal coefficient swapping alone proves nonlinear equivalence.

## 5. Material laws and constitutive evaluation

### 5.1 Authoring model

Laws belong to materials. Each applicable constitutive coefficient has at most:

- One field law.
- One time drive.
- One alternate factor.

Keep structural composition rather than an arbitrary list of additive law blocks. The coefficient composition is

\[
c(x,t,\text{field})=c_0(x)\,
g_{\rm field}(\text{field},x)\,
h_{\rm drive}(x,t)\,
h_{\rm switch}(t).
\]

The constitutive map uses this coefficient; recover the observable by the required constitutive inverse. An explicitly reciprocal coefficient applies its reciprocal semantics to the authored expression, not by substituting the same multiplier into an inverse response.

Authoring views expose different representations of this same material. Linear mechanical editing exposes stiffness `k₀(x)`; advanced nonlinear editing exposes the direct constitutive coefficient `s₀(x)=1/k₀(x)`, labeled **Reciprocal stiffness**. The existing base/field-law/drive/alternate slots remain structural composition, with explicit direct/reciprocal semantics. Section 11 specifies the view conversion and parameter conventions; changing views never changes the constitutive map.

Store the direct constitutive coefficient as authoritative material data: mechanical `s₀` corresponds to ε, while ρ corresponds to μ. Linear `k₀` is a derived editing view, not a second independent stored truth. Preserve the underlying spatial expression and exact reciprocal representation through edits/serialization. Drives and alternate factors act on the explicitly named authoritative coefficient: multiplying `s₀` by two halves its reciprocal stiffness. The advanced constitutive view also applies to time-driven materials whose field response remains linear. This decision supersedes any assumption that the old `stiffness_law` slot's name determines the stored or compiled physical role.

Spatial expressions remain `ScalarField` expressions evaluated in the region's material frame, including field-law, loss-law, and restoring coefficients. Time drives are a separate type; do not add an unrestricted time variable to the spatial expression grammar. Material-drive types are siblings of the source `TimeSignal` type.

Retain:

| Slot | Intended forms |
| --- | --- |
| Field response | Linear; Polynomial; Saturable; Kerr specialization |
| Time drive | None; harmonic “Parametric pump”; smoothed square “Time crystal”; “Travelling modulation” |
| Alternate | Positive factor per coefficient |
| Loss rate | Constant; passive saturable absorption; polynomial rate; later active laws |
| Oscillator/restoring extension | Klein–Gordon; sine–Gordon; φ⁴, behind section 10's derivation gate |

Concrete parameter/schema requirements:

| Form | Parameters and response |
| --- | --- |
| Linear field law | `g=1` |
| Polynomial | `χ₁, χ₂, amplitude_bound`; signed scalar `g(s)=1+χ₁s+χ₂s²`, with Gate C governing any vector/portable extension |
| Kerr | Polynomial with `χ₁=0`; portable isotropic `g(r)=1+χ₂r²` |
| Saturable | `χ,u_s` with `u_s>0`; `g(r)=1+χr²/(1+r²/u_s²)` |
| Harmonic drive | `depth,f,phase` |
| Square drive | `depth,f,phase,sharpness` |
| Travelling drive | `depth,f,phase,q_magnitude,q_angle` |
| Alternate | Positive factor per constitutive coefficient |
| Material Switch | Shared `switch_ramp` duration; independent coefficient targets |
| Loss-rate laws | Independent electric and magnetic channels: Constant; saturable absorption `u_s`; polynomial `β₁,β₂,bound`; later active laws behind their derivation gate. Each channel has an optional time drive; van der Pol `threshold,bound` additionally requires its actual oscillator equation. |

Here `r` is the magnitude of the physical field entering an isotropic constitutive law; for a scalar component it is the absolute value. It must not silently replace the signed argument `s` of the signed χ₁ polynomial.

A concrete default drive convention is `h=1+d cos θ`, where `θ=2π f t+φ` for a pump and `θ=2π f t+φ-q·x_material` for travelling modulation. Use `0≤d<1` so the multiplier stays positive. For the smooth square, use `h=1+d tanh(s cos θ)/tanh(s)` with `s>0`, using its cosine limit for small `s`. Its temporal resolution must be checked at the selected sharpness. Any hard-edge variant follows section 3.2.

Time and switch multipliers act on the authored constitutive coefficient before applying its inverse. The field law uses the actual current physical-field argument. Do not multiply an inverse response by the same factor without deriving the reciprocal scaling.

A material has one Switch gesture and ramp, with independent coefficient alternate factors. Both ε-only switching and equal ε/μ scaling remain possible.

Drive parameters, alternate factors, and declared validity bounds are spatially constant expressions over material parameters unless a specific spatial drive form supplies the dependence. Travelling modulation stores the wavevector/phase rule explicitly and samples position in a documented frame.

### 5.2 Primary versus complementary evaluation

For primary laws, assemble and invert the complete nodal map `P_i`, including all contributions.

For complementary laws, read independent vector flux `b` at quadrature, apply the constitutive inverse there, and gather the resulting nodal force. Do not infer its amplitude from the primary scalar field or a pair of neighboring scalar values.

For isotropic electric Kerr,

\[
D=\epsilon_{\rm lin}(x,t)(1+\chi |E|^2)E.
\]

In TM this is a scalar nodal constitutive law. In TE, obtain `E` from independent transverse `D=b`. With `r=|E|`,

\[
r+\chi r^3=|D|/\epsilon_{\rm lin},\qquad
E=r\,D/|D|,
\]

with the zero-vector case handled explicitly. The magnetic counterpart is analogous.

Precompute static linear complementary tensors and gather data. Keep CSR stiffness for comparison, bounds/filtering, and justified cache optimizations, not as an assumed replacement for independent `b`. Spatially/time-varying complementary laws use element/quadrature data prepared once per generation; a travelling drive is sampled at quadrature, not approximated by one material-wide number.

Several modulated materials may share a node or matrix entry. Their contributions remain distinct until gathered. No one-law-per-CSR-entry restriction.

### 5.3 Field arguments, conversion, and anisotropy

**Design gate C — portable nonlinear catalogue.**

The signed polynomial `1+χ₁u+χ₂u²` does not by itself define an isotropic vector constitutive law. Replacing `u` by `|E|` changes the χ₁ term's meaning.

Start the common portable nonlinear implementation with the unambiguous Kerr/intensity-dependent and saturable forms. Before exposing nonzero χ₁ across skins, choose and document either an appropriate vector/tensor law or an explicitly different toy amplitude law. Do not silently change its argument, disable a law during a toggle, or enable the shock preset without its stated field-argument contract.

Likewise, nonlinear anisotropy requires a specified vector constitutive map and positive differential response. Preserve existing linear anisotropy; do not silently approximate an unsupported nonlinear combination. The initial reciprocal complementary laws must derive from a convex constitutive energy so that the stability and energy contracts below apply. More general nonsymmetric response needs its own contract. Spatiotemporal modulation of these reciprocal instantaneous laws can still produce nonreciprocal propagation.

Mechanical `k↔ε` conversion may require a reciprocal coefficient expression. A reciprocal nonlinear coefficient is not automatically an inverse nonlinear constitutive map. Keep document round-trip tests, and derive each supported conversion's physical-field argument and monotonicity separately. The coefficient schema must preserve explicit reciprocal semantics (for example an `inverted` representation or equivalent expression), including serialization and effective-law display. Under `k↔ε` conversion, reciprocal factors and parameter expressions must transform with the coefficient. TM↔TE does not rewrite electric/magnetic material data. Skin changes are transactional across all materials: if any conversion or live-state admission fails, retain the accepted scene/state and report the material.

### 5.4 Inversion and admissibility

A valid law must provide:

- Its constitutive map and derivative/Jacobian.
- A declared admissible domain where the required inverse exists uniquely.
- Positive lower/upper tangent bounds adequate for the timestep estimate.
- Finite coefficient evaluation at all actual nodal/quadrature sample sites.
- Contextual errors naming material, coefficient, region/frame, and location.

For a scalar map `P(u)=m g(u)u`, the relevant derivative is `m[g(u)+u g′(u)]`. Positive secant coefficient alone is insufficient.

For an isotropic vector law `D=f(r)E`, check both tangential and radial differential responses:

\[
f(r)>0,\qquad f(r)+r f'(r)>0.
\]

Use these analytic scalar cases as validation and timestep regression fixtures, restricted to the explicitly supported scalar maps. The quoted global thresholds apply to unrestricted amplitudes; a law restricted to a declared bounded interval is validated over that interval instead:

| Map | Required check |
| --- | --- |
| `P=m(1+χ₁u+χ₂u²)u` | `P′/m=1+2χ₁u+3χ₂u²`. For `χ₂>0`, a global strictly positive tangent bound requires `1-χ₁²/(3χ₂)>0`. Treat the linear case separately; otherwise check the declared interval minimum and positive coefficient. |
| Direct Saturable, `a=χu_s²<0` | Require `a>-8/9` for strictly positive radial tangent; positive coefficient alone only requires `a>-1`. |
| Explicit reciprocal scalar coefficient, `P=mu/g(u)` | `P′/m=(g-u g′)/g²`, with `g>0`. For the polynomial, the numerator is `1-χ₂u²`. |
| Reciprocal Saturable | For `a>0` require `a<8`; for `a<0` require `a>-1`. |

Test values on both sides of each admissibility threshold and near the weakest accepted tangent. These scalar reciprocal fixtures do not establish a vector constitutive inverse or a physical skin conversion.

Validate constant expressions at edit time and spatial expressions at every actual nodal/quadrature evaluation site. An origin-only editor preview is not the assembly validity check.

Use safeguarded Newton/bisection or an equivalent bracketed method. A finite iteration cap means detectable failure, not permission to accept an unconverged result. Bounded laws must reject an out-of-range `Q`; bounding the root bracket must not become clipping the physical field to the authored amplitude limit.

Use a sensible cached/predicted initial guess and a residual/bracket criterion tied to f32 representability and the required temporal accuracy. Verify CPU and GPU behavior near zero, at interval endpoints, and near the weakest allowed tangent.

Always retain canonical `Q`. Do not replace it with `P` evaluated at an approximate root and inject that residual into future history.

A failed nonlinear stage must not become the committed state. Keep the last complete finite step, set a GPU error/status flag, and make subsequent already-encoded steps honor it. Report through the existing readback/status path. A candidate law outside the active timestep envelope is prepared with a new timestep before activation.

Candidate step state must remain separate from accepted state until globally validated through ordered GPU dispatches. A workgroup-local barrier or per-node error check cannot make an in-place multi-workgroup update atomic. Commit state, clock, auxiliary memory, force-cache ownership, event serials, and accounting coherently. Apply the same acceptance discipline to pulses, law patches, filters, and maintenance corrections.

Prevent failures with conservative tangent/time-resolution bounds, safeguarded inverse solves, dissipative passive-loss updates, and admission before committing material/pulse edits. Invalid candidate edits retain the accepted running scene/state. A runtime failure pauses at the last accepted complete state, keeps editing available, and permits resume after a valid corrective edit; Reset is not the default recovery requirement. A bounded smaller-step retry for numerical overshoot is optional future implementation work, not a spike-validated capability. Do not use retries to disguise a nonexistent inverse, invalid material domain, or exhausted validity bound; document any retry limit and never loop indefinitely. Error status distinguishes those cases and preserves accepted accounting and clock state.

## 6. Stability and diagnostics

### 6.1 Tangent-based timestep

For a lossless stage, define

\[
A=\partial U/\partial Q,\qquad J_b=\partial\mathcal C^{-1}/\partial b,
\qquad B=C^\mathsf T WJ_b C.
\]

For the admitted scalar primary laws and reciprocal complementary laws derived from a convex constitutive energy, `A` is positive diagonal and `B` is symmetric positive-semidefinite. A more general nonsymmetric law requires a separate stability and energy contract before it is enabled. Bound

\[
\Lambda\ge
\sup_{\text{admissible states and drive phases}}
\lambda_{\max}(A^{1/2}BA^{1/2}).
\]

The static kick–drift–kick limit is `Δt<2/√Λ`, with a chosen safety margin. Use an efficiently assembled conservative bound; do not add a runtime eigensolver.

A practical sufficient comparison is:

\[
P'_i\ge \alpha (M_0)_{ii},\quad B\preceq\beta K_0
\quad\Longrightarrow\quad
\Lambda\le(\beta/\alpha)\lambda_{\max}(M_0^{-1/2}K_0M_0^{-1/2}).
\]

Bounds must cover both constitutive sides, all nodal contributions, quadrature tensors, switch states, drive extrema, and admitted field amplitudes.

Add separate restrictions for temporal resolution, local source/loss integration, active gain, and auxiliary boundary states. A static CFL bound does not suppress genuine parametric growth or establish stability for arbitrary time variation.

Perform geometry/positive-area/positive-mass checks independently of constitutive timestep scaling. A strong valid law that requires a smaller timestep must not be reported as a near-degenerate mesh element.

The UI may show a conservative wave-speed/timestep range, but it must not report a secant coefficient range as though it were the full differential stability bound.

### 6.2 Runtime checks

Check actual evaluated amplitudes/tangents against the compiled envelope at the solver stages that use them. A delayed display readback alone is insufficient protection against an invalid inverse or timestep.

Use failure/status behavior consistent with section 5.4. A runtime violation sets the diagnostic warning badge and enters the Log with material/law context; a transient status notification alone is insufficient. Do not globally rescale or clip the field to conceal a violation.

### 6.3 Energy and observables

For constitutive maps satisfying that energy contract, use the canonical energy with nodal part

\[
T(Q,t)=\sum_i\int_0^{Q_i}U_i(q,t)\,dq
\]

and complementary energy `Σ_q W_q ∫₀^{b_q} 𝒞⁻¹(z,t)·dz`. Its gradient gives the complementary observable and the paired operators cancel bulk power. For linear media with inverse tensor J this is

\[
\mathcal H=\tfrac12 Q^\mathsf T M^{-1}Q
+\tfrac12 b^\mathsf T WJb.
\]

Temporal modulation can do work; sources inject energy; losses remove it; a geometry edit may change it. Separate these contributions where measured. Compute energy from the active constitutive maps and state. A “gain” label is not a substitute for that calculation.

Add thin-gap and outgoing storage energies explicitly. Synchronize scalar and complementary fields for displays/probes. The shared-core Poynting flow is `η u Rv`: TM gives `u RH`, TE/Mechanical gives `-u RE`. On compatible isotropic potential-reference data this reduces to `-k u ∇ψ`. Mechanical presentation uses the shared core's energy/flow meaning; the classical mechanical diagnostic `-k u̇∇u` belongs to a different energy functional.

## 7. Physical DC, invariant maintenance, and filtering

### 7.1 Physical field display and stationary state

Recover complementary observables directly from `b`. There is no filtered inverse-derivative reconstruction, bulk potential, potential rebasing, or per-component scalar display subtraction. Optional arrow smoothing is presentation-only.

A bounded uniform primary field remains visible. Do not impose zero component totals to hide it. Direct complementary state can contain both legitimate loss-generated stationary flux and numerical stationary artifacts. The [core spike, sections 1 and 3](funfern-material-laws-spike-report.md) measures these separately; a blanket projection onto potential-derived flux would erase valid state.

### 7.2 Correct only numerical invariant drift

For a closed unforced lossless component, `I_c=ΣᵢQᵢ` is constant. With sources, loss, boundary exchange, or topology events, maintain the intended total including accepted increments. Identify invariant components from the actual physical coupling/constraint graph, not simply geometric node identity or the old scalar `ConstantModes::free` flag. Thin-gap paired forces exchange flux between traces while preserving their combined total.

For static linear mass, a small measured drift `δ=I_c-ΣQᵢ` can be corrected with `Qᵢ←Qᵢ+mᵢδ/Σmⱼ`. For nonlinear maps, positive local tangent weights give an analogous inexpensive correction followed by reinversion. Validate domain/timestep admission; no clipping or global nonlinear projection. Set f32-scaled tolerances and cadence from real-core tests, and report corrections separately from physical forcing.

Avoid global reductions every step solely for bookkeeping. Accumulate applied increments where needed and reduce on the maintenance cadence. Closed components with no exchanges need no per-step source accounting. Accept accounting, corrections, state, and clock together; rejected candidates leave all unchanged. Deliberate source totals and material-induced changes in mean `u` are not numerical drift.

### 7.3 Selected linear grid filter; nonlinear gate F

For fixed linear maps `Q=Mu`, `v=Jb`, let `K=CᵀWJC` and let `Λ` bound the largest eigenvalue of `M⁻¹/²KM⁻¹/²`. Use the paired polynomial reference

```text
Q ← Q - α K M⁻¹ K M⁻¹ Q / Λ²
b ← b - α C M⁻¹ K M⁻¹ CᵀWJ b / Λ².
```

Both right-hand sides use the pre-filter state; `0≤α≤1`. Skip zero-operator components rather than divide by zero. Compatible `b=ηCψ` remains compatible; constant primary fields and free component totals are preserved to arithmetic accuracy. With the appropriate linear energy norms, a mode of eigenvalue λ is attenuated by `1-α(λ/Λ)²`. The [core spike filter fixture](funfern-material-laws-spike-report.md) tests the unit-complementary-map case: α=0.8 attenuates the top mode by 80%, and a mode at 8.52% of maximum frequency by 0.00422%.

This filter leaves `ker(CᵀWJ)` unchanged: it is not a cure for stationary remap artifacts. Validate tensor weighting, boundaries/constraints, nonuniform meshes, cadence, and f32 in the real core. Constrain the filter at prescribed nodes and account exchange explicitly. Do not silently use this fixed-linear energy argument for nonlinear/time-driven maps; derive and test the chosen frozen/reference-operator extension, its admissibility and its energy effect before enabling those media. Filtering invalidates force/observable caches and has staged acceptance.

## 8. GPU handoff and topology changes

### 8.1 Preserve the transaction architecture

The CPU prepares geometry, law metadata, local transfer stencils, trace correspondence, and factorizations while the old accepted state evolves. At commit, GPU field-dependent calculations use the latest accepted state. Keep generation validation/rollback and coherent display switching. Additional ordered GPU dispatches are allowed; an additional live CPU field-readback/calculation/upload cycle is not.

Transfer `Q,b`, physical thin-gap/outgoing/oscillator history, solver clock, phase anchors, material ramp/serial records through stable IDs, and invariant accounting. Rebuild observables/caches under the new maps. A same-mesh material-only event copies free `Q,b` exactly; prescribed nodes have the explicit boundary exchange of section 3.2. Unchanged physical histories copy exactly when their operator/coordinate contract is unchanged.

### 8.2 Complementary flux and new cells

Use local vector transfer in a common physical coordinate frame, with material/baffle side restrictions. The selected reference reconstructs a quadratic polynomial from each old element's six quadrature values, then samples it at target quadrature points. Geometry preparation supplies source elements, sample weights and incidence; the GPU supplies live values. Copy unchanged element/sample data exactly when geometry and coordinate contracts match. Rotation of an authored material frame is not permission to rotate the physical flux arbitrarily.

Preserve constants and surviving fields outside the affected region. For newly exposed connected cells use bounded local extension/blending from the correct side; a new disconnected island may initialize to zero. Specify and test fallback behavior when no trustworthy donor exists. No global harmonic relaxation is required. Genuine field mismatches across a new join may generate transients; there is no bulk gauge mismatch or seam-offset system to solve.

The [core spike remap results, section 3](funfern-material-laws-spike-report.md) support this inexpensive approach on resolved structured fixtures, not arbitrary production meshes. The coarse n=4 fixture already has 5.817% initial reconstruction error and fails the 5% accuracy criterion even on identity; do not reclassify it as a passing transfer or require a transfer to repair unresolved input. Resolved n=8→16 error is 1.485%, n=16→8 is 0.383%, and 12 cycles change the vector by 2.19%. Track stationary-energy injection separately from legitimate initial nonpotential state.

Gate T includes irregular and one-sided remaps, anisotropic frames, new/deleted cells, split/merge order independence, repeated cycles, and f32. Set fixture-specific thresholds before judging each new case. The recorded structured thresholds are evidence, not a universal 5% relative-error promise near zero fields.

### 8.3 Conservative integrated Q transfer

Transfer geometric flux density, not raw integrated nodal `Q` as though it were pointwise `u`. Prepare positive geometric support weights `vᵢ`, retained/deleted support shares, component correspondence, and affected correction support. Matching node coordinates alone does not establish identical support.

Interpolate `Qᵢ/vᵢ`, multiply by new support, and apply a bounded positive-weight component-total correction only on affected support. Preserve exact copies outside it. Normalize correction size against an absolute/L1-like flux scale, not only a signed component total which can cancel. Set maximum correction and support limits in CPU fixtures before the GPU port. Validate corrected values against the new inverse domains; reject or revise an infeasible candidate rather than clipping or spreading corrections globally.

Required contracts: exact identity on unchanged geometry; conserved accounted totals on unchanged domains; split-child totals equal prepared retained shares; merged totals add; added/deleted support has explicit initialization/removal accounting. Apply prescribed-primary exchange after conservative transfer. Exact polygon-overlap integration is not required if the cheaper bounded policy passes conservation and visible-artifact tests. This event correction is distinct from periodic roundoff maintenance.

### 8.4 Physical history is not bulk gauge

Thin-gap jump history and outgoing auxiliaries require their own transfer, not reconstruction from new `Q,b`. Prepare stable trace/side correspondence. Preserve history under unchanged operators, and test reversed trace orientation, refinement/coarsening, split/merge, removed edges, and newly created boundaries. New genuinely unexcited outgoing boundaries start at zero history; changing an existing excited boundary does not authorize an unexplained reset.

Outgoing auxiliaries are stored in energy-normalized modal coordinates (section 4.3). Eigenvector sign changes, reordering and degenerate eigenspace rotations must not change the represented physical history. Define transfer through an invariant trace-space representation or an equivalent basis-aware map, then validate energy/exchange and acceptance. Changed exterior/operator parameters require a documented history map and event-energy accounting. This is an implementation gate before boundary handoff, not a settled interpolation formula.

Thin-gap state follows its physical trace orientation and surface energy. New/deleted gap coupling needs an explicit initialization/removal rule and edit-energy accounting. Future oscillator memory is gated independently; it must not reintroduce generic bulk potential gauge work.

### 8.5 GPU execution and acceptance

Prepared GPU work order:

1. Read latest accepted `Q,b` and physical histories.
2. Apply local support-aware scalar/vector transfer and extension.
3. Reduce retained component totals and apply bounded affected-support corrections.
4. Enforce prescribed-primary fields and account boundary exchange.
5. Map boundary/gap history and runtime/clock/phase records; derive observables/caches.
6. Validate finite values, domains, correction budgets, layouts and event serials.
7. Atomically accept the generation, state, histories, clock and accounting, or retain the old generation intact.

There is no seam-potential sampling, gauge reduction, or component-offset solve. Boundary modal transforms/solves remain nonlocal physical work and must be budgeted separately. Profile ordinary AMR, split/merge and large boundary components, including peak old/new generation memory.

## 9. GPU data, performance, and integration constraints

Performance acceptance is measured on the production-intended solver core as it is built: first the complete linear CPU/GPU path with real boundary state, acceptance/failure handling and transfer, then nonlinear/time-driven increments. Measure wall time per simulated second at matched accuracy, memory, preparation/handoff latency and responsiveness. Do not defer all measurement until the catalogue/editor is finished, and do not use a disposable isolated kernel benchmark as architecture acceptance. The boundary candidate's nonlocal transforms and trace solves must be included; the old local-auxiliary cost is not a valid estimate for it.

Retain the current maximum of 32 authored materials unless a separate capacity change is budgeted and verified. More than 32 nodal material-instance contributions across region frames are not implicitly forbidden by that authored-material limit.

The GPU implementation is limited to eight storage bindings per stage. Law, contribution, quadrature, boundary-history, transfer and runtime tables need explicit packing/offset layouts within the portable limit, or separate kernels with their own compliant layouts.

Before implementation, record:

- Authoritative and scratch state lanes, including failure-safe staging.
- Per-node contribution ranges and per-contribution data.
- Per-element/quadrature geometry and constitutive data.
- Material law table and stable-ID runtime mapping.
- Node/element gather structure; do not assume portable floating-point atomics.
- Local scalar/vector transfer stencils, invariant reductions, trace-history maps, boundary factors/transforms, and scratch space.
- Every Rust/WGSL shared structure, offset, stride, and access mode.
- Bytes per node/element/material, transfer bytes, and dispatch count for each path.

Use the state budget freed by unnecessary reconstruction/filter history where possible. Keep memory cost close to the measured baseline as a target, not an established result; local state savings do not automatically cover nonlinear quadrature/gather work.

Keep three measured performance cases separate:

| Case | Intended spatial work |
| --- | --- |
| Static linear medium | Precomputed complementary tensors, quadrature drift and deterministic force gather; endpoint cache where valid |
| Nonlinear/time-driven primary law only | Same bulk spatial path plus local nodal inversion/evaluation; nonlinear primary boundary solves where applicable |
| Dynamic or nonlinear complementary law | Quadrature constitutive evaluation and deterministic nodal gather |

Precompute basis gradients, quadrature coordinates, material-frame data, and adjacency during resumable preparation. Do not reassemble the full operator on the CPU every timestep.

Keep force units explicit: a per-mass acceleration matrix is not integrated force on `Q`. A potential-derived CSR force alone cannot represent arbitrary independent `b`; any optimized cache update must recover the same force after every loss, filter and event. Reference lumped masses and normalized source arrays remain immutable within a generation. Changing a constitutive law must not stale source normalization or reuse a cache with the wrong scaling.

The [core resource screen](funfern-material-laws-spike-report.md#5-provisional-gpu-resource-design) is provisional, not a GPU measurement: its roughly 1.03× main-state estimate excludes full law/runtime tables and the subsequently selected nonlocal boundary realization. The [auxiliary cost analysis](funfern-boundary-auxiliary-spike.md) requires about 24Nb bytes for accepted/candidate three-state f32 histories, plus dense transforms/factors of order Nb² and factor preparation of order Nb³ in the reference implementation. Auxiliary elimination reduces each solve to the primary trace; do not port the full dense four-state update matrix. Budget resumable preparation, peak old/new factors, solve accuracy and the largest connected boundary. A cheaper realization requires parity/passivity evidence, not an assumed local cost.

Store integer serials, indices, and event counters in integer lanes. Bitcasting small integers into f32 can produce subnormals that some backends flush. Check every Rust/WGSL array stride and every intentional prefix-only buffer declaration, as well as complete shared structures.

Classify law edits by their actual preparation needs: table-only updates, refreshed node/quadrature coefficient metadata, or a new layout/timestep. Re-evaluate spatial formulas at the compiled sample sites before upload. Same-layout, timestep-safe candidates may use a coherent in-place buffer swap after GPU admission; all others use the generation handoff. No source or law buffer update may land between the stages of a complete solver step.

Source/law buffer replacement must carry all unchanged tables and runtime mappings. Compile against the running generation's material indexing. A newer editor document must not corrupt an in-flight candidate or live buffer update.

Update every consumer: solver, transfer, grid filter, probes, far-field/energy diagnostics, CPU readback adapters, buffer-size estimates, and shader-layout tests.

AMR is an explicit physics port. The current scalar residual and its displacement/velocity/acceleration snapshot can be retained for the static linear core only with a justified synchronized adapter. Before enabling time-driven or nonlinear media, derive residuals and normalization for their equations, including material interfaces, thin gaps, driven boundaries, and outgoing auxiliary state. Revalidate the reported relative-error measure and wavelength guidance for modulation/generated harmonics. Reuse the resumable estimator/controller architecture, not an unchanged scalar residual with renamed state fields.

Port point/curve/area probes, canonical energy/flow, vector overlays, and far-field sampling at synchronized stages. Preserve far-field analysis's supported exterior-medium assumptions and reject unsupported nonlinear/driven exteriors explicitly. Treat probe history whose physical meaning changes at a skin transition with an explicit segmentation/conversion policy.

## 10. Oscillator media and design gates

The implementation programme includes Klein–Gordon, sine–Gordon, φ⁴, and van der Pol media. Their parameter slots are:

| Medium | Authored parameters | Required model contract |
| --- | --- | --- |
| Klein–Gordon | `ω₀` | Linear restoring acceleration with frequency-squared coefficient |
| sine–Gordon | `ω₀` | Sine restoring law, with the scalar phase/amplitude units and kink scale defined |
| φ⁴ | `λ, amplitude_bound` | Quartic restoring family; define force normalization, equilibrium amplitudes, and coefficient units |
| van der Pol | `threshold, bound` with the loss-rate scale | Active/saturating oscillator law; define threshold units and the actual coupled equation |

Restoring is its own material row, default None. Its spatially varying coefficients use material-frame `ScalarField` evaluation. The common wave core can ship before these extensions, but the full catalogue is complete only after their dependent models and demonstrations are implemented.

For comparison only, an extended potential model with a fixed linear restoring term gives

\[
\dot Q=-(K+V)\psi,\qquad \dot\psi=M^{-1}Q
\]

and therefore `Mü+(K+V)u=0`. This is not the selected bulk implementation. An equivalent direct-core extension needs genuine additional physical state; its uniform component can become physical and must be initialized/transferred under that model's contract.

For a nonlinear force, adding `-V′(ψ)` to the first-order flux equation does not produce a scalar acceleration `-V′(u)` after differentiation. Specify the intended equation before selecting its state representation.

**Design gate O — oscillator extension.** Choose either:

- Explicit additional polarization/oscillator state with a specified coupling and transfer rule; or
- A deliberately reformulated toy medium with honest equations and preset names.

Derive energy/gain behavior, initialization, timestep limits, gauge/reference freedoms, skin mapping, and GPU handoff before enabling the corresponding controls.

Other gates and their completion point:

| Gate | Required decision/evidence | Must close before |
| --- | --- | --- |
| V: architecture evidence — closed | Direct Q,b and passive outgoing law with corrected kicks adopted; three reports linked in section 12 retain failures and limits | Completed before implementation; production-specific gates below remain |
| S: sources | Direct-source units, pulse/boundary normalization, legacy source mapping | Production common-core cutover |
| B: boundaries | Stage equations, auxiliary mapping, passivity/stability/reflection tests | Production common-core cutover |
| F: grid filter | Canonical action and spectral/invariant tests | Production common-core cutover |
| T: integrated transfer | Geometry-prepared conservative policy and bounded correction measurements | Production common-core cutover |
| C: nonlinear catalogue | Signed χ₁/vector meaning, reciprocal conversion, nonlinear anisotropy | Enabling those combinations/presets |
| O: oscillator media | Equations and extra-state/reference contract | Enabling KG/sine–Gordon/φ⁴/van der Pol as promised models |

These are engineering derivation/validation tasks. They do not require stopping ordinary implementation to ask permission for routine choices.

## 11. UI, persistence, presets, and compatibility

Provide two material-editor views:

- **Simplified:** base values, preset selector, named parameters, effective-law text, Switch, and stability readout.
- **Advanced:** all applicable law rows and numeric effective-law details.

Place the Advanced toggle in the material Library header beside formula help. It defaults off, persists as presentation state, and is not an undoable model edit. Use phenomenon names in controls, with the mathematical form in hover text. Provide the material `Switch ▸` button and its hotkey.

The coefficient-editing contract is:

| Editor context | Coefficients and controls shown |
| --- | --- |
| Linear mechanical material | Familiar density `ρ₀(x)` and stiffness `k₀(x)` spatial fields |
| Simplified nonlinear material | Named preset parameters, such as base stiffness, nonlinear strength, and saturation scale, with a readable effective-law summary |
| Advanced nonlinear or time-driven material | Direct constitutive coefficient fields; the mechanical complementary row exposes authoritative `s₀(x)=1/k₀(x)` as **Reciprocal stiffness**, alongside the applicable response/drive/alternate slots |

These are authoring presentations of one material model, not separate solver modes. The EM views expose their corresponding direct electric/magnetic constitutive coefficients. Advanced exposes both named loss channels and their physical-field arguments; Simplified presets may expose only the parameters relevant to the selected phenomenon while preserving all authored channels. The mechanical complementary observable `e` in the following example is the stress-like toy field of the shared-core adapter; its identity and normalized units must be stated in help, rather than substituted with the displayed primary scalar or the flux argument.

For a supported Kerr-type complementary response, the advanced mechanical view presents

\[
\xi=s_0(x)\bigl(1+\chi|e|^2\bigr)e,
\qquad s_0(x)=1/k_0(x).
\]

It does not require the user to manipulate the equivalent nested reciprocal expression for effective stiffness. Show that effective stiffness only as optional derived detail. Opening Advanced automatically presents the reciprocal of an existing `k₀(x)` expression; users do not have to calculate or re-enter it. Preserve arbitrary valid spatial formulas and parameter references symbolically, and refer to the named `s₀(x)` slot in the law summary instead of expanding its formula repeatedly. Use **Reciprocal stiffness** as the primary label; explanatory “compliance” terminology must follow the selected mechanical contract.

Switching Simplified/Advanced is presentation-only: preserve authored law meaning, live state, runtime ramps, document dirtiness, and undo history. Do not run a solver transaction or cumulatively rewrite formulas merely to change the view. Actual coefficient edits or applying a preset remain authored changes with the normal undo/validation/activation transaction. A Custom advanced law must remain intact when Simplified is opened; display its available named parameters and Custom status without fitting or replacing it with a preset.

Keep nonlinear coefficient conventions explicit and invariant between views. In the factored expression above, `χ` is the relative field-law coefficient. In the equivalent expanded map `ξ=s₀e+a₃|e|²e`, the absolute cubic coefficient is `a₃(x)=s₀(x)χ`. These are distinct quantities with distinct units/spatial dependence. The initial editor retains the factored convention specified in section 5.1. Any future editable absolute-coefficient form needs a separately named representation and lossless conversion; it must not silently reinterpret the same χ control or introduce an arbitrary additive-block model.

Tests must cover a nontrivial spatial `k₀(x)` opened in Advanced and returned to Simplified/linear presentation without expression growth or changed maps; formula references and persistence; direct editing of `s₀(x)`; correct effective-law arguments; unchanged state/ramp/history on view toggles; Custom-law preservation; and the distinction between relative χ and absolute `a₃`. Linear presentation applies to a linear material: selecting a linear law from a nonlinear material is a real authored edit, not a consequence of closing Advanced.

Presets are factories that write material data. They are snapshots, not hidden live bindings. Show a preset name only on structural match; otherwise show Custom. Applying a preset or changing a material is one undo step.

Each preset creates semantically named parameters and formulas referring to them. Check name collisions and remaining capacity while preserving user-authored parameters. Support both creating a material from a preset and applying one to the selected material. Filter the selector by skin and implemented law support; a preset with an open design gate is unavailable.

Provide an `= 2 × source` helper for pump frequency, with an explicit selected source when several are present. Effective-law text is implemented and tested in core: one line per modified coefficient, names in Simplified and numbers in Advanced. Display reciprocals as division and name the actual physical-field argument; TE electric response must not be labeled as a law of scalar `H_z`.

Parameter-name discovery, `uses_frame`, the UI's referenced-parameter checks, and volume-source parameter handling must traverse every applicable law field. Parameter rename is transactional: rewrite all affected formulas or commit none. Validate reserved identifiers and the eight-parameter/material limit. Errors identify the actual material, coefficient, and sample location.

Formula help includes a separate Laws section describing field arguments, coefficient composition, time drives, switch factors, and validity bounds. Keep the parser grammar reference and its tests limited to syntax actually accepted by the expression parser.

Retain planned demonstrations:

| Demonstration | Dependency |
| --- | --- |
| Kerr/self-focusing | Correct primary and complementary electric laws |
| Parametric pump | Stage-time drives and tangent bounds |
| Time crystal / temporal interfaces | Edge policy and coherent state continuity |
| Travelling modulation | Spatial phase at actual nodal/quadrature evaluation sites |
| Shock/steepening medium | Gate C's signed/nonlinear argument contract |
| Sine–Gordon soliton | Gate O's actual oscillator equation |

Add a gallery scene only after its physical/numerical claim is verified. An enabled example must compile and assemble in tests.

Existing scene files, links, autosaves, and examples must load with inert law defaults. The baseline is document version 22. Changing runtime state alone does not force a file version bump, but changing serialized source/law meaning needs explicit migration/version handling. Do not silently reinterpret a serialized field to preserve a version number.

Round-trip every supported law variant and structural preset. Test rejection of malformed variants, missing required fields, non-finite values, and parameters outside their admissible domains. Preserve supported inert defaults for version-22 documents that omit added fields. Runtime switch/ramp state remains outside the authored document unless a future explicit simulation-checkpoint feature is added.

## 12. Implementation sequence and handoff

### Completed validation and adopted decision

The bounded correctness work is complete; no production solver has been changed by it.

| Evidence | Outcome and use |
| --- | --- |
| [Core/material spike](funfern-material-laws-spike-report.md) | Select direct Q,b; keep filter/remap limits and resource qualifications. Original 113 passes/6 failures remain historical evidence, not a claimed all-pass result. |
| [Passive auxiliary spike](funfern-boundary-auxiliary-spike.md) | Adopt its three-state passive law; 64 passing auxiliary/stability checks. Its standalone boundary time split is superseded. |
| [Single-boundary scattering spike](funfern-boundary-scattering-spike.md) | Adopt force-coupled midpoint kicks; 248 passes. Retain the old split's 21 failures as regression evidence. |

The user accepted the candidate's reflection/nonlocal-cost tradeoff. Do not reopen the bulk representation or repeat broad spikes without new contrary evidence. This does not close f32, full nonlinear composition, irregular remap, curved radiation, history-transfer or actual-core performance gates.

### Reviewable implementation stages

The [detailed stage plan](funfern-material-laws-review.md#5-implementation-stages) is the task breakdown; this table fixes its order and release boundaries.

| Stage | Deliverable | Enablement boundary |
| --- | --- | --- |
| 0 (complete) | Source/history/fixture contracts, consumer inventory, baseline measurements and budget criteria | No production change; concrete thresholds precede new fixture judgments |
| 1 (complete) | Restore inert authoring/build baseline, correct validation/traversal, persistence and support checks | Unsupported physics stays unavailable |
| 2 (complete) | Linear f64 Q,b core, physical maps/tensors, KDK, energy and compatibility initializer | Old production solver remains available for comparison |
| 3 (complete) | CPU source/loss/boundary/filter composition and bounded scalar/vector/history remap | Linear S/B/F/T equations and reference fixtures closed before their GPU port; production f32/performance acceptance remains Stage 4 |
| 4 (complete) | Production-intended GPU evolution, real layouts, clock, all-or-none acceptance | CPU/f32 parity, failure injection and measured steady cost closed in the [Stage 4 report](funfern-material-laws-stage4-report.md) |
| 5 | Latest-state GPU transfer, physical histories, live edits/paused events | No live CPU field calculation; rollback and handoff performance |
| 6 | AMR, thin-gap diagnostics, probes/rendering/energy/flow/far-field; shared-skin cutover | First release: validated linear common core, both outgoing orders, supported existing scenes |
| 7 | Time drives, Switch, reciprocal ramp semantics, modulation-aware AMR/filter/boundaries | Enable tested time-driven combinations |
| 8 | CPU nonlinear maps/inverses, tangent bounds, nonlinear boundary/filter/AMR contracts | Kerr/saturable first; unsupported C combinations remain disabled |
| 9 | GPU nonlinear execution, admission/recovery and actual-core performance | Enable supported nonlinear combinations only after composition/parity tests |
| 10 | Complete Simplified/Advanced UI, presets, demonstrations and compatibility docs | Second release: supported instantaneous nonlinear/time-driven catalogue |
| 11 | Individually derived oscillator/active-media extensions | Enable each only after Gate O equations, transfer and demonstrations pass |

Performance is measured incrementally on the real solver path in Stages 2–6 and again for dynamic/nonlinear additions, not by a disposable isolated kernel benchmark and not only after the catalogue is finished. Include boundary preparation/solve, validation, peak memory and handoff; compare wall time per simulated second at matched accuracy.

### Current implementation handoff

Read this specification and the detailed stages; consult reports for derivations, not as competing live plans. Baseline `beca47e` remains the clean numerical reference. Stage 1 coalesced the useful unfinished authoring work, removed superseded semantics, restored all material literals/imports, and added explicit legacy-solver rejection for non-inert laws. Do not copy the spike Python into production, implement bulk gauge machinery, or interpret the green Stage 1 checks as validation of a canonical or nonlinear solver that does not exist yet.

Stage 0's concrete production source-edit anchors/normalization, legacy loss-channel migration, boundary/gap history mapping and initialization, irregular transfer thresholds, event ownership and target-device performance budgets are fixed in the [Stage 0 implementation contracts](funfern-material-laws-stage0-contracts.md). If implementation evidence requires a material scope/accuracy change, report and amend it explicitly rather than weakening a fixture in place.

Stage 3 added CPU source, boundary, fixed-linear loss, filter and transfer composition to the linear direct reference. It includes analytic legacy source/loss conversion oracles but deliberately does not rewrite persisted version-22 data while the old solver remains the production application path. The corrected scattering suite still passes all 248 checks, and CPU fixtures cover passivity, prescribed intersections, basis-invariant physical histories, changed/deleted/new traces, connected extension and irregular repeated remap. Dynamic/field-dependent loss execution remains at Stages 7–8.

The standard CPU reference measures about 5.2 simulated seconds per wall second without an outgoing trace and 1.9 with the dense 276-DOF second-order trace. The latter uses about 0.58 MiB of modal rows, 0.62 MiB of immutable factor data and roughly 0.15 ms per reduced trace solve. Typical second-order operator/factor preparation was about 227 ms, though slower eigensolve runs crossed it.

Stage 4 ports those equations to a dormant production-intended f32 WebGPU core with one eight-storage-binding manifest, explicit accepted/candidate lanes, a bounded epoch clock and one global acceptance boundary. On the Apple M1 Max, 1,000-step standard fixtures run at 18.46 simulated seconds/wall second reflecting, 18.50 first order, 16.51 with two-sided fixed loss and 10.18 with the 276-node passive second-order trace. All have `Q,b <= 1.88e-5`, auxiliary RMS `<= 6.14e-8`, and energy residual `<= 9.02e-6`; prescribed second-order and the eight-obstacle first-order cases also pass. The standard second-order end-to-end CPU preparation is about 302–311 ms, while a 128-step GPU request completes in about 50 ms; these are reported separately rather than hiding preparation in throughput. Exact failure rollback, resumption, staged linear events and clock rebase pass. See the [Stage 4 implementation and acceptance report](funfern-material-laws-stage4-report.md) for layouts, byte counts, commands and qualifications.

Stage 5 now owns latest-state GPU transfer, generation handoff, physical-history mapping and genuinely live/paused event service. The old production solver remains the comparison and application path until the later cutover stages. Each stage produces a reviewable change with tests and an engineering-log entry. Keep implementation status distinct from planned behavior, and keep unsupported features unavailable rather than silently ignored.

## 13. Verification and acceptance

### 13.1 Numerical tests

- Fixed lossless linear modes: frequency, phase, and second-order temporal convergence; measure the separate order/error of frozen-rate dissipative composition.
- Two-sided loss: electric/magnetic argument preservation across skins, independent and combined dissipation, and the spike-selected state's constraint/stationary-mode behavior.
- Migrated source offsets and zero/near-zero frequency; phase anchors through edits/handoff; ramp reversal continuity without an unsupported derivative-continuity claim.
- Canonical/scalar equivalence for compatible data; exactly the scoped free-mode difference.
- Several disconnected free components with independent constants and mean velocities.
- Constant primary field: bounded physical state without bulk potential/rebasing; stationary complementary state retained and measured.
- CPU/GPU agreement with declared tolerances and identical stage/source conventions.
- Constitutive inverse residuals and branch validity, including material junctions.
- Both temporal coefficients changing: continuity of `Q` and the discrete complementary flux. Use homogeneous-medium fixtures for analytical pointwise `D,B` tests; do not assert universal `Q̇` continuity.
- Equal ε/μ scaling: temporal reflection check with compatible travelling-wave data.
- Parametric growth against a linearized reference in a specified small-amplitude regime; frequency scans rather than a blanket “all detuned cases stable” claim.
- Polarization-correct nonlinear tests. Generic electric-Kerr TM and TE scenes need not match; duality tests must exchange electric/magnetic laws and map data.
- Boundary reflection/passivity and auxiliary stability.
- Grid-filter effect on unresolved versus resolved modes and component totals.
- GPU failure containment when an inverse or compiled amplitude envelope is violated.
- Self-focusing: compare beam width against a linear control in a declared valid regime; saturation remains bounded. Use an admitted negative-response law as a defocusing control where appropriate.
- Travelling modulation: compare pulses launched with and against the modulation, measure spectral asymmetry, and verify that it disappears at zero depth.
- Signed χ₁, after Gate C: measure harmonic generation/steepening with the specified field argument.
- Klein–Gordon, after Gate O: verify `ω²=c²k²+ω₀²` for the admitted linear medium.
- sine–Gordon, after Gate O: initialize the kink profile of the actual derived equation (for the conventional normalized model, `u=4 atan exp(x/ℓ)` with its derived `ℓ`) and bound shape error.
- Use radiating boundaries to measure outgoing decay, and closed cavities for trapped-mode/parametric tests; the boundary choice must match the measured claim.

### 13.2 Handoff tests

- Old evolution continues throughout resumable preparation.
- Transfer uses the latest GPU state, not a preparation-time field snapshot.
- Material-only edits preserve free `Q,b`; prescribed-primary changes have explicit boundary exchange and derived observables follow the new law.
- Outgoing-history mapping is invariant under modal signs, ordering and degenerate-basis rotations.
- Matching physical fields/traces remain matched; history mapping creates no unexplained pulse or reset.
- Multiway merges with cycles are independent of processing order.
- Splits preserve inherited fields and assign totals correctly.
- New-cell extension preserves constants; untouched-support Q and unchanged quadrature b copy exactly. New/deleted history has explicit initialization/removal accounting.
- Active ramp, source phase, and absolute solver time survive handoff and timestep changes.
- Repeated source/pulse buffer replacements do not reset law serials.
- In-flight editor changes cannot mix material indexing across generations.
- No additional CPU field-readback dependency appears in the handoff.
- Measure GPU commit duration separately from CPU preparation and the existing validation wait.

### 13.3 Repository checks

Run the established checks appropriate to each implementation phase:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo build --release -p funfern-app --locked
cargo check --target wasm32-unknown-unknown -p funfern-app --locked
```

Validate all WGSL with the existing naga test, and verify shared Rust/WGSL layouts across every binder. Shader parsing alone does not prove cross-file layout agreement.

Interactive checks must use isolated application data. On Linux the baseline supports `XDG_STATE_HOME`; do not repurpose `HOME`. On other platforms use an isolated test account/profile or add an explicit test-data path before running tests that write autosave. A browser check uses isolated site storage.

Compile/assemble every gallery example. Native and WASM builds must remain within the declared portable binding limits. Do not force unrelated rebuilds or repeat broad checks after no relevant changes.

The interactive acceptance sequence loads every enabled preset, inspects its effective-law text, uses Switch, sets pump depth to zero, toggles Simplified/Advanced, changes Mechanical/TM/TE skins while evolving, and undoes each authored edit in one step. Runtime Switch and presentation toggles retain their specified non-undoable behavior. Inspect phase continuity, bounded seam transients, warning/log output, and candidate preparation while editing.

## 14. Code entry points

These links identify the reviewed baseline; use symbols and current code when implementing rather than relying on stale line numbers.

| Area | Entry points |
| --- | --- |
| Core field/assembly/components | [wave_quadratic.rs](https://github.com/DmitriyNE/funfern/blob/beca47e0d05c9b24945ef632b00c5cc16f80c8a0/crates/funfern-core/src/wave_quadratic.rs): `QuadraticWaveState`, `QuadraticWaveOperator`, `QuadraticAssemblyWork`, `ConstantModes` |
| Physics/material conversion | [wave.rs](https://github.com/DmitriyNE/funfern/blob/beca47e0d05c9b24945ef632b00c5cc16f80c8a0/crates/funfern-core/src/wave.rs): `PhysicsModel`, `wave_coefficients`, `convert_material` |
| Material/formula model | [material.rs](https://github.com/DmitriyNE/funfern/blob/beca47e0d05c9b24945ef632b00c5cc16f80c8a0/crates/funfern-core/src/material.rs), [geometry.rs](https://github.com/DmitriyNE/funfern/blob/beca47e0d05c9b24945ef632b00c5cc16f80c8a0/crates/funfern-core/src/geometry.rs) |
| CPU transfer preparation | [transfer.rs](https://github.com/DmitriyNE/funfern/blob/beca47e0d05c9b24945ef632b00c5cc16f80c8a0/crates/funfern-core/src/transfer.rs): `QuadraticTransferJob`, `QuadraticTransferMap` |
| GPU state and handoff | [wave_gpu.rs](https://github.com/DmitriyNE/funfern/blob/beca47e0d05c9b24945ef632b00c5cc16f80c8a0/crates/funfern-app/src/wave_gpu.rs): buffer layouts, `replace_transferred_with_volume_sources`, ordered transfer dispatches |
| Evolution/transfer shaders | [wave.wgsl](https://github.com/DmitriyNE/funfern/blob/beca47e0d05c9b24945ef632b00c5cc16f80c8a0/crates/funfern-app/src/wave.wgsl), [wave_transfer_old.wgsl](https://github.com/DmitriyNE/funfern/blob/beca47e0d05c9b24945ef632b00c5cc16f80c8a0/crates/funfern-app/src/wave_transfer_old.wgsl), [wave_transfer_new.wgsl](https://github.com/DmitriyNE/funfern/blob/beca47e0d05c9b24945ef632b00c5cc16f80c8a0/crates/funfern-app/src/wave_transfer_new.wgsl) |
| Candidate lifecycle | [topology_runtime.rs](https://github.com/DmitriyNE/funfern/blob/beca47e0d05c9b24945ef632b00c5cc16f80c8a0/crates/funfern-app/src/topology_runtime.rs), [topology_editor.rs](https://github.com/DmitriyNE/funfern/blob/beca47e0d05c9b24945ef632b00c5cc16f80c8a0/crates/funfern-app/src/topology_editor.rs) |
| Persistence/editor/examples | `topology_persistence.rs`, `document.rs`, `ui.rs`, `topology_examples.rs` under `crates/funfern-app/src` |
| Existing architecture | [architecture.md](https://github.com/DmitriyNE/funfern/blob/beca47e0d05c9b24945ef632b00c5cc16f80c8a0/docs/architecture.md): reconstruction, DC handling, transaction lifecycle, transfer, binding budget |

Implementation ownership and integration notes:

- Core `Material` has no serde implementation; app `StoredMaterial` and its encode/decode paths own document persistence.
- Keep reciprocal/multiply/divide expression conversion in core, where those `ScalarField` utilities are available.
- Implement shared material parameter/reference traversal, adding a `parameter_names` API where useful, and extend `Material::rename_parameter`, `uses_frame`, and `VolumeSourceCompileJob` consistently.
- Update `gpu_forcing`, `gpu_nodes_with_damping`, `WaveBufferHandles`, and every full buffer replacement to carry current law tables and runtime mappings.
- Reuse `retime_for_speed` and `paced_time_step` integration points while preserving the canonical timestep/event contract.
- Use `material_scalar_editor` for spatial coefficients; add a tested core effective-law formatter and a material-from-preset editor operation.
- Shader tests must cover cross-file layouts and intentional prefixes, not only per-file naga parsing.
- Every completed implementation phase updates `docs/engineering-log.md` with dated findings and validation, plus README/architecture/plan sections relevant to the delivered behavior.

The specification describes required implementation and verification; it does not claim those code changes or performance measurements have already been completed.
