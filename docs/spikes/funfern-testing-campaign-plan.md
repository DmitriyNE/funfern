# Testing campaign plan — 12 October 2026

**Decision:** the campaign starts with the structure of the solver's inputs,
not with physics. Configuration sweeps at the preparation layer, regression
scenes that prepare rather than merely open, an audit of tests that feed a
solver something other than what the assembly feeds it, and mutations that
prove the gate bites come first. The physics fixtures follow, each one a
single Rust test at three discretization levels with its reference computed
offline, and their parameter sweeps live under `experiments/` as scripts.
This reorders the plan the user wrote on 11 October (its fixture identifiers
PH, IV, AMR and RT are kept below so the two documents map onto each other);
it keeps that plan's layering, its oracle rule and most of its fixtures.

## 1. Why structure first

The question is what has actually broken. The defects of September and
October, from `docs/engineering-log.md`:

| Defect | Class | What found it |
|---|---|---|
| The banded trace solve wrote a zero between two pieces at the other piece's position and erased a diagonal (9d19e20) | structure | the user's scene refusing to load |
| The second-order wall kept pinned corners on its trace and reflected 2.5 to 10 times its law beside pinned sides | structure | the reflection measurement against the law |
| A remesh lost its component totals, inside the device examples' 3e-5 | bookkeeping | the end-to-end cavity's 5e-6 |
| The far-field ring had 54 empty delay buckets inside its window | bookkeeping | an instrumented run of the app |
| Four defects and a design gap behind "impossible to delete any span" | structure | manual testing |
| Mesher decisions differed by one libm bit across platforms | platform | CI on Linux |
| Metal's fast math folds `x != x` to false, so a NaN guard never fired | precision | an instrumented run |
| naga's f32 literals past 2^63 broke Safari | platform | Safari testing |
| The device's exp-minus-one and van der Pol maps were off by an f32 unit, the same sign every stage (5556315) | precision | the long-run device check |

Not one is a wrong physical model. One was found by a physical oracle, the
wall's reflection against its own law, and that one was a trace-numbering
defect too. The rest are structure, bookkeeping, precision and platform.
A cavity-eigenmode fixture on a four-walled box passes while the U with
one-sided arms refuses to load, so the first work goes where the defects
are, and the physics fixtures follow with their budgets justified.

Two of the nine, the band fill and the pinned corners, are the same shape:
a wall configuration no test had assembled. That is the class the first
stage covers.

## 2. Rules

1. **Four layers, reported separately.** Implementation (the device against
   the f64 reference on the same discretization), discretization (the scheme
   against an analytical or manufactured solution), physical approximation
   (the implemented law against exact DtN, Fresnel or the constitutive
   relation), application (the app's accepted state, generation identity and
   event timeline). A device-versus-f64 pass is never a physical result, and
   a passive approximate wall is never required to converge to exact
   transparency.
2. **A boundary-law fixture passes when it converges to the law it claims to
   implement.** Its distance from exact DtN is reported, never enforced:
   `E_discretization = |R_numeric - R_implemented|` must go to zero,
   `E_approximation = |R_implemented - R_exact|` generally does not.
3. **The oracle is independent, the fixture runs the production path.** The
   reference never reuses the assembly, the integrator, the boundary
   approximation or the transfer; the fixture always goes through meshing,
   assembly, the canonical compile and, where it applies, the device.
4. **Tests consume what production produces.** A solver under test takes the
   assembly's own output, explicit zeros and all; a dense copy, a hand-built
   matrix or a reshaped signature is a second test, never the only one.
5. **Three levels wherever convergence is claimed**, spatial with the
   temporal error negligible, temporal against the semidiscrete reference,
   and the expected order enforced only in its demonstrated asymptotic range;
   an f32 plateau is reported, not fitted.
6. **A budget per fixture**, derived from its f64 convergence study, its
   reference accuracy and its f32 floor. The established 5e-6 device-versus-f64
   bound stays for the short fixtures.
7. **References are computed offline** and checked in as tables with their
   generator and the identities that validate them. The test runner is cargo;
   CI runs no Python beyond the browser server.
8. **Every category carries at least one mutation** that its fixture must
   fail, run as a scripted check outside the gate. A test that cannot fail is
   not a test; the oracle test of the banded solve was one until this
   morning.
9. **The frozen fixtures are never regenerated.** A scene that failed in use
   is added as it was saved, and a schema change keeps it opening by a
   default or a migration under a new version.

## 3. Stage A0: structure

Target: two weeks. Every item is a plain cargo test or script; nothing here
needs a special function.

### A0.1 Configuration sweeps at the preparation layer

A generator over the 45 gallery scenes, `Scene::initial` and the U-and-line
scene of 12 October takes each scene and varies its walls:

- every outer side set to each `OuterBoundaryCondition` in turn, and all four
  at once;
- every curve span's left and right set to each `FaceBoundaryCondition`
  (reflecting, impedance 0.5, second-order outgoing, electric wall, magnetic
  wall, a zero and a driven Neumann, a zero and a driven Dirichlet), one side
  at a time with the other reflecting, then both;
- every span transmitting and separated;
- every pair of spans whose ends lie within one edge length of each other,
  or of the outer wall, both outgoing: the configuration of 12 October.

Each configuration is prepared the way the app prepares it, through
`TopologyRuntime::request` at a target edge of 0.2 sized for the domain's
area, and the unmodified scene once more at its own edge. The generator
sorts every case into one of three classes before it runs, and the test
asserts the class, never "succeeds or refuses":

- **must succeed:** every combination the editor's own validity rules
  accept, `FaceBoundaryCondition::valid`, `SpanBehavior::valid` and the
  document's structural checks; it must mesh, assemble and compile, and the
  count of successes must equal the count of such cases, so a solver that
  politely refuses everything cannot pass;
- **must refuse, with the stated reason:** every combination those rules
  reject, such as a thin gap beside a side that is not reflecting; the
  preparation must stop with the refusal the rule names, and nowhere later;
- **exploratory:** combinations the editor cannot author, which only the
  core-level twin reaches; a refusal there is recorded with its reason and
  listed in the report, not counted as a pass.

In every class a panic is a failure. For every case that prepares:

- the outgoing trace, where there is one, has its pieces counted, is
  positive semidefinite, and its banded solve agrees with the dense oracle
  to 1e-12 of the norm in values and 1e-13 in residual;
- a second assembly of the same configuration gives the same signature and
  the same mesh hash, so the sweep doubles as a determinism check.

The GPU plan is packed on the host by A0.2's regression test rather than by
the sweep, once per scene and both ways (A0.3).

Coverage is stated, not sampled. The mandatory set per scene is: each outer
law on each side alone and on all four at once; each face law on each
span's left and on its right with the other side reflecting, and on both
sides together; each span transmitting and separated; and every adjacent
pair, two span ends within an edge length of each other or of the outer
wall, both second-order outgoing. For a scene of five spans with nine face
laws and four outer laws that is about 190 cases, and the gallery's 45
scenes give roughly eight thousand preparations. A supplementary sample of
mixed cases, two spans under different laws or a face law beside a changed
outer wall, is drawn with a fixed seed and capped at forty per scene. The
test reports, per scene and per class, what it covered and what it omitted,
and the omissions are only ever from the supplementary sample.

The run time decides what the gate carries: the mandatory set at edge 0.2
is measured at implementation, and if the whole gallery exceeds about three
minutes in release the gate runs a fixed subset of scenes chosen to cover
every geometry feature, open and closed curves, junctions, obstacles beside
the wall, several regions, and the nightly runs all 45 with the
supplementary sample. A core-level twin walks the `Scene` path with
obstacles and internal boundaries, which the gallery does not author.

Done when the sweep runs in the gate with its coverage report, its classes
and selection are documented in the test, and a sweep with the band fill of
1bc6d82 reinstated fails on the U-and-line scene, which is the first
mutation of A0.4.

**Done on 2026-10-12** (`docs/engineering-log.md`): 40 scenes, 12,042
mandatory and 1,600 supplementary cases, 113 s on ten threads at the Coarse
preset, the five arrays of one obstacle in the nightly at 0.16 and 0.2; the
refusals at a corner and at a shared node predicted from the draft and from
the compiled plan rather than from the assembly; the adjacency reduced to
one pair of spans per pair of curves, which an array would otherwise turn
into thousands. The first run found 36 configurations the editor accepts and
the mesher cannot build, every one a span changed between transmitting and
separated, carried in the sweep's `KNOWN` table until the mesher fix the
same day emptied it (the log), and a Dirichlet refusal the assembly made
after acceptance, which validation makes since the same day; both are
ticked under Maintenance in `docs/plan.md`. The core twin fails with the
band fill of 1bc6d82 reinstated.

### A0.2 Regression scenes that prepare

`crates/funfern-app/tests/fixtures/scenes-regressions/` beside `scenes-v22`,
with a README in the same terms: never edited, never regenerated, one file
per scene that failed in use, named by date and defect. The first entry is
the autosave of 12 October with the one-sided arms. `tests/regression_scenes.rs`
opens each, prepares it through the runtime at the document's own mesh edge,
and asserts success and the trace invariants of A0.1. The same test prepares
the 45 frozen version-22 scenes and the gallery; today they are only opened.

Done when every reported bug with a scene adds its file in the fix's commit;
the rule goes into the gate memory and the README.

**Done on 2026-10-12** (`docs/engineering-log.md`): the folder with the
autosave of 12 October as its first file, `tests/regression_scenes.rs` over
it, the 45 frozen scenes and the gallery at their own edge, 91 scenes in a
second, and the rule in the README and in memory.

### A0.3 Audit of what the tests feed the solvers

For each solver with an oracle test, the trace eigensolve, the transfer and
handoff maps, the far-field recorder, the probe estimators, the GPU plan
packer and the material-law tables, read the test's input path and answer
one question: is this what the assembly hands over? The band fill's oracle
was not; a dense matrix has no explicit zeros. Each finding is either fixed
in place or justified in a comment, and the list goes into the log entry.

Done when the list is in the log and every item on it is closed.

**Done on 2026-10-12** (`docs/engineering-log.md`): eight verdicts; the
packer, the component-total correction and the outgoing boundary's reuse
and refusals closed with production-shaped twins; the far-field recorder
and the probe estimators deferred to PH-18 by name; the indicator's
auxiliary alignment found dormant with a slip in it, listed for removal.

### A0.4 Mutations

`scripts/mutations.sh` applies each patch under `tests/mutations/`, runs the
test the patch names, and reverts; the gate does not run it, the nightly
does, and so does anyone touching the fixture it guards. A mutation counts
as detected only when the mutated tree builds, the named test runs to the
check the mutation targets, and fails there for the intended reason: each
patch comes with a manifest naming the test and a pattern the failure
output must match, the assertion's own message. A compile error, a panic
elsewhere, a missing device or a timeout is "not detected" and fails the
mutation run, and so does the unmutated test not passing first. The first
six, each naming the fixture class that must catch it:

| Mutation | Must fail |
|---|---|
| Reinstate the band fill without the owner check | `zeros_between_pieces_leave_the_solve_unchanged`, the oracle test's U-and-line scene, A0.1 |
| Flip the sign of the second-order wall's flux | the wall's reflection against its law (`canonical_temporal`) |
| Drop the outgoing wall's auxiliary update | the wall's energy balance |
| Reset the physical clock on handoff | the `handoff` fixture and `canonical_gpu_temporal_handoff` |
| Apply a material switch twice | the `switch` fixture and the protocol proofs |
| Drop the transfer's component-total correction | the `remesh` fixture |

Done when all six fail as named on Linux in the nightly.

**Done on 2026-10-12** (`docs/engineering-log.md`): `scripts/mutations.sh`
with the four-step detection rule, `scripts/e2e-fixture.sh` for the device
ones on either platform, the six mutations under `tests/mutations/`, each
detected as its manifest names, and the nightly `mutations` job. Two of the
six catch at an earlier check than first intended, and the manifests name
the check that fires.

### A0.5 The device suite keeps its own binary

`scripts/device-suite.sh` copies the e2e build of `funfern-app` into its log
directory right after building and runs the fixtures from there, so a plain
build during the GPU runs cannot replace it; this morning one did, and the
fixtures sat in the first-run tour for 900 s each.

**Done on 2026-10-12** (`docs/engineering-log.md`): every binary the suite
runs is hard-linked into its log directory and run from there, the linked
app checked for its driver first; the fixture wrapper links its binary the
same way. Stage A0 is complete. The driver's place in the shipped binary is
the hazard's root, and moving the fixtures out of it follows as its own
item: the `funfern-fixtures` binary took the native consumers and then the
browser bundles the same day, and the feature went.

## 4. Stage A1: the architecture's own physics

These are the two fixtures that test what is unusual here, independent Q
and b and boundary memory through a transactional handoff, and neither needs
a reference beyond arithmetic.

**PH-03, independent complementary modes.** Build a state with Q zero and b
in the discrete kernel of the canonical coupling, a nonzero force-free flux,
on a reflecting box and on a box with a separated baffle. Step it in f64
and on the device. On the fixed mesh the demand is exact: Q stays at
roundoff, b and the complementary energy are preserved, nothing crosses the
baffle. Across a neutral remesh the demand is not invariance, since a
vector in the old discrete kernel does not land in the new one: the test
separates the physical component, measured by an independent integral of
the transferred flux and required within the transfer's discretization
budget, from the transfer error itself, and from the coupling residual the
transferred b has in the new space, which is bounded and whose leakage into
Q over the following steps stays within that same budget. The component
must stay on the side of the baffle it was on. Add a device suite entry
and an e2e fixture.

**PH-10, boundary memory through handoff.** Run a pulse into a second-order
wall until its auxiliaries are nonzero, then perform a no-change rebuild, a
remesh of the same geometry, a material change that needs a new generation,
and a timestep change. Each has its own reference, and only the first is
the uninterrupted run:

- the no-change rebuild is an identity transfer, compared with the
  uninterrupted control directly, coefficient by coefficient;
- the remesh changes the boundary basis, so the histories are compared in
  their physical representation, the wall's trace field, its stored energy
  and the waveform that comes back from it afterwards, within the transfer
  budget, never auxiliary by auxiliary;
- the material change is compared with an f64 reference that applies the
  same physical change at the same accepted step, as the `switch` and
  `handoff` fixtures already do; agreement with the unchanged control is
  not the expected outcome;
- the timestep change is compared with the reference re-stepped at the new
  timestep from the same accepted state.

In every case Q, b, the physical time and the stored boundary energy are
read, and the time origin must survive: `t = t_origin + N Δt` per
generation, never a phase reset. An e2e fixture, `handoff-memory`, beside
`handoff`.

**Done on 2026-10-13** (`docs/engineering-log.md`): five fixtures,
`handoff-memory-rebuild`, `-remesh`, `-edit`, `-timestep` and `-running`, on
the cavity with a second-order wall and a driven side, the carry read at the
handoff itself and the run read at the endpoint, the clock on every fixture,
`CanonicalWaveState` given the time origin the reference needs, and the
mutation `drive-phase-reset-on-handoff`, which only the running fixture
sees: a held run has no time between the candidate's preparation and its
arrival, and a carried drive takes its phase from the old clock. Found and
reported: the remesh's carry raises the wall's stored energy by a fifth,
2.3e-2 of the memory outside the new basis; the fixture reports both, and
the bound on the energy's change waits on the diagnosis.

## 5. Stage A2: bulk propagation and planar walls

**PH-01, rectangular cavity modes.** Neumann box, modes (1,0), (0,1), (1,2),
(2,3), a superposition and the constant, in Mechanical, TM and TE, the
initial Q and b from the closed-form cosines and their compatible
complementary field. Three mesh levels at a timestep small enough that the
temporal error is negligible, then three timesteps on one mesh against the
semidiscrete reference, which is the assembled operator itself and so an
implementation-layer measurement by rule 1. Metrics: frequency and phase
over several periods, amplitude drift, weighted L2 and scaled L∞ field
error, and the discrete energy's bounded oscillation. The fourth-order step's
claimed temporal phase order is checked here. The references are closed
forms computed in the test.

**PH-04, planar reflection against the implemented law.** Promote the
Bloch-strip methodology of `docs/spikes/funfern-boundary-scattering-spike.md`
and the `wave_boundary_reflection` example into a three-level test: angles
0°, 45° and 75°, first-order and second-order walls, propagating modes fitted
to incoming and reflected components for the complex `R`. The implemented
law's own prediction comes from `experiments/material-laws-spike/boundary.py`
as a checked-in table with its generator; `R_exact` from the half-space
relation is reported beside it. `E_discretization` must converge;
`E_approximation` is a number in the report.

## 6. Milestones B and C

The remaining fixtures of the 11 October plan keep their identifiers and
their content, under three rules: one parameterized Rust test per fixture at
three levels, references offline, sweeps as scripts under `experiments/`
with their reports in `docs/spikes/`.

- **B, physical subsystems:** PH-05 wall passivity across material laws,
  PH-06 and PH-07 the circular outgoing wall and the absorbing hole, pass/fail
  on `E_discretization` only, with `E_approximation` against the Hankel
  relations reported, PH-08 rectangular radiation as a one-off experiment
  rather than a regression test, since no nonlocal operator is planned,
  PH-09 transient DtN against the Laplace-domain kernel, PH-11 Fresnel,
  PH-12 cylinder scattering, PH-13 thin gaps, PH-14 and PH-15 temporal and
  modulated materials, PH-16 nonlinear laws, PH-17 energy accounting, PH-18
  probes and observables, PH-19 manufactured solutions.
- **C, invariants and stress:** IV-01 to IV-13, AMR-01 to AMR-04, RT-01 to
  RT-08, the numerical edge cases, long runs and resource lifecycle. The
  metamorphic tests and the real-device fuzzing need none of the reference
  machinery and can start whenever a stage leaves room.

## 7. Infrastructure

- **References.** `tests/physics/references/` holds the tables as JSON, the
  Python that generated each, and a check script that recomputes the
  identities the plan's section 6.4 asks for, precision refinement and
  limiting cases, run by hand when a table changes. A test reads its table.
- **Runner and CI.** The gate stays one command. Every pull request runs the
  workspace tests, the Kani proofs, the device fixtures on software Vulkan
  and the SwiftShader cavity; the compact physics subset joins the workspace
  tests as they land. The nightly runs the full SwiftShader set, the
  mutations and the long sweeps. The device suite on Metal stays a local run
  before any commit that touches shaders, the device path or the outgoing
  basis.
- **Failure artifacts.** A failed fixture writes the authored scene, the
  accepted mesh hash, the event timeline, the generation and step history,
  the reference and measured observables and the violated bound under
  `target/`, as the end-to-end fixtures already print their build and
  endpoint hashes. A randomized test keeps its minimized sequence, not its
  seed.
- **Budgets.** A new fixture begins with an f64 convergence study and its
  budget is written into the test with the study's numbers; the device and
  browser bounds follow from the measured f32 floor.

## 8. Definition of done

- **A0:** the sweep and the regression scenes run in the gate; the audit's
  list is closed; the six mutations fail as named; the suite keeps its
  binary.
- **A1:** independent b is exact on the fixed mesh and within the transfer
  budget across a remesh, physical component, transfer error and coupling
  residual reported apart; boundary memory and the time origin survive four
  kinds of handoff, each against its own reference.
- **A2:** spatial and temporal convergence of the cavity modes demonstrated
  on the device in three physics models; planar reflection converges to the
  implemented law at three angles with the DtN gap reported.
- **B and C:** as the 11 October plan states per fixture, under the three
  rules of section 6.

## 9. First step

A0.1, the configuration sweep, presented as an engagement plan before any
edit: the generator's selection rule, where the sweep lives, its assertions
and its run time at edge 0.2, with the band fill of 1bc6d82 as its first
mutation.
