# Material-law overhaul: review and staged implementation

Review date: 19 September 2026. Baseline: `beca47e`, plus the current uncommitted material-law work. Specification reviewed: [funfern-material-laws-plan.md](funfern-material-laws-plan.md).

This is a planning deliverable, not an implementation. The initial review changed no source code or existing specification. Following user review, the accepted boundary-exchange, thin-gap/AMR/consumer, and global GPU acceptance corrections were incorporated into the specification. Source code remains unchanged by this planning work.

## 1. Recommendation

Proceed with staged implementation of direct `Q,b` and physical trace/oscillator memory. The shared constitutive-field approach fits funfern's enriched quadratic FEM, material frames, GPU evolution and resumable candidate/commit architecture. The completed [core spike](funfern-material-laws-spike-report.md) supports two-sided physical loss and the direct-state choice; the user has adopted the [passive outgoing law](funfern-boundary-auxiliary-spike.md) with the [corrected force-coupled kicks](funfern-boundary-scattering-spike.md).

Accepted product decisions include authoritative `s₀`, named electric/magnetic loss channels, analytic source migration, phase anchors, inexpensive frozen-rate loss and ramp reversal, and recoverable global acceptance. This review and the specification now describe the selected architecture. No bulk potential, gauge rebasing, seam-offset solve or assumed one-CSR stepping path is required. Potential notation below is confined to compatibility/analytical explanations.

The existing nonlinear work is mostly reusable **authoring infrastructure**, not an implemented nonlinear solver. Keep its structural composition and formula utilities; revise its numerical validation, field arguments, conversion claims, and oscillator semantics. No nonlinear stepping, constitutive inversion, GPU law execution, or canonical transfer has been implemented in the changes inspected.

The main engineering risk is the linear production migration: boundary auxiliaries and their physical histories, conservative scalar/vector remeshing, diagnostic consumers and GPU failure containment. Complete these before enabling nonlinear materials. The detailed stages below separate this work and its measured performance gates.

## 2. What checks out

### Interior equations and integration

With `C=RG`, `Q̇=-ηCᵀWJb` and `ḃ=ηCu`, the TM/TE signs give positive stiffness `K=CᵀWJC` and flow `ηuRv`. Differentiation with `Q=Mu` gives `Mü+Ku=0` for fixed lossless linear maps. Compatible `b=ηCψ` reproduces the potential reference; independent b also carries stationary state. The spike verifies the actual enriched-FEM operators, not merely the continuous algebra.

The restricted free-component initial-data compatibility is real and accurately described. This is not an arbitrary replacement of all second-order initial states. Default zero scenes and scalar pulses are compatible; arbitrary legacy initial velocities need the stated initialization projection/solve. The uniform-field and damping distinctions in the specification should remain explicit product behavior.

The endpoint/midpoint kick–drift–kick construction is second order for smooth time-dependent constitutive maps. Its static linear CFL statement is appropriate. The specification correctly distinguishes that local stability restriction from real parametric amplification and from temporal-resolution requirements.

Canonical energy and the differentiation argument use different meanings of energy than the existing mechanical displacement/velocity diagnostic. The specification acknowledges this correctly. Energy and flow consumers must change together.

### Match to the actual discretization

`QuadraticAssemblyWork::element` in `wave_quadratic.rs` uses exactly the seven nodes, positive area weights `1/20, 2/15, 9/20`, and six-point stiffness quadrature described in the plan. Preserve these. The nonlinear discretization is a deliberately lumped nodal primary map plus quadrature complementary response, not exact integration of arbitrary nonlinear formulas.

Existing anisotropy is a rotated symmetric stiffness tensor. If that tensor is `A`, the complementary inverse constitutive tensor must be `R A Rᵀ`, since `Cᵀ(R A Rᵀ)C = Gᵀ A G`. Copying `A` directly into the transverse constitutive inverse generally rotates the anisotropy incorrectly.

The dependency-free f64 core, compute passes, gather operations, stable material/region/trace IDs and resumable preparation support this architecture. The bulk stays explicit; the adopted boundary needs a nonlocal trace solve and prepared factors. No whole-domain nonlinear solve or live CPU field round trip is planned.

### Transactions and bounded scope

The existing `WaveGpuRequest` transfer path retains old buffer handles, prepares interpolation from geometry, transfers live state, and validates before acceptance. Reuse that lifecycle, not its historical-level reconstruction equations.

The distinction between authored material IDs and region/frame-specific contributions is necessary. One material used in two frames is not one evaluated constitutive contribution. Likewise, the eight-storage-binding limit is already an explicit repository contract; the current wave and transfer pipelines use all eight.

Direct state removes bulk gauge alignment and admits physical nonpotential flux, but does not promise general Maxwell background-field authoring or automatic removal of discretization artifacts. Separation of oscillators from instantaneous constitutive laws remains useful. A generic GPU expression interpreter, exact global remap optimization and nonlinear anisotropy in the first release would materially expand the project.

## 3. Corrections and additions to the specification

### A. Prescribed fields need an exception to flux continuity

Preserving every Q through a material event conflicts with prescribed `Q=P(g,t)` when nonzero g and the constitutive map change. The accepted exception is now in specification sections 3.2 and 8.1.

Accepted contract: preserve free Q and b; enforce prescribed primary fields at the event boundary and account ΔQ as external boundary exchange. Subsequent drift/history stages use the prescribed u. Apply the same rule to newly constrained nodes and distinguish it in identity/invariant tests. Homogeneous g=0 with maps through the origin is the simple special case.

This resolves the original specification conflict; no boundary potential is needed.

### B. Thin gaps are part of Gate B

The current toy supports conservative thin-gap coupling of duplicated traces. Specification section 4.3 now includes it explicitly.

For the linear port, use physical jump history `ż=T u`, force `Q̇_gap=-TᵀKg z` and energy `½zᵀKg z`. This reproduces the compatible linear spring term without bulk potential. Include stiffness in CFL bounds, diagnostics and residuals. Trace orientation, initialization and history transfer remain explicit work; invariant-component metadata follows physical couplings.

Retain first and second order. The adopted passive rational law replaces the unstable legacy second-order extension; its corrected kick and physical histories must be implemented, not obtained by copying the old auxiliary array. New scenes default to second order, so that capability cannot be deferred past linear production cutover.

### C. AMR requires an explicit numerical port

The current `QuadraticSolutionSnapshot` and estimator consume aligned displacement, velocity, acceleration, volume acceleration, and radiation memory. Their residual measures the old scalar PDE. Merely renaming the readback fields would be wrong for nonlinear/time-varying constitutive media.

For the static linear migration, retain a justified scalar-equivalent estimator using synchronized derived quantities. Before dynamic/nonlinear media are enabled, derive constitutive first-order residuals or the correctly differentiated variable-coefficient equation, together with its normalization and interface/boundary terms. Recovered quantities must respect material and baffle traces.

Specify how material-drive frequencies and generated harmonics affect wavelength guidance. A floor based only on source frequency is insufficient for these media. Preserve the existing refinement/coarsening controller and resumable scheduling; replace the physics inputs and estimator where required. Do not claim an unchanged calibrated relative-error percentage without validating it again.

### D. Define law arguments before promising skin portability

Authoring rows are coefficient rows, not permanent primary/complementary solver roles. In EM, the existing storage slots mean ε and μ in both polarizations; TE makes ε complementary. Compile them into explicit electric/magnetic roles, then into primary/complementary evaluation sites. Mechanical needs an equally explicit adapter.

Start with isotropic Kerr and saturable response on their actual electric/magnetic field magnitudes. Reject unsupported signed-polynomial, reciprocal, or anisotropic combinations explicitly. Preserve linear anisotropy throughout. Add capability checks independently of scalar formula validity.

Flipping a coefficient's reciprocal flag preserves algebraic authoring information; it does not prove that a nonlinear map or its field argument survives a skin conversion. Conversion must be validated across all materials before activation. Oscillator names and acceleration text must remain unavailable until Gate O selects their equations.

#### Concrete nonlinear skin-conversion proposal and accepted editor contract

The editor, physical-field arguments and direct state below are accepted. Remaining unsupported Gate C conversion combinations need their constitutive derivations before enablement, not another state-selection spike.

The subsequent editor decision is accepted and incorporated into specification sections 5.1 and 11: linear mechanical materials expose `k₀(x)`; Simplified nonlinear editing exposes named parameters; Advanced nonlinear editing exposes `s₀(x)=1/k₀(x)` as **Reciprocal stiffness**, directly in the constitutive relation. Existing expressions convert symbolically without manual reciprocal entry. View changes preserve the map, runtime state, formulas, dirtiness, and undo history. Keep the existing factored relative-χ convention explicit; an absolute cubic coefficient `a₃=s₀χ` is a different representation, not a relabeling of the same control. Custom advanced laws survive opening Simplified intact.

**Compile physical constitutive maps before assigning solver roles.** A compiled law records its electric/magnetic or mechanical-adapter role, physical field argument, scalar/vector extension, direct/reciprocal coefficient expression, frame, admitted domain, inverse, energy, and differential bounds. The existing authored slots can remain for persistence compatibility, but `mass_law` must not mean “apply to whichever scalar is currently displayed.”

| Presentation | Primary map | Complementary map |
| --- | --- | --- |
| TM | `D_z(E_z)` | `B_perp(H_perp)` |
| TE | `B_z(H_z)` | `D_perp(E_perp)` |
| Mechanical, TE-equivalent adapter | `q(u)=ρ(u)u` | `ξ(e)=e/k(e)`, with independent `ξ=b` and `e` the complementary observable |

For isotropic vector response, coefficient arguments above mean the vector magnitude. The mechanical `e` is a named complementary, stress-like toy field; it is not the displayed scalar `u` or simply `|∇ψ|`. Define its normalized units and explain it in effective-law/help text. This follows the specification's mechanical-as-presentation contract; it does not introduce conventional nonlinear displacement elasticity.

**TM ↔ TE changes roles, not material definitions.** Electric Kerr remains `D=ε₀(1+χ|E|²)E`. It is a scalar nodal map in TM and a vector quadrature map, requiring inversion from D, in TE. Magnetic laws follow the analogous rule. Preserve material parameters, frame, drive phase, and coefficient-bound runtime switch factors. Keep canonical live state through the event, subject to boundary exchange and admission. Do not promise identical trajectories or preservation of separately named physical E/H fields across polarization changes.

**Mechanical ↔ TE preserves the compiled constitutive maps.** Associate `u` with the TE magnetic observable and `e` with its electric observable: `ρ(u)↔μ(H)` and `k(e)↔1/ε(E)`. The same coefficient expression, field argument, frame, domain, and runtime state travel together. For supported media with equivalent boundaries/sources, this is a presentation/authoring conversion and should preserve compiled operators, constitutive observables under renaming, and subsequent canonical evolution. Mechanical ↔ TM composes this adapter with the polarization role change above; it need not preserve trajectories.

For example, electric Kerr converted to the mechanical presentation becomes `k(e)=1/[ε₀(1+χ|e|²)]`, so `ξ=e/k(e)=ε₀(1+χ|e|²)e`. It must not become `k(u)` or `e=ε₀(1+χ|ξ|²)ξ`. Conversely, an authored direct mechanical stiffness `k(e)=k₀(1+χ|e|²)` produces `ξ=e/[k₀(1+χ|e|²)]`, which for positive χ needs a bounded domain below its radial turnover. That restriction belongs to the mechanical law from the outset; a skin switch must not invent or clip a bound to make conversion pass.

**Loss has two physical channels.** Electric loss follows E and magnetic loss follows H through all skins; primary/complementary roles change with polarization. Direct b supports loss-generated state outside `image(C)`, as demonstrated in the core spike. Actual f32 composition, histories and resource cost remain production checks. Legacy single-rate migration must specify its named channel assignment rather than doubling damping by assigning the full rate to both. Oscillator state remains behind Gate O.

**Admission is a single scene transaction.** Prepare all converted material data and support checks on the CPU. At a complete GPU boundary, test the latest Q, complementary flux, prescribed fields, and runtime ramp factors against the target maps and timestep envelope. Accept all materials and runtime mappings together, or retain the old scene/state and report the failing material/combination. A smaller valid timestep prepares a generation; a missing inverse rejects the conversion. Reciprocal views retain the underlying switch factor/ramp trajectory: do not replace a reciprocal of a ramp with a new linear interpolation between reciprocal endpoints.

**Delivery and evidence.** Stage 0 freezes this argument/support table and normalized units. Stage 1 adds compiler-role types, identity normalization, and authoring round-trip fixtures. Stages 2–6 establish linear/tensor adapter and live-switch parity. Stage 7 tests reciprocal temporal factors and mid-ramp conversion. Stages 8–9 add scalar/vector Kerr and saturable map parity, domain rejection, and GPU live admission. Stage 10 enables only supported combinations in the editor.

Required conversion tests: electric and magnetic laws retain their field arguments in both polarizations; Mechanical↔TE preserves compiled maps for supported media; round-trips preserve authored formulas/domains without expression growth; reciprocal factors and active ramps retain their trajectories; mixed-node/material-frame cases agree; a bad material rejects the whole scene transaction; target live-state domain rejection leaves the old state intact; TM/TE duality tests exchange electric/magnetic data rather than assert generic scene equality. Signed χ₁ and nonlinear anisotropy remain separately gated until their vector laws are selected and verified.

### E. Make source migration concrete, including DC and spatial normalization

The accepted migration uses the zero-initial-rate antiderivative. For legacy `a(t)=a₀+A sin(ωt+φ)`, convert to `r(t)=a₀t+A[cos φ-cos(ωt+φ)]/ω`, and assemble `S` from the immutable reference mass and current spatial carrier. At zero frequency use `(a₀+A sin φ)t`; evaluate near zero with the stable sinc identity in specification section 4.1. This replaces the earlier steady-harmonic-only proposal and explicitly defines the integration constant.

Recommended normal form for point/volume source controls is a primary-field rate multiplied by an explicitly stored generation reference mass, producing integrated `S`. This fits the existing Gaussian and volume carriers and the plan's immutable reference-normalization rule. Boundary weak loads retain separately defined edge-integrated units; prescribed-primary boundaries retain field units. Document what changes when a new generation changes the reference coefficients.

Store the converted waveform explicitly with schema/version handling; new sources use direct-drive units. Prescribed-primary boundaries retain field units, and weak boundary loads need separate edge normalization. Legacy DC acceleration retains its growing forcing rather than being silently reinterpreted or clipped. The core spike verifies zero/near-zero waveform algebra; finish converted-waveform edit anchors and production normalization in Stage 0. Matching legacy envelope transients is not required.

Continue evaluating every source on its current carrier. No persistent spatial antiderivative state is required. Envelopes, source changes during a run, boundary startup, and reset/phase behavior need one documented CPU/GPU rule.

Phase preservation uses a temporal phase/time anchor per source/material drive, updated at the actual GPU commit time when frequency changes. This is scalar event work, not field work. Explicit phase edits intentionally change phase. Keep the run-start shared envelope schedule through ordinary edits, so introducing a slower source does not re-attenuate an established run. Service paused Switch events at a zero-duration commit boundary. Mid-ramp reversal restarts from the current factor, accepting a derivative discontinuity; edited ramp bounds cover the current factor and complete new trajectory.

### F. Define conservation and transfer support using more than node identity

An unchanged node position does not imply unchanged lumped support: a neighboring refinement can change its integrated volume. Exact `Q` copies should mean an unchanged DOF **and unchanged geometric support/contribution contract**. Recompute affected-support transfer for nodes whose support changes, even if their coordinates match.

Prepare geometric weights `v_i`, transfer an appropriate density such as `Q_i/v_i`, and define retained/deleted support shares. Do not use material mass as an unexplained substitute for geometric volume. Identity transfers on unchanged geometry remain exact, subject to the prescribed-boundary exception above.

Gate T also needs a numeric acceptance policy: correction size relative to an absolute field/flux scale, maximum affected support, and admissible capacity. Normalize against an L1-like flux scale rather than only the signed component total, which can vanish for a large oscillating field. Reject a candidate if correction cannot fit the permitted support/domain.

Keep geometric/transfer connectivity, physical trace couplings and invariant-correction eligibility explicit. Do not blindly reuse the old `ConstantModes::free` flag. There is no bulk potential gauge graph; future oscillator references are model-specific.

### G. Failure-safe stepping needs an explicit global commit boundary

Per-node error flags alone cannot make an in-place GPU step atomic across workgroups. Stage candidate state separately, collect validation status in ordered dispatches, then commit only if the complete candidate step is valid. No workgroup-local barrier can supply a global acceptance decision.

Clock, force cache, boundary memory, runtime events, and invariant accounting must advance only with the accepted step/event. A later encoded step must see the failure latch. Include pulse, filter, invariant correction, and law-patch admission in the same discipline. GPU flags can use integer atomics; force gathering does not need floating-point atomics.

Specify deterministic priority for reset, authored edits, runtime Switch, pulse, and step requests. Recommended paused behavior: service pending events at a zero-duration complete-step boundary, so Switch is responsive without advancing simulation time. Hard periodic edges can remain unsupported initially; the specified smoothed square already supplies the scheduled time-crystal drive.

### H. Finish the consumer and precision contracts

Use constant-annihilating difference arithmetic for Cu and any CSR filter/reference operator. Subtract a local primary-field reference before applying basis gradients, with consistent gather assembly. Test f32 flux/totals/long-clock behavior directly; no bulk potential rebasing exists in this design.

Define a GPU clock using integer step/epoch bookkeeping and bounded local-time arithmetic, with compensated/split elapsed-time storage and phase reduction as needed. Select and test the layout before shader work. Test epoch rollover, long pauses, high-frequency drives, and timestep changes; a pair of counters without an accurate physical-time conversion is insufficient.

Far-field output currently assumes a homogeneous, isotropic, linear, lossless exterior. Preserve and extend those capability checks when the exterior becomes nonlinear or driven. Nonlinear material inside a valid homogeneous exterior is a different case. Do not feed instantaneous coefficients into a static exterior formula and call it validated.

Keep synchronized primary/complementary/auxiliary samples for point, curve, area, energy, far-field, and adaptation consumers. Energy diagnostics use discrete nodal and quadrature constitutive energies plus physical boundary/gap storage; a smooth visualization need not use the solver's energy integration rule.

## 4. Existing work: retain, revise, complete

| Existing change | Disposition | Reason/work remaining |
| --- | --- | --- |
| `ScalarField::spatially_constant`, `evaluate_constant`, and formula coordinate-use detection | Retain | Useful distinction between spatial coefficients and parameter-only drives/bounds. |
| `FieldLaw`, `TimeDrive`, `CoefficientLaw`, `DampingLaw` structural composition | Retain as authoring scaffolding | Matches one field law, drive, alternate; do not confuse these with compiled constitutive maps. |
| Explicit `inverted` flag | Retain representation | Revise conversion claims and validate each supported physical interpretation. |
| New material fields and inert defaults | Retain intent | Complete imports, all constructors/fixtures, persistence, and support checks. Restoring remains gated. |
| Shared parameter traversal and staged rename in `Material` | Retain | Finish app deletion/reference checks and volume-source references/rename transaction. |
| Material-frame traversal | Retain and extend | Travelling phase uses the frame even when every drive parameter is spatially constant. |
| Direct scalar multiplier and derivative helpers | Reuse after numeric review | Useful scalar fixtures, not complete vector inverses or stage solvers. |
| Tangent-range routines and tests | Rework before use | Concrete defects below; sampled extrema are not certified CFL bounds. |
| Drive evaluation | Reuse structure; change convention | Existing implementation uses sine; plan specifies cosine. Add small-sharpness treatment and reduced phase. |
| Effective-law formatting | Retain framework; rewrite semantics | Current TE electric-law label uses H; restoring text claims an acceleration equation not yet derived. |
| `PhysicsModel::convert_material` law swapping | Retain linear/algebraic pieces | Add capability, physical argument, rate, and live-state admission checks. |
| Added `..Material::default_medium()` literals | Retain | Mechanical completion work, currently incomplete. |

### Concrete defects found by inspection

1. **Every bounded direct polynomial is rejected.** `FieldLawValues::tangent_range` passes `[-bound, bound, 0.0]` to `range_at` as `extra`. That function treats them as tangent values, not sample locations, making the minimum nonpositive. The bounded Kerr fixture expecting `(1,13)` cannot pass.

2. **The reciprocal-saturable regression expects the wrong minimum.** With `a = χ u_s² = 3`, the reciprocal tangent has minimum `5/32 = 0.15625`, at `u²/u_s² = 3/4`; the test expects `1/4`, its asymptotic value. More generally its nontrivial extremum is at `v = 3/(1+a)` and has value `(8-a)/(8(1+a))`. This confirms the specification's `a < 8` threshold, not the existing fixture.

3. **Sampling is not a sufficient lower-bound certificate.** `sampled_range` scans 4096 intervals. It can overestimate the true minimum and miss a narrow inadmissible interval, especially near a threshold. Use analytic extrema for these simple rational laws or a conservative interval method. The claim that 100 saturation amplitudes reaches the asymptote to double precision is also false in general.

4. **Passive polynomial loss does not enforce passivity.** `RateLaw::valid` checks finite coefficients and an optional positive bound, but not nonnegative rates over the admitted domain. Its van der Pol variant lacks the planned bound and supplies only a rate expression, not a derived oscillator model.

5. **Travelling drive frame dependence is missed.** `varies_in_space` examines the parameters' formulas; constant drive parameters still produce `q·x_material` dependence. `uses_frame` needs to include this behavior.

6. **Text and conversion overclaim physical equivalence.** `SkinNames` supplies one scalar field argument to both constitutive rows. A TE ε law is consequently printed as a law of H, contrary to the specification. Module comments and conversion comments claim that reciprocal flags alone preserve physics and that restoring acceleration is invariant. Those statements do not hold for the new core without the gated derivations.

7. **Inert reciprocal forms need normalization.** `through_reciprocal` toggles `inverted` even for a completely inert law; `is_linear` tests structural equality against the non-inverted default. Linear conversions can therefore be misclassified as active laws and display division by 1. Normalize identities before selecting capabilities or fast paths.

8. **No solver-level rejection exists yet.** `Material::evaluate` still reads only base coefficients. Valid authored laws can otherwise reach an old solver which ignores them. Scaffolding must not expose or silently accept enabled unsupported physics.

### Actual verification status

Ran `cargo test -p funfern-core --lib --locked` without source edits. It failed to compile: `geometry.rs` does not import `CoefficientLaw`, `DampingLaw`, or `RestoringLaw`; the material literal in `mesh/amr.rs` is missing the new fields. No numerical tests ran.

Inspection also found incomplete app literals in the GRIN example and `topology_persistence.rs` decoder. `StoredMaterial` and its encoder do not contain law fields. Those are inspection findings, not results of an app build. The tangent defects above are established from the code/arithmetic, not a claimed successful test run.

## 5. Implementation stages

The user has adopted direct `Q,b` and the passive three-state second-order outgoing law with corrected force-coupled midpoint kicks. Architecture selection is complete. The [core spike](funfern-material-laws-spike-report.md), [auxiliary derivation](funfern-boundary-auxiliary-spike.md), and [scattering correction](funfern-boundary-scattering-spike.md) are the evidence; their limitations remain gates below. No production implementation stage has started. Each stage is a reviewable change with focused tests and an engineering-log entry.

### Stage 0 — Close implementation contracts and establish acceptance cases

Scope: use specification sections 2–9 as the selected architecture, not a choice between potential and direct state. Inventory every state consumer, physical boundary history, serialized field whose meaning changes, and supported skin/material/boundary combination.

Finish concrete contracts for:

- Point/volume/weak-boundary source units, reference normalization across generations, legacy converted-waveform edit anchors, startup/reset, and legacy single-rate loss-channel assignment.
- Trace junctions/prescribed intersections, thin-gap orientation/initialization, outgoing modal-history mapping under basis/operator/topology changes, and new/deleted-edge energy accounting.
- Scalar support shares, local six-sample vector transfer, affected-support correction limits, new-cell donors, and invariant component eligibility.
- Stage synchronization, event priority, paused service, clock/epoch layout, accepted/candidate ownership, status priorities and recoverable failure.
- Initial capabilities: linear tensors retained; Kerr/saturable first; signed χ₁, general reciprocal nonlinear maps, nonlinear anisotropy and oscillator models remain gated.

Record a clean baseline build separately from the dirty authoring branch. Measure representative meshes/boundary sizes, bytes, wall time per simulated second, preparation slices and handoff latency on target hardware. Set matched-accuracy acceptance criteria before judging implementation results. Numerical tolerances from the spike READMEs remain pinned for those fixtures; define production f32 and irregular-mesh thresholds explicitly.

Acceptance inventory: free/prescribed modes; disconnected components; rotated anisotropy; thin gap; both outgoing orders; the previous complementary-loss instability; planar scattering with angle/wavelength and fixed-CFL refinement; curved/ignored topology reproducer; nonzero Dirichlet plus material edit; material-frame junctions; AMR cycles; baffle split/merge; new/deleted cells; active sources/ramps during retiming.

Exit: the initial linear work has equations, state ownership, code integration owners, reproducible fixtures and declared tolerances/budgets. Nonlinear/oscillator details may close in their dependent stages, but linear boundary/history/transfer work cannot be deferred until after cutover. No new broad spike is needed.

### Stage 1 — Finish safe, inert material authoring infrastructure

Scope: complete the existing types/imports/literals, parameter traversal, reciprocal identity normalization, analytic scalar validation and passive-rate checks. Fix drive convention, frame dependency and field-argument text. Compile physical electric/magnetic roles rather than treating authored rows as permanent primary/complementary roles.

Add `StoredMaterial` law encoding/decoding with inert defaults for version-22 documents. Version changed source/loss semantics explicitly. Traverse material and attached source references transactionally for rename/deletion. Reserve oscillator schema only under an explicit compatibility policy; do not enable its behavior. Authoring validity and currently executable capabilities are separate checks: no unsupported law may be silently ignored.

Primary files: `material_law.rs`, `material.rs`, `geometry.rs`, `wave.rs`, mesh/app material literals and fixtures, `topology_persistence.rs`, `topology_editor.rs`, source compilation and UI references.

Exit: workspace builds/tests pass; old scenes retain inert behavior; supported authored data round-trips; all section 4 inspection defects have regression tests; no nonlinear production behavior is enabled yet.

### Stage 2 — Compile constitutive data and implement the linear CPU core

Scope: separate geometric lumping weights, material-instance/frame contributions and immutable reference coefficients. Add synchronized owned `Q,b`, derived primary/complementary observables, cache generation tags, and extensible physical auxiliary state. Use six two-component quadrature samples per element and seven-node nodal assembly.

Implement linear scalar/tensor maps, direct KDK, constitutive energy, initialization and compatible scalar comparison. Preserve the rotated complementary inverse `R A Rᵀ` for existing stiffness tensor A. Keep all distinct node/material-frame contributions; the 32 authored-material limit is not a 32-contribution cap.

Prepare geometry/tensors/gather adjacency resumably from owned generation snapshots. CSR remains a comparison/filter/bounds tool or a separately justified cache optimization, not an assumed complete direct-state stepping path. Preserve nonpotential `b`; no bulk ψ or gauge maintenance.

Primary files: `wave_quadratic.rs`, `wave.rs`, a focused canonical-state/constitutive module, and convergence examples. Keep old production stepping temporarily for comparison.

Exit: compatible linear force matches existing K; both signs and tensor rotations pass; second-order conservative time convergence, uniform primary field, extra stationary modes and scoped initial-velocity compatibility are verified. Begin actual-core timing here; do not interpret NumPy timings as production predictions.

### Stage 3 — CPU sources, boundaries, losses, filter and reference remapping

Implement specification S/B/F/T against the CPU reference:

- Direct point/volume/weak-boundary drives, converted legacy waveform, pulse delta updates and prescribed-primary exchange.
- Independent physical loss channels using constitutively weighted frozen rates; second-order conservative evolution and the documented first-order varying-loss approximation.
- Physical thin-gap memory, both outgoing orders, energy/power accounting and stage-time source/prescribed handling.
- Passive auxiliary boundary using the [force-coupled midpoint kicks](funfern-boundary-scattering-spike.md#4-corrected-boundary-aware-kicks); eliminate auxiliaries to a trace solve. Do not use the rejected standalone boundary split.
- Linear paired filter on Q,b, spectral/invariant tests, and explicit stationary-mode limitations.
- Support-aware conservative Q transfer, local vector reconstruction/extension, physical gap/outgoing history maps, and bounded invariant maintenance. These CPU oracles are not a live field-transfer service.

Preserve old boundary semantics by derivation, not raw memory copying. Test basis sign/order/degeneracy invariance in modal-history transfer, changed boundary parameters, new/deleted edges, side restrictions and split/merge order independence. Include energy cost of edit initialization/removal.

Exit: linear S/B/F/T equations and fixtures pass before their GPU implementation; reflection/passivity/fixed-CFL regressions include the corrected scheme. Structured spike remaps are reproduced and production irregular/one-sided/repeated-transfer tests meet predeclared thresholds. Record nonlocal preparation/solve cost. Nonlinear boundary composition is still gated at Stage 8, not claimed by the standalone Kerr auxiliary test.

### Stage 4 — GPU linear evolution, clock and atomic acceptance

Write the shared buffer/layout manifest before shaders: accepted/candidate Q,b, physical auxiliaries, scratch/caches, constitutive and incidence tables, source/runtime records, clock, event serials, status and accounting. Include dense boundary transforms/factors and peak allocations; the core spike's main-state ratio excludes these.

Implement the CPU equations in real production-intended kernels with deterministic force gather and the eight-storage-binding ceiling. Use stage-specific bindings/packed offsets as needed; integers stay integer lanes. Keep constant-annihilating arithmetic for gradients and validate f32 stability of boundary transforms/solves.

Every writer stages candidate data; ordered dispatches validate globally, then accept all fields, history, clock, caches and accounting together. Later encoded steps honor a failure latch. Cover pulses, filters, law patches and maintenance, not just ordinary stepping.

Primary files: `wave_gpu.rs`, `wave.wgsl`, shared layout declarations/tests and GPU comparison harness.

Exit: f64/f32 agreement at declared tolerances; native/WASM checks; cross-file layout tests; long-clock/retiming and injected failures without partial commit; actual bytes/dispatches/throughput, including boundary work. Old production remains until transfer and consumers pass.

### Stage 5 — GPU transfer, physical histories and live events

Extend `QuadraticTransferJob` and map data with geometric support shares, local vector stencils, trace-side correspondence, bounded extension and affected correction support. Reuse valid exact-copy and baffle restrictions.

GPU order is specification section 8.5: latest-state local Q,b transfer; total reduction and bounded correction; prescribed exchange; physical history/runtime mapping; derived-state validation; atomic generation acceptance. No seam potential, gauge reduction or offset solve.

Transfer outgoing normalized histories through a basis-invariant physical mapping. Test changing modal bases/operators, thin-gap orientation, new/deleted boundaries, cycles and large components. Preserve source phase, ramps, clock, integer serials and immutable accepted generation indexing. Service events while paused, and keep editing/resume available after runtime failure.

Primary files: `transfer.rs`, both transfer shaders, `wave_gpu.rs`, UI/topology runtime candidate integration.

Exit: latest-state transfer, exact unchanged support/samples, bounded conservative AMR, constant-preserving extension, multiway order independence, physical-history continuity/admitted exchange and rollback. No added live CPU field-calculation round trip. Measure preparation slices, GPU commit duration, validation latency and peak old/new memory.

### Stage 6 — Port all consumers and switch production skins

Port scalar/vector rendering, point/curve/area probes, discrete bulk-plus-boundary energy and flow, far-field eligibility/sampling, readback adapters and static-linear AMR. Derive synchronized scalar-equivalent estimator inputs and include thin-gap/outgoing/interface terms. Revalidate error normalization. Segment or convert probe history explicitly when its physical meaning changes.

Switch Mechanical/TM/TE to the same direct state. Remove inverse-derivative reconstruction and display mean subtraction only when replacement consumers pass. Update buffer estimates, run/reset/step behavior, source help and energy terminology. Retain optional old-solver comparison only while useful; do not ship separate production formulations by skin.

Exit: first release boundary. Supported existing linear scenes, both outgoing orders, curved/topology regressions, static tensors, live edits/handoff/skin toggles and all consumers pass. Actual-core steady cost, matched-accuracy throughput, responsiveness and memory meet agreed budgets. If a gate fails, report the specific tradeoff; do not silently downgrade boundaries or discard history.

### Stage 7 — Time-driven media and runtime switching

Implement stage-time coefficient evaluation on both physical sides, harmonic/smoothed-square/travelling drives, Switch stamping, phase anchors, ramp reversal and trajectory bounds. Sample travelling phase in actual material frames at nodes/quadrature. Validate reciprocal factors as reciprocal trajectories, not newly interpolated endpoints.

Support timestep-safe table updates and generation retiming with admission against latest state. Verify ε-only and equal ε/μ temporal interfaces, shared-node independently driven materials, paused events and source edits during ramps. Preserve frozen run-start source envelope.

Before enabling combinations, derive/test dynamic AMR residuals, temporal-work diagnostics, filter behavior and boundary exterior/reference-impedance policy. Time-dependent boundary operator/history mapping cannot be silently approximated by resetting auxiliaries.

Exit: all enabled drives pass phase/event/energy/temporal-reflection and handoff tests; actual-core incremental cost is recorded.

### Stage 8 — Nonlinear CPU maps, bounds and composition

Implement assembled primary and radial complementary Kerr/saturable inverses, energies/Jacobians, bracketed solves, domain admission and tangent envelopes including frame/material junctions. Use analytic thresholds and f32-aware criteria; no clipping or replacing Q with an approximate forward-map result.

Derive and verify the corrected force-coupled nonlinear primary boundary kick, including interior forcing and sources. Use the auxiliary report's discrete-gradient energy identity as a starting point, not proof of the complete scheme. Verify complementary nonlinear force, gap/loss composition, nonlinear filter admission/energy and AMR residuals/normalization. Test boundary-touching nonlinear cases rather than imposing a blanket domain ban. Unsupported combinations stay unavailable pending their own evidence.

Exit: CPU scalar/vector maps, duality/Mechanical↔TE contracts, near-bound/domain failures, full nonlinear boundary composition and declared accuracy/passivity tests pass for enabled combinations. Signed χ₁, reciprocal nonlinear variants and nonlinear anisotropy close Gate C separately.

### Stage 9 — GPU nonlinear evolution and recoverable failure

Port admitted CPU maps/compositions with conservative compiled envelopes, cached initial guesses and bounded safeguarded inverses. Validate at actual stages, not delayed display readback. Admit changed maps/pulses against latest Q,b and current/full ramp trajectory. Domain failure rejects the edit; a valid smaller timestep prepares a generation.

Invalid candidate edits preserve the running accepted simulation. Runtime failure pauses at the last accepted complete state; corrective editing and resume remain available. No automatic clipping/reset. A bounded smaller-step retry is optional only after its own evidence, and cannot repair a nonexistent inverse.

Exit: CPU/GPU parity for nonlinear bulk/boundaries/loss/filter/AMR; injected failure/rollback/recovery; source/material/skin edits near domain limits; measured cost of primary versus complementary and boundary nonlinear solves. Enable only combinations that pass.

### Stage 10 — Complete UX, presets and compatibility

Finish Simplified/Advanced views, effective-law formatter, preset factories, Switch, stability readouts and source-frequency helper. Preserve symbolic linear k₀ / Advanced s₀ views, relative χ semantics, Custom laws, document/undo state and runtime ramps. Parameter rename/preset application is transactional; test capacity/collisions and actual spatial sample errors.

Editing controls accompany Stages 7–9 for validation; this stage completes the product experience. Add Kerr/pump/time-crystal/travelling examples only after their claims pass. Unsupported Gate C variants remain filtered out, not silently converted.

Exit: second release boundary. Enabled laws/presets round-trip and compile/assemble; all live edit/skin/undo/source/ramp workflows pass. README, architecture, help, plan and engineering log accurately describe delivered behavior and measured costs.

### Stage 11 — Oscillator and active-medium extensions

Close Gate O separately for KG, sine–Gordon, φ⁴ and van der Pol. Prefer explicit physical auxiliary state with derived coupling. A nonlinear force of a potential must not be advertised as a nonlinear acceleration of the displayed field.

Specify equation, energy/gain, amplitude/phase units, domain, references, timestep, initialization, skins, diagnostics and history transfer. Any new reference freedom belongs to that physical extension, not a resurrected bulk gauge system. Budget state and GPU stages explicitly.

Exit: KG dispersion, actual derived sine–Gordon kink, φ⁴ equilibria/domain behavior and chosen van der Pol gain/limit-cycle behavior pass independently. These are included scope, but not blockers for the first two release boundaries.

## 6. Dependency and verification policy

Order: completed spikes/adoption/consolidation → 0 → 1 → 2 → 3 → 4 → 5 → 6 → 7 → 8 → 9 → 10 → 11. Some authoring polish and independent CPU fixtures can overlap, but feature enablement respects the gates. Implement only one bulk state architecture.

Use analytical/manufactured f64 cases, then f32/GPU parity, then interaction/performance acceptance. The spikes are isolated oracles, not production code or GPU synchronization evidence. Compare trajectories at expected discretization accuracy rather than demand bitwise equality with old startup/history levels.

Run focused tests during each stage, then formatting, Clippy, workspace tests, native and WASM checks at integration boundaries. Use GPU/transfer/AMR/convergence/reflection/browser harnesses when their paths change. Interactive checks use isolated app/site data; do not touch user autosave.

Fix new-fixture thresholds and target-device budgets before judging results. Track time per simulated second at matched accuracy, steady stepping, resumable preparation, trace solve, transfer/validation latency and peak memory separately. Do not claim one dispatch, constant memory, exact curved DtN, universal reflection improvement or unconditional nonlinear stability from the existing evidence.

## 7. Handoff status

Planning and correctness spikes are complete enough to start staged implementation. The selected design is fully reconciled in the [specification](funfern-material-laws-plan.md). The remaining tasks are explicit implementation-stage gates, not unresolved architecture alternatives.

Start with Stage 0's bounded contract/fixture work and Stage 1's inert baseline repair. Preserve the unfinished user changes and the concrete reusable-work audit in section 4. No production solver/editor migration has been performed by planning, and the last production build check was red for the recorded incomplete scaffolding. The committed experiment artifacts retain both passing and expected-failing results; do not erase historical failures or use them as a current all-tests-pass claim.
