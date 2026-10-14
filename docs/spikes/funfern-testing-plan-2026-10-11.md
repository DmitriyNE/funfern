# Testing plan — 11 October 2026

The user's plan for the testing campaign, written on 11 October 2026 and
kept here as written, a record that is not edited. The work is ordered by
`docs/spikes/funfern-testing-campaign-plan.md`, which keeps this plan's
fixture identifiers (PH, IV, AMR, RT) and takes each fixture's content from
here.

**Scope:** Numerical physics, absorbing boundaries, discretization, state transfer, adaptive meshing, runtime correctness, GPU/browser execution, and robustness.

**Objective:** Establish that funfern not only executes its numerical formulation consistently, but also solves the intended physical problems accurately, preserves the required mathematical properties, and behaves correctly under interactive operation.

This plan extends the existing tests rather than replacing them. In particular, the current end-to-end infrastructure already exercises cavity evolution, submission batching, material switching, handoffs, remeshing, numerical rejection, and device loss across the application and GPU.

The largest remaining opportunity is **independent physical validation**, followed by more systematic testing of invariants, asynchronous execution, and adaptive discretization.

## 1. Verification strategy

Every fixture should distinguish four kinds of correctness.

| Layer | Question | Reference |
|---|---|---|
| **Implementation** | Does the GPU correctly execute the intended numerical scheme? | f64 reference on the same discretization |
| **Discretization** | Does the numerical scheme converge to the governing equations? | Analytical solution, manufactured solution, or independent high-accuracy solver |
| **Physical approximation** | How accurately does the chosen model approximate the intended physics? | Exact DtN, exact constitutive relation, or another known physical result |
| **Application integration** | Does the actual application execute and observe the correct calculation? | Independent invariants, event timelines, generation identity, accepted-state checkpoints |

These should produce separate test results. In particular, a GPU/f64 comparison must never substitute for a physical oracle, and a physically approximate absorbing boundary must not be required to converge to exact transparency merely through mesh refinement.

### 1.1 Common requirements

Each numerical fixture should:

1. Construct a complete authored scene and run through production preparation, assembly, GPU execution, and readback.
2. Exercise a specified physical configuration, including nonzero fields on every relevant state channel.
3. Compare results at identical **accepted physical times**, not merely matching frame counts.
4. Verify finite values, expected array dimensions, generation identity, and activation of the intended solver features.
5. Measure both global error and localized errors capable of exposing small but serious defects.
6. Report spatial and temporal convergence separately where applicable.
7. Retain enough information to reproduce failures across platforms.

The current `e2e` fixture mechanism is an appropriate integration point. Extend its scenarios and observation facilities rather than building another application driver.

### 1.2 Independent reference implementation

Create a small reference package, preferably Python with NumPy/SciPy and arbitrary-precision support where needed.

It should provide:

- Analytical wave fields and derivatives.
- Bessel, Hankel, and modified-Bessel functions and stable logarithmic derivatives.
- Exact planar and circular DtN symbols.
- Fresnel and cylindrical-scattering coefficients.
- High-accuracy ODE integration.
- Independent numerical differentiation and quadrature.
- Modal decomposition and complex-amplitude fitting.
- Convergence and error-budget evaluation.

**Do not reuse funfern's FEM assembly, integrator, boundary approximation, or transfer routines to construct the physical oracle.**

Those implementations remain appropriate for a separate GPU/f64 parity check.

For exact analytical fields, evaluate the result at physical coordinates and times. For comparisons involving integrated degrees of freedom, independently evaluate the corresponding integrals rather than treating point samples as their equivalent.

### 1.3 Standard numerical measurements

Use a consistent set of metrics:

| Metric | Purpose |
|---|---|
| Relative weighted L2 error | Global field accuracy |
| Scaled L∞ error | Localized defects |
| Complex amplitude and phase error | Harmonic scattering and dispersion |
| Frequency error | Eigenmodes and nonlinear oscillators |
| Complex reflection coefficient | Absorbing boundaries and interfaces |
| Energy-balance residual | Conservation, dissipation, and work |
| Convergence order | Verification of spatial and temporal schemes |
| Accepted-state hash | Strict reproducibility within the same backend |
| Generation/time/event identity | Application correctness |

For convergence, record:

`p_observed = log₂(error(h) / error(h/2))`

Only evaluate an expected convergence order in a range where discretization error dominates reference error and floating-point noise.

The existing approximately `5×10⁻⁶` GPU/f64 acceptance threshold can remain for the established short fixtures. **Do not apply that threshold indiscriminately to physical error.** Each physical test needs a budget derived from its analytical solution, theoretical convergence, and independent reference accuracy.

---

## 2. Physical benchmark suite

### 2.1 Bulk wave propagation

#### PH-01: Rectangular cavity eigenmodes

**Priority: P0**

Use a homogeneous rectangular domain with reflecting boundaries and no physical or numerical loss.

For Neumann boundaries, an analytical eigenmode is:

`φₘₙ(x,y) = cos(mπx/Lₓ) cos(nπy/Lᵧ)`

with frequency:

`ωₘₙ = πc √[(m/Lₓ)² + (n/Lᵧ)²]`

The initial canonical state must represent the corresponding analytical field and its compatible complementary field.

**Test cases**

| Case | Purpose |
|---|---|
| m=1, n=0 | Propagation along one coordinate |
| m=0, n=1 | Propagation along the other coordinate |
| m=1, n=2 | Genuine two-dimensional mode |
| m=2, n=3 | Higher spatial frequency |
| Superposition of two modes | Independent modal evolution |
| Spatially constant mode | Conservation of the zero-frequency component |

Measure:

- Modal frequency and phase over multiple periods.
- Amplitude error and unintended damping/growth.
- Spatial field error in Q and b, interpreted through their physical constitutive variables.
- Conservation or bounded oscillation of the appropriate discrete energy.
- Error under independent refinement of mesh spacing and timestep.

**Acceptance:** Convergence to the analytical solution at the expected rate. Specifically test the claimed fourth-order temporal phase accuracy of the modified Störmer scheme on its applicable linear conservative problem.

For that temporal test, compare frequencies against the **semidiscrete** reference, or use temporal Richardson extrapolation. Otherwise, fixed spatial dispersion contaminates the observed temporal order.

Repeat representative cases in mechanical, TM, and TE modes.

#### PH-02: Plane-wave propagation and dispersion

**Priority: P0**

Use analytical plane waves, with prescribed boundaries or manufactured forcing where necessary.

Sweep:

- Propagation direction.
- Wavelength relative to mesh size.
- Constant isotropic and diagonal anisotropic materials.
- Mesh orientation and element aspect ratio.
- Time step relative to the stability limit.

Measure the numerical dispersion relation, group velocity, polarization-dependent field reconstruction, and phase error.

**Acceptance:** Agreement with the physical dispersion relation under refinement, with the predicted spatial and temporal error behaviour.

This fixture is particularly useful for separating errors caused by the element family from those caused by the time integrator.

#### PH-03: Independent complementary-field modes

**Priority: P0**

This is specific to funfern's formulation and should not be omitted.

Construct states with nonzero complementary flux b that are not merely reconstructions from the primary field Q.

Use a nonzero discrete force-free component, satisfying the appropriate kernel condition for the canonical coupling.

Test:

- Preservation under source-free evolution.
- Absence of spurious primary-field excitation.
- Preservation of the complementary energy contribution.
- Behaviour under neutral remeshing and topology edits.
- Preservation of separated-side information across baffles.

**Acceptance:** The independent complementary component is not spuriously projected away, regenerated from Q, or transferred to an unrelated trace.

This explicitly tests the mathematical distinction that motivated evolving b independently.

---

### 2.2 Planar absorbing boundaries and exact DtN

#### PH-04: Planar modal reflection

**Priority: P0**

Test first-order matched and second-order absorbing boundaries against the exact homogeneous half-space response.

For an outgoing harmonic wave with frequency ω and tangential wavenumber kₜ:

`k = ω/c`

`q = √(k² − kₜ²)`

The exact harmonic DtN relation is:

`∂ₙu = iq u`

using time dependence `exp(−iωt)` and the outgoing/decaying branch.

The equivalent power-conjugate admittance must be converted according to funfern's constitutive variables.

**Geometry:** A strip with one absorbing end and controlled transverse modes. Reflecting transverse boundaries can provide modes `kₜ = mπ/W`. Alternatively, use a Bloch-periodic strip if supported.

**Parameter sweep**

| Parameter | Cases |
|---|---|
| Incidence angle | 0°, 15°, 30°, 45°, 60°, 75°, 85° |
| Spatial resolution | 8, 16, 32, 64 cells/wavelength |
| Temporal resolution | Several independently refined timesteps |
| Boundary law | First-order matched, second-order passive |
| Material | Homogeneous lossless; representative wave impedances |
| Modal regime | Propagating, near-cutoff, evanescent |

For propagating modes, fit the field to incoming and reflected components:

`u(x) = Aᵢ exp(iqx) + Aᵣ exp(−iqx)`

and measure the complex reflection coefficient:

`R = Aᵣ/Aᵢ`

At first order, the analytical continuum result is:

`R₁(θ) = −tan²(θ/2)`

Thus the expected reflection is zero at normal incidence but nonzero at oblique angles.

For second order, use the **actual rational boundary response** implemented by funfern, rather than substituting a textbook second-order truncation. The adopted passive approximation has a documented accuracy/passivity tradeoff and need not outperform every alternative at every angle.

For each configuration, evaluate:

1. `R_numeric − R_implemented`: implementation/discretization error.
2. `R_implemented − R_exact`: intrinsic absorbing-boundary approximation error.

The first should converge toward zero. The second generally should not.

Near propagation cutoff, fit the complex admittance or field response directly. Do not rely on an ill-conditioned decomposition into propagating amplitudes.

For finite-resolution reflection measurements, distinguish numerical interior dispersion from boundary reflection. Your existing Bloch-strip scattering work already established the appropriate methodology.

**Acceptance:** Correct complex reflection magnitude and phase, appropriate convergence, stable evanescent response, and no unexpected active behaviour from passive boundaries.

#### PH-05: Planar boundary passivity and stability

**Priority: P1**

Exercise the absorbing-boundary subsystem across its supported material combinations.

Cases:

- Lossless interior.
- Primary loss.
- Complementary loss.
- Spatially varying loss.
- Supported nonlinear constitutive laws.
- Driven materials, with external work accounted for.
- Frequencies approaching troublesome auxiliary poles.

Measure the boundary's supplied power, stored auxiliary energy, dissipation, and total energy exchange.

**Acceptance:** Correct energy balance and bounded evolution within the documented stable operating range.

Do not require instantaneous boundary power to be positive. A passive boundary with memory can return previously stored energy.

---

### 2.3 Cylindrical absorbing boundaries and exact DtN

These should be **two distinct fixtures**.

The analytical machinery is similar, but the outward normals, radiation direction, and physical interpretation are different.

#### PH-06: Outgoing radiation through a circular outer boundary

**Priority: P0**

Use an annular computational region with a driven inner boundary and an absorbing circular outer boundary.

For harmonic time dependence `exp(−iωt)`, the outgoing radial solutions are Hankel functions of the first kind:

`uₘ(r,φ) = Hₘ⁽¹⁾(kr) exp(imφ)`

The exact circular exterior DtN eigenvalue is:

`Tₘ,out(k,R) = k Hₘ⁽¹⁾′(kR) / Hₘ⁽¹⁾(kR)`

Test the implemented first- and second-order conditions against this exact response.

**Parameter sweep**

| Parameter | Cases |
|---|---|
| Angular order | m = 0, 1, 2, 4, 8 |
| Dimensionless radius | kR ≈ 0.8, 2, 5, 10 |
| Boundary law | First-order, second-order |
| Excitation | Individual modes and selected superpositions |
| Geometry | Circular boundary at several mesh resolutions |

Include configurations with `m ≈ kR`, where radial behaviour changes rapidly.

Fit the numerical radial field as:

`uₘ(r) = A Hₘ⁽¹⁾(kr) + B Hₘ⁽²⁾(kr)`

Then measure `B/A`.

For exact outgoing DtN, `B/A = 0`. For approximate absorbing conditions, compare against the reflection predicted by the corresponding boundary law.

Also verify the predicted large-radius behaviour and curvature correction.

**Acceptance:** Complex reflection converges to the result of the implemented boundary condition; its deviation from exact circular DtN matches the predicted approximation error.

Control circular-geometry approximation separately. If the authored curve is not an exact circle, do not attribute its shape error to the boundary law.

#### PH-07: Inward radiation into an absorbing circular hole

**Priority: P0**

Use an annular domain with an absorbing inner boundary and drive an inward-propagating cylindrical wave from the outer region.

The inward cylindrical solution is:

`uₘ(r,φ) = Hₘ⁽²⁾(kr) exp(imφ)`

The outward normal of the computational annulus at its inner boundary points toward the centre. Therefore:

`Tₘ,in(k,R) = −k Hₘ⁽²⁾′(kR) / Hₘ⁽²⁾(kR)`

Test the same angular orders and dimensionless radii as PH-06.

Measure the ratio of reflected outward waves to incoming inward waves:

`uₘ(r) = A Hₘ⁽²⁾(kr) + B Hₘ⁽¹⁾(kr)`

`R = B/A`

**Acceptance:** Correct complex reflection, curvature response, normal orientation, and outward power-flow sign.

**Important physical distinction:** This is a frequency-domain ideal inward-radiation sink. It is not the ordinary DtN operator of a homogeneous disk extending to r=0.

A regular interior disk uses Jₘ, not Hₘ⁽²⁾, and can return energy. Its inward-normal DtN eigenvalue is:

`Tₘ,disk = −k Jₘ′(kR) / Jₘ(kR)`

The regular-disk problem can be included as a separate control, but it must not be labelled an absorbing hole. The ideal inward Hankel condition should not automatically be assumed to possess a passive, causal time-domain realization.

---

### 2.4 Global radiation and transient DtN

#### PH-08: Exact outgoing multipoles on rectangular boundaries

**Priority: P1**

Use outgoing cylindrical multipoles evaluated along funfern's rectangular outer boundary.

For a source location inside the rectangle:

`uₘ(x,y) = Hₘ⁽¹⁾(k|x−x₀|) exp(im arg(x−x₀))`

Both the boundary field and its normal derivative are known analytically.

Therefore, the exact exterior DtN map satisfies:

`T_exact[u|Γ] = ∂ₙu|Γ`

even though Γ is rectangular.

This provides exact input/output pairs for a **global nonlocal radiation operator** without implementing that operator itself.

Test:

- Centred monopole.
- Off-centre monopole.
- Dipole and higher multipoles.
- Sources close to an edge.
- Sources close to a corner.
- Multiple simultaneous multipoles.

Compare the approximate boundary response against the exact field/flux pair, preferably through consistent weak boundary integrals.

For the complete dynamic solver, use an analytically compatible source or prescribed boundary excitation and compare the resulting harmonic field.

**Acceptance:** Correct limiting behaviour under discretization refinement, quantitatively characterized approximation error, and correct global energy flux.

The exact DtN map for a general contour is spatially nonlocal. Four independent planar half-space DtN maps are not the exact rectangular DtN operator. This distinction matters especially near corners. Exact nonreflecting conditions and their nonlocal kernels have been studied extensively by Alpert, Greengard, and Hagstrom. :chatgpt-content-reference{index="0"}

#### PH-09: Transient DtN and boundary-memory response

**Priority: P1**

Use a broadband transient whose reference can be evaluated from an exact causal radiation kernel.

For circular exterior radiation, the Laplace-domain DtN eigenvalue is:

`Tₘ(s) = (s/c) Kₘ′(sR/c) / Kₘ(sR/c)`

where Kₘ is the modified Bessel function of the second kind and the exterior is initially unexcited.

Compute the reference response with an independent, converged inverse-Laplace or convolution method. Exact time-domain nonreflecting boundaries involve precisely this kind of history dependence. :chatgpt-content-reference{index="1"}

Test:

- Short broadband pulse.
- Narrowband pulse.
- Low-frequency excitation.
- Multiple angular modes.
- Long late-time tail.
- Pulse followed by silence.
- Repeated pulses separated by incomplete boundary-memory decay.

Compare boundary flux and reflected field against the causal reference.

**Acceptance:** Correct response shape, phase, late-time behaviour, and convergence. No spurious persistent reflection attributable to history-update errors.

#### PH-10: Boundary-memory transfer through handoff

**Priority: P1**

Create a propagating field and wait until the absorbing boundary has accumulated **nonzero auxiliary state**.

Then perform:

1. A no-physics-change rebuild.
2. Remeshing of the same geometry.
3. A material change requiring a new generation.
4. A timestep change, where supported.

Compare against an uninterrupted control, including:

- Q and b.
- Boundary auxiliary histories.
- Physical time and event phase.
- Stored boundary energy.
- Subsequent reflected waveform.

**Acceptance:** No unexplained history loss or energy creation, and bounded field error consistent with transfer discretization.

In particular, the physical time origin must survive a handoff even when the timestep changes.

---

### 2.5 Material interfaces, scattering, and internal coupling

#### PH-11: Planar material interfaces

**Priority: P1**

Use analytical reflection and transmission at a planar interface between two homogeneous media.

For electromagnetic problems, calculate the appropriate Fresnel coefficients independently for TE and TM.

Test:

| Configuration | Purpose |
|---|---|
| Normal incidence | Impedance and constitutive mapping |
| Oblique incidence | Polarization-dependent transmission |
| Impedance-matched interface | Suppression of reflection |
| Strong impedance contrast | Numerical conditioning |
| Total internal reflection | Evanescent transmitted field |
| Multiple interfaces | Phase accumulation and resonances |
| Anisotropic interface | Tensor orientation and boundary continuity |

Measure complex reflected/transmitted amplitudes, tangential field continuity, normal flux continuity, and power balance.

**Acceptance:** Convergence to analytical Fresnel results, including phase and the correct treatment of evanescent fields.

The full wave impedance must be used for TE/TM. A universal scalar impedance formula is insufficient.

#### PH-12: Circular-cylinder scattering

**Priority: P1**

Use an analytical cylindrical partial-wave expansion, sometimes called a 2D Mie-series solution.

A plane wave incident on a homogeneous circular inclusion or hole has known scattering coefficients involving Bessel and Hankel functions.

Test:

- Reflecting circular obstacle.
- Penetrable circular inclusion.
- Multiple material contrasts.
- TE and TM.
- Several size parameters `ka`.
- Different cylinder positions and orientations.

Measure:

- Near-field complex amplitude.
- Angular far-field scattering amplitude.
- Scattering and extinction quantities where applicable.
- Energy conservation.
- Convergence with mesh and circular-geometry resolution.

This validates substantially more than the absorbing boundary: **curved geometry, material interfaces, field reconstruction, angular scattering and far-field diagnostics**.

The analytical cylinder solution is a well-established benchmark for time-domain electromagnetic scattering calculations. :chatgpt-content-reference{index="2"}

**Acceptance:** Near-field and far-field agreement with the independently converged analytical series.

Account separately for outer absorbing-boundary error, or measure before contamination from outer-boundary reflections.

#### PH-13: Thin-gap and impedance interfaces

**Priority: P1**

Use a planar interface with a known impedance, compliance, or thin-layer transfer relation.

Construct the independent harmonic solution from the interface jump conditions or an appropriate 2×2 transfer matrix.

Test:

- Zero coupling.
- Strong coupling.
- Intermediate coupling.
- Resonant response.
- Conservative thin-gap storage.
- Dissipative impedance where supported.
- Two-sided boundary orientation.

Measure reflection, transmission, phase, interface jump, and stored energy.

**Acceptance:** Agreement with the analytical interface model and correct energy accounting.

This should include nonzero thin-gap history during at least one handoff.

---

### 2.6 Time-dependent and nonlinear physics

#### PH-14: Temporal material interfaces

**Priority: P1**

Use spatially homogeneous materials whose coefficients change at a specified physical time.

Begin with an instantaneous change, then extend to finite-time ramps and periodic modulation.

For electromagnetic temporal interfaces without impulsive free sources, use the appropriate continuity conditions on D and B.

Test:

- Permittivity change.
- Permeability change.
- Impedance-preserving parameter change.
- Simultaneous coefficient changes.
- Instantaneous and finite-duration transitions.
- Repeated temporal interfaces.

Measure:

- Field jumps and continuity conditions.
- Frequency conversion.
- Phase.
- Energy change attributable to modulation.
- Dependence on switching time.

**Acceptance:** Analytical temporal-interface relations and energy exchange are reproduced under refinement.

This is distinct from merely verifying that the switch event reached the device.

#### PH-15: Periodically modulated materials

**Priority: P1**

Use spatially homogeneous modulation with a reduced independent reference model.

Configurations:

- Sinusoidal mass-side modulation.
- Stiffness-side modulation.
- Constant-impedance modulation.
- Parametric amplification.
- Stable and unstable Floquet regimes.

Compare against independently integrated mode-amplitude equations or a high-accuracy Floquet calculation.

Measure frequency sidebands, growth/decay exponents, phase, and energy supplied by the modulation.

**Acceptance:** Correct Floquet behaviour and convergence, including cases close to the stability boundary.

#### PH-16: Nonlinear constitutive laws and oscillators

**Priority: P1**

Test separately:

**Constitutive inversion:** Prescribe an analytically known or independently integrated flux history and compare recovered physical fields against an independent high-precision root solver.

**Restoring-law dynamics:** Use spatially uniform or single-mode reductions of Klein–Gordon, sine-Gordon, and φ⁴ where the reduction is valid.

**Nonlinear dissipation/gain:** Use reduced van der Pol-type dynamics and independently computed limit-cycle properties.

**Combined laws:** Exercise selected compositions such as Kerr with damping, restoring force, or short-wave loss.

Measure:

- Constitutive residuals.
- Oscillation frequency and amplitude.
- Nonlinear energy.
- Limit-cycle amplitude and stability.
- Behaviour near admissibility bounds.
- CPU/GPU agreement through nonlinear stages.

For sine-Gordon's spatially uniform reduction:

`r̈ + ω₀² sin(r) = 0`

An independent high-accuracy ODE integrator gives a straightforward reference. The conservative case also admits an analytical energy integral.

**Acceptance:** Correct nonlinear trajectories, constitutive inversion, domain rejection, and energy exchange.

Include physically valid near-zero and large-amplitude cases. Do not make every nonlinear test a failure-domain test.

---

### 2.7 Energy, diagnostics, and general manufactured solutions

#### PH-17: Energy accounting

**Priority: P1**

Exercise each energy channel independently and in selected combinations.

| Configuration | Required behaviour |
|---|---|
| Conservative autonomous system | Bounded numerical energy error; appropriate modified-energy behaviour |
| Constant physical loss | Correct integrated dissipation |
| Primary and complementary loss | Correct allocation between energy channels |
| Passive absorbing boundary | Boundary loss plus auxiliary storage balances field-energy change |
| Conservative thin gap | Energy exchange with its stored state |
| Time-dependent material | External modulation work accounted for |
| Driven source | Injected energy matches measured work |
| Active nonlinear medium | Gain recorded with correct sign |
| Remeshing | Numerically induced energy change accounted for, not silently hidden |

Evaluate an independently assembled balance of the form:

`ΔE + W_dissipated − W_sources − W_modulation − W_edit = residual`

Use the energy definitions appropriate to the actual physical and numerical model.

For conservative symplectic stepping, do not generally demand exact conservation of the ordinary physical Hamiltonian at every timestep.

**Acceptance:** The residual converges appropriately, accounting is complete, and passive cases exhibit no unexplained sustained energy production.

#### PH-18: Analytical diagnostics and observables

**Priority: P1**

Test the quantities users actually inspect, not only internal state arrays.

Use known analytical solutions to evaluate:

- Point probes.
- Line and area probes.
- Vector field reconstruction.
- Boundary flux integrals.
- Time derivatives and probe timestamps.
- Energy density and total energy.
- Far-field angular patterns.
- Spectral measurements.

Compare probe values against independent analytical point evaluations or integrals.

For a harmonic field, also verify probe phase and derivative relationships.

**Acceptance:** Correct value, units, sign, physical timestamp, and interpolation/integration error.

Adding probes must not change the numerical solution.

#### PH-19: Manufactured solutions

**Priority: P2**

Manufactured solutions extend physical coverage to configurations without natural closed forms.

Choose a sufficiently smooth prescribed field `u*(x,y,t)` and derive the forcing and boundary conditions analytically from the governing equations.

Use this for:

- Spatially varying material properties.
- Anisotropic tensors with off-diagonal terms.
- Mixed boundary conditions.
- Nonuniform damping.
- Smooth coefficient modulation.
- Curved geometry and nontrivial forcing.
- Compatible nonlinear constitutive laws.

For a representative scalar wave equation:

`uₜₜ − ∇·(A(x,y)∇u) = f`

construct:

`f = u*ₜₜ − ∇·(A∇u*)`

with all additional terms included according to the tested physical model.

**Acceptance:** Independent residual construction and the expected convergence of the numerical solution to `u*`.

The forcing must be derived independently, not generated by applying funfern's assembled discrete operator to the desired answer. That would test internal consistency instead of consistency with the PDE.

---

## 3. Metamorphic and invariant tests

These do not generally require a closed-form solution. They test mathematical relationships that must hold between different simulations.

### 3.1 Transformation invariance

| ID | Priority | Test | Required invariant |
|---|---|---|---|
| **IV-01** | P1 | Linear superposition | Responses combine linearly for fixed linear physics and discretization |
| **IV-02** | P1 | Material/region ID permutation | Physical solution is independent of arbitrary IDs |
| **IV-03** | P1 | Mesh renumbering | Reordering nodes/elements does not change the represented physical solution |
| **IV-04** | P1 | Geometric symmetry | Translation, rotation and reflection preserve the corresponding transformed solution |
| **IV-05** | P1 | Observer independence | Probes, recording, rendering and UI state do not affect solver evolution |
| **IV-06** | P2 | Physical similarity scaling | Consistently rescaled nondispersive problems give corresponding scaled solutions |
| **IV-07** | P2 | Time reversibility | Conservative autonomous fixed-mesh evolution approximately recovers its initial state |

For geometric symmetry, rotate material tensor frames and vector quantities consistently. For scaling, transform material rates, frequencies, source strengths and time correctly.

Do not demand bitwise equality after a mesh renumbering if it changes floating-point reduction order.

### 3.2 Topology and representation invariance

| ID | Priority | Test | Required invariant |
|---|---|---|---|
| **IV-08** | P1 | Insert/remove identical-material separator | No physical discontinuity or unexplained field change |
| **IV-09** | P1 | Equivalent curve representation | Same physical geometry gives equivalent numerical solutions |
| **IV-10** | P1 | Neutral remesh | Conserved component quantities remain correct; field error is bounded |
| **IV-11** | P1 | Repeated remeshing | Transfer errors do not accumulate pathologically |
| **IV-12** | P1 | Baffle-side preservation | No transfer or extension across an impermeable separated trace |
| **IV-13** | P2 | Split/merge reversal | Returning to the original topology does not produce excessive accumulated field error |

Use nontrivial initial fields, including nonzero complementary modes and nonzero energy.

For IV-10, directly measure the conserved component totals before and after handoff. Do not rely exclusively on GPU/f64 agreement, because both may use the same erroneous transfer policy.

Distinguish a true identity operation, where strict preservation is appropriate, from remeshing onto a different approximation space, where some error is unavoidable.

---

## 4. Adaptive meshing verification

### AMR-01: Closed-loop adaptive simulation

**Priority: P1**

Use a localized propagating wave packet whose spatial structure evolves.

Run:

- A coarse fixed-mesh simulation.
- A fine fixed-mesh reference.
- The production adaptive simulation.

Measure:

- Physical-field error against the fine-mesh reference.
- Element count versus physical time.
- Refinement and coarsening locations.
- Estimator value and actual error.
- Cumulative transfer error.
- Adaptation work and application responsiveness.

**Acceptance:** Both refinement and coarsening occur where appropriate; the error stays within the declared budget; adaptation does not degenerate into global refinement.

### AMR-02: Estimator calibration

**Priority: P1**

Measure an effectivity index:

`I_eff = estimated error / independently measured error`

Sweep smooth waves, localized packets, driven media, material interfaces, and selected boundary conditions.

**Acceptance:** Effectivity remains reasonably bounded and does not deteriorate systematically under refinement within the supported problem classes.

An estimator's spatial distribution also matters. A globally reasonable scalar estimate can still refine the wrong elements.

### AMR-03: Long-term adaptation stability

**Priority: P2**

Run repeated refinement/coarsening cycles with propagating and oscillatory fields.

Look for:

- Excessive oscillation between mesh resolutions.
- Cumulative artificial damping or growth.
- Loss of independent complementary-field content.
- Violations of component conservation.
- Growing transfer error.
- Repeated or abandoned preparation jobs.

**Acceptance:** Bounded errors and resource use, with no persistent adaptation instability.

### AMR-04: Concurrent editing and adaptation

**Priority: P2**

Start adaptation while performing real user edits or new topology requests.

Verify that discarded adaptation results cannot publish over newer accepted generations and that the solver eventually runs the correct scene.

This overlaps the runtime fuzzing suite, but should retain a few focused regressions.

---

## 5. Runtime, device, and robustness testing

The existing protocol proofs and generated host-runtime tests provide a substantial foundation. The additional work should specifically target **actual asynchronous GPU execution** and behaviours that the protocol model cannot independently establish.

### 5.1 Device and concurrency fixtures

| ID | Priority | Fixture | Validation |
|---|---|---|---|
| **RT-01** | P1 | Real-device asynchronous fuzzing | Randomly interleave edits, resets, source events, handoffs, readbacks, and scene replacement without serializing every operation |
| **RT-02** | P1 | Submission partitioning | Identical endpoints for different GPU step-batching schedules, where identical arithmetic is expected |
| **RT-03** | P1 | Stale completion handling | Old readbacks, handoffs, or preparation results cannot replace newer accepted state |
| **RT-04** | P1 | Event ordering | Events are neither duplicated, omitted, nor applied to the wrong generation or physical time |
| **RT-05** | P1 | Failure and rollback | Device-side rejection preserves the last accepted state; subsequent retry behaves correctly |
| **RT-06** | P2 | Clock maintenance | Correct physical time, source phase, and auxiliary histories across clock rebasing and timestep changes |
| **RT-07** | P2 | Resource lifecycle | Repeated edits, remeshing, resets, and device recovery attempts do not leak resources |
| **RT-08** | P2 | Preparation scheduling | Large jobs remain responsive; cancellation and replacement do not produce stale results |

For RT-01, generate action sequences but execute them through the production application and GPU.

Record the actual sequence of accepted events and generations. Replaying a numerical reference should use that accepted timeline, not assume that events requested at identical wall-clock times necessarily reached identical simulation times.

**Acceptance:** Correct generation identity, no stale publications, and eventual quiescence after actions stop and the device completes its outstanding work.

The real-device fuzzing complements, rather than replaces, the existing protocol-model testing.

### 5.2 Numerical edge cases

**Priority: P1/P2**

| Category | Cases |
|---|---|
| Workgroup dimensions | N−1, N, N+1 around relevant dispatch and reduction sizes |
| Empty resources | Zero sources, probes, optional histories, or boundary modes |
| Floating-point range | Zero, subnormal, small normal, ordinary, and large finite values |
| Nonlinear inversion | Near-zero derivative, nearly singular Jacobian, values approaching the admissible domain |
| Material conditioning | Increasing coefficient contrast and anisotropy |
| Geometry | Very short edges, narrow gaps, nearly coincident vertices, challenging junctions |
| Timestep | Small timesteps, values near supported stability bounds |
| Time | Long simulation duration, large clock values, rebasing boundaries |
| Reduction | Cancellation-heavy fields, positive and negative contributions of similar magnitude |

For valid cases, require finite results and the relevant accuracy/invariance properties.

For invalid cases, require explicit rejection without corrupting accepted state.

Do not require exact cross-backend agreement for floating-point edge cases. Lavapipe, Metal, and WebGPU software implementations may differ in subnormal and transcendental handling.

### 5.3 Long-running numerical stability

**Priority: P2**

Run extended simulations of:

- Conservative cavity modes.
- Passive absorbing boundaries.
- Nonlinear oscillators.
- Driven parametric media.
- Automatically adapting meshes.

Track energy, phase, modal amplitude, physical time, mesh statistics, and memory use.

**Acceptance:** No unexplained long-term drift or instability beyond the independently established numerical error model.

For conservative integrators, bounded oscillatory energy error can be acceptable. Systematic secular growth inconsistent with the method is not.

### 5.4 Resource and lifecycle testing

**Priority: P2**

Repeatedly cycle through a controlled set of scenes and mesh sizes while creating and destroying:

- Mesh and operator generations.
- GPU buffers.
- Rendering and compute pipelines.
- Probe/measurement buffers.
- Full-state readbacks.
- Preparation and adaptation jobs.

After draining outstanding work, measure the retained live resources.

**Acceptance:** Resource use reaches a bounded steady state for a bounded workload.

Do not require allocator-reported process memory to return to its exact initial value.

### 5.5 Browser and platform matrix

Retain the existing browser test infrastructure, including build-identity verification and service-worker startup checks.

Extend it with representative **physics fixtures** rather than repeating every numerical permutation on every browser.

| Environment | Primary purpose |
|---|---|
| Native Lavapipe/Vulkan | Automated full device-path regression |
| Chromium + SwiftShader | Browser shader translation, WebGPU execution, and integration |
| Native Metal | Hardware GPU execution and numerical comparison |
| Chrome WebGPU on Metal | Browser/WebGPU backend agreement |
| Safari WebGPU | Independent browser translation and runtime behaviour |
| Native hardware Vulkan | Actual Vulkan driver coverage |
| Single-threaded browser | Cooperative preparation and fallback |
| Threaded browser with isolation | Worker and shared-memory execution |
| Pages-style service worker | Deployed hosting behaviour |

The same physical fixture should identify which backend ran it and compare its results against the appropriate independent reference.

---

## 6. Test infrastructure and execution model

### 6.1 Proposed organization

Retain the existing `e2e` feature and device/browser suites.

Add a dedicated **physics reference package** and declarative fixture definitions.

A possible organization:

```text
tests/
  physics/
    references/
      bulk.py
      planar_dtn.py
      cylindrical_dtn.py
      scattering.py
      temporal.py
      nonlinear.py
      manufactured.py

    fixtures/
      cavity_modes/
      plane_waves/
      planar_abc/
      cylindrical_outer/
      cylindrical_inner/
      rectangular_radiation/
      transient_dtn/
      fresnel/
      cylinder_scattering/
      thin_gap/
      temporal_interfaces/
      nonlinear/
      energy/

    analysis/
      fields.py
      modes.py
      reflections.py
      convergence.py
      energy.py
      reporting.py
```

The physical references should remain independent of the production Rust numerical implementation.

Each fixture definition should specify:

| Field | Description |
|---|---|
| Identifier | Stable test name |
| Physical model | Mechanical, TM, TE |
| Geometry | Authored scene and physical dimensions |
| Materials | Constitutive parameters and laws |
| Boundaries | All boundary types and parameters |
| Initial conditions | Complete physical/canonical initial state |
| Excitation | Sources, prescribed data, and event schedule |
| Checkpoints | Accepted physical times and relevant events |
| Reference | Analytical or independently computed result |
| Observables | Fields, modal amplitudes, histories, energy, probes |
| Sweep axes | Mesh, timestep, frequency, angle, mode order, etc. |
| Error budget | Applicable norms and tolerances |
| Expected convergence | Theoretical order and permitted deviations |

Keep the format declarative enough to permit a fixture to run over multiple discretizations and backends.

### 6.2 Production-path execution

A fixture should follow this sequence:

**Generate scene → prepare/assemble → initialize → execute → read accepted state → evaluate physical reference → calculate errors → report.**

The application should retain control of its real assembly and GPU submission.

Test-only functionality may provide:

- Deterministic accepted-step checkpoints.
- Explicit timestep selection within the supported stability range.
- Complete accepted-state readbacks.
- Controlled event injection through normal application handlers.
- Export of accepted mesh and operator metadata.
- Access to boundary auxiliary histories and energy accounting.
- Failure diagnostics and test-result reporting.

Avoid test-only numerical shortcuts in the production solver itself.

### 6.3 Numerical state and observables

For complete end-to-end coverage, expose or export:

| State | Required information |
|---|---|
| Primary | Q, physical primary field |
| Complementary | Both b components, reconstructed physical vector field |
| Integrated | r, where present |
| Boundaries | Outgoing-boundary auxiliary histories |
| Internal interfaces | Thin-gap and relevant interface histories |
| Materials | Runtime state of switches and driven coefficients |
| Energy | Stored, lost, injected, modulation, and edit-exchange contributions |
| Measurements | Probe values, time stamps, and sampling metadata |
| Runtime | Generation, accepted step, physical time, timestep, event serials |
| Geometry | Mesh, element connectivity, coordinates, boundary identification |

Not every fixture needs to inspect every quantity, but tests should be able to access the relevant physical state.

For time-dependent problems, maintain the physical time origin independently of the number of steps in the current generation:

`t = t_origin + N_generation Δt_generation`

That avoids introducing an artificial phase reset when reconstructing a reference after handoff.

### 6.4 Reference generation

Precompute high-precision analytical data where practical.

For example:

- Exact DtN eigenvalues.
- Complex Fresnel coefficients.
- Bessel/Hankel scattering amplitudes.
- Independent nonlinear trajectories.
- Exact modal frequencies.
- Reference convolution histories.

Reference generation should itself have tests:

- Precision-refinement checks.
- Independent identities and limiting cases.
- Agreement with alternative formulations where practical.
- Documented treatment of singularities and branch cuts.

For cylindrical DtN, validate Bessel-function logarithmic derivatives in difficult parameter regimes. A numerically unreliable special-function library would make a particularly irritating oracle.

### 6.5 Failure artifacts

Every failed fixture should retain:

1. Test identifier and reference version.
2. Exact authored scene.
3. Accepted mesh and physical parameters.
4. Initial state and accepted event timeline.
5. Generation/step/time history.
6. Reference and measured observables.
7. Error metrics and violated acceptance criterion.
8. Device, driver, browser, and build identification.
9. Relevant logs and numerical failure status.

For randomized tests, retain the **actual minimized action sequence**, not merely its seed.

---

## 7. Acceptance criteria

### 7.1 General numerical criteria

| Property | Acceptance |
|---|---|
| Finite state | All required numerical quantities finite |
| Backend agreement | GPU/f64 error within fixture-specific budget |
| Analytical agreement | Error below independently determined discretization budget |
| Spatial convergence | Observed convergence compatible with the element formulation |
| Temporal convergence | Observed convergence compatible with the applicable time integrator |
| Boundary reflection | Complex result converges to implemented boundary-law prediction |
| DtN approximation | Intrinsic error matches independently evaluated approximation |
| Conservation | Required invariants preserved within numerical bounds |
| Energy accounting | All required work/storage/dissipation channels balance |
| Handoff | Correct state/history transfer at the accepted physical time |
| Runtime | No generation mismatch, stale publication, or inconsistent event ordering |
| Rejection | Clean failure; accepted state remains recoverable |
| Reproducibility | Failure can be replayed with the recorded inputs |

### 7.2 Convergence requirements

For physics fixtures, require at least three discretization levels wherever convergence is part of the acceptance claim.

Use separate sweeps:

**Spatial:** Refine h while keeping temporal error negligible relative to the spatial error.

**Temporal:** Refine Δt on a fixed mesh, comparing against the semidiscrete reference or otherwise removing spatial error.

**Boundary approximation:** Refine both h and Δt to distinguish numerical convergence from the nonzero intrinsic approximation error.

**Long-time behaviour:** Measure error versus physical time, rather than only at one convenient endpoint.

An expected convergence order should only be enforced in its demonstrated asymptotic regime. If f32 precision becomes the limiting error, report the plateau rather than fitting a meaningless convergence slope.

### 7.3 Reflection and DtN criteria

For each boundary fixture, report:

`R_numeric`

`R_implemented`

`R_exact`

and:

`E_discretization = |R_numeric − R_implemented|`

`E_approximation = |R_implemented − R_exact|`

When exact DtN gives zero reflection, the second quantity reduces to the intrinsic reflection magnitude of the approximation.

For global operators, use suitable complex field/flux norms rather than trying to reduce every response to one scalar reflection coefficient.

**A boundary-law fixture passes when it converges to the law it claims to implement.** Its accuracy relative to exact DtN is a separately reported physical quality metric.

### 7.4 Numerical-error budgets

Avoid a single tolerance covering everything.

Use budgets that account for:

- Physical discretization error.
- Reference evaluation error.
- Time integration error.
- Float32 rounding and backend variation.
- Interpolation/projection error.
- Expected boundary approximation error.

The established short GPU/f64 fixtures can retain their existing tolerance until new evidence justifies changing it.

A new analytical test should begin with an f64 convergence study. Once its asymptotic behaviour is established, derive the f32 and browser acceptance limits from the resulting error floor.

### 7.5 Test sensitivity

For every major category, deliberately introduce at least one relevant defect.

Examples:

| Mutation | Test expected to fail |
|---|---|
| Change a sign in boundary flux | Planar/cylindrical DtN |
| Remove a boundary auxiliary update | Transient DtN |
| Reverse the normal on a hole | Inward cylindrical DtN |
| Perturb a constitutive coefficient | Bulk dispersion or Fresnel scattering |
| Omit modulation work | Temporal-interface energy |
| Zero the independent b component | Complementary-mode fixture |
| Drop component-total correction | Neutral-remesh conservation |
| Reset the physical clock on handoff | Source-through-handoff fixture |
| Duplicate an event | Runtime ordering |
| Return an older readback | Generation/checkpoint test |
| Change a probe timestamp | Analytical observables |

Mutation testing should demonstrate that the acceptance criteria actually detect the intended classes of defects.

---

## 8. CI and execution schedule

### 8.1 Every pull request

Keep the existing standard CI tests, Kani proofs, Lavapipe end-to-end fixtures, and SwiftShader browser smoke tests.

Add a compact selection:

| Category | PR subset |
|---|---|
| Analytical bulk | One low-order cavity eigenmode |
| Planar DtN | Normal and one oblique propagating mode |
| Cylindrical DtN | One outgoing mode and one inward-hole mode |
| Material physics | One Fresnel or temporal-interface fixture |
| Energy | One conservative and one passive case |
| Invariance | Superposition and observer independence |
| Runtime | Short real-device asynchronous sequences |
| Numerical edge cases | A few critical workgroup/conditioning cases |

Run the compact physical tests through the real application on Lavapipe.

For the browser PR gate, start with one physical fixture beyond the existing cavity parity test. Keep the larger matrix out of every-PR SwiftShader execution because its cost is substantial.

### 8.2 Nightly

Run the full physical suite, including:

- Complete planar DtN angle/frequency sweeps.
- Circular inner and outer DtN mode sweeps.
- Rectangular multipole radiation.
- Transient DtN and nonzero-memory handoffs.
- Fresnel and cylindrical scattering.
- Time interfaces and nonlinear dynamics.
- Energy accounting and manufactured solutions.
- Full metamorphic suite.
- Closed-loop AMR.
- Extended asynchronous fuzzing.
- Numerical edge-case sweeps.
- Resource-lifecycle tests.

Generate machine-readable numerical reports and convergence plots.

### 8.3 Periodic hardware runs

Run selected full-precision and browser suites on:

- Metal.
- Hardware Vulkan.
- Chrome WebGPU.
- Safari WebGPU.

Include both threaded and cooperative browser execution when relevant.

The physical reference is shared, but backend tolerances may differ after characterization.

### 8.4 Manual or research-oriented runs

Keep the most expensive investigations outside routine CI:

- Very long conservative integrations.
- Extreme grazing and high-order DtN modes.
- Difficult anisotropic scattering.
- High-precision reference generation.
- Large AMR studies.
- Extensive mutation campaigns.
- Performance and scaling characterization.

These should still produce reproducible reports, not just screenshots or informal observations.

---

## 9. Implementation roadmap

The suggested order is based on dependencies and diagnostic value, not simply the number of fixtures.

| Stage | Deliverable | Priority | Completion criterion |
|---|---|---|---|
| **0** | Common analytical-reference infrastructure | P0 | Independent oracle, physical-coordinate comparisons, convergence analysis, reproducible reports |
| **1** | Bulk eigenmodes and dispersion | P0 | Spatial/temporal convergence demonstrated on the actual GPU |
| **2** | Planar absorbing boundaries | P0 | Complex reflection matches the implemented first-/second-order laws and their predicted DtN approximation errors |
| **3** | Circular outgoing and absorbing-hole DtN | P0 | Correct angular modes, curvature behaviour and normal orientation |
| **4** | Independent complementary-field modes | P0 | Nonzero independent b preserved through evolution and neutral handoff |
| **5** | Energy and boundary-memory validation | P1 | Storage/work balance and nonzero-memory handoff validated |
| **6** | Global rectangular radiation | P1 | Outgoing multipole field/flux references and approximation error characterized |
| **7** | Fresnel and circular-cylinder scattering | P1 | TE/TM reflection, transmission, near field and far field validated |
| **8** | Thin-gap and impedance interfaces | P1 | Analytical scattering, history and stored energy validated |
| **9** | Temporal materials and nonlinear dynamics | P1 | Frequency conversion, constitutive inversion, nonlinear trajectories and energy exchange validated |
| **10** | Metamorphic/invariance tests | P1 | Superposition, symmetry, observer independence and neutral edits covered |
| **11** | Closed-loop AMR | P1/P2 | Error, effectivity, cumulative transfer and adaptation behaviour characterized |
| **12** | Asynchronous device fuzzing | P1 | Real-device interleavings with reproducible failures and consistency checks |
| **13** | Manufactured solutions | P2 | Coverage of variable coefficients, anisotropy and mixed boundary conditions |
| **14** | Robustness and lifecycle testing | P2 | Numerical limits, resource bounds, clock maintenance and long runs covered |
| **15** | Mutation testing | P2/P3 | Representative defects detected by the intended fixtures |

Some stages can proceed independently.

In particular, metamorphic tests and asynchronous fuzzing do not need to wait for the full analytical-reference framework. The existing infrastructure already supports much of their execution.

## 10. Definition of done

I would consider the expanded verification program mature when the following are true:

**Physical correctness**

- Fundamental wave propagation has demonstrated spatial and temporal convergence.
- Matched and second-order boundaries have measured complex reflection matching their analytical models.
- Planar, circular-outgoing, and circular-hole DtN benchmarks are covered.
- Material interfaces, temporal changes, and representative nonlinear dynamics have independent references.
- Energy accounting and important physical observables are validated.

**Numerical correctness**

- Q and b are both exercised independently.
- Remeshing preserves required conserved quantities and has bounded transfer error.
- Adaptive refinement/coarsening has measured effectiveness.
- Numerical edge cases fail cleanly or remain accurate within documented bounds.

**Application correctness**

- The real GPU and browser paths execute representative physics fixtures.
- Asynchronous operation cannot publish stale generations or misapply events.
- Device failures, rejection, and recovery preserve their defined invariants.
- Test results are reproducible and tied to exact builds and accepted states.

**Test credibility**

- Independent references do not reuse the implementation under test.
- Numerical tolerances are justified rather than guessed.
- Convergence claims are backed by actual refinement sweeps.
- Representative intentional defects cause the relevant fixtures to fail.

---

## 11. Final prioritization

For practical implementation, I would group the work into three milestones.

**Milestone A: Independent physics foundations**

PH-01 through PH-04, plus PH-06 and PH-07.

This establishes analytical bulk propagation, independent complementary-state behaviour, and both planar and cylindrical absorbing-boundary accuracy. It addresses the biggest remaining distinction between numerical self-consistency and physical correctness.

**Milestone B: Complete physical subsystems**

PH-05 and PH-08 through PH-19.

This validates nonlocal radiation, transient boundary memory, material interfaces, nonlinear behaviour, energy accounting, observables, and manufactured solutions.

**Milestone C: Systematic numerical and runtime stress**

The IV, AMR, and RT suites, together with mutation testing.

These establish that the solver remains correct under equivalent representations, automatic discretization changes, unusual numerical inputs, and arbitrary interactive execution order.

### The most important architectural decision

**Keep the physical oracle independent, but execute the physical fixture through the real application.**

That combination is what makes this plan valuable.

A standalone analytical comparison can miss an application integration defect. A GPU/f64 comparison can miss a shared numerical error. A protocol proof can miss incorrect numerical state. A screenshot can miss practically everything.

Together, the proposed fixtures address those distinct failure modes without requiring a second implementation of the entire funfern solver.

The foundation for exact circular time-domain nonreflecting references is well established in the work of [Alpert, Greengard, and Hagstrom (2002)](https://www.nist.gov/publications/nonreflecting-boundary-conditions-time-dependent-wave-equation). The existing funfern architecture, with independent Q/b evolution and transactional live remeshing, makes **complementary-field invariance, physical-time continuity, and boundary-history transfer** particularly important additions beyond conventional FEM verification.
