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
it keeps that plan's layering, its oracle rule and most of its fixtures. That
plan is kept as written in `funfern-testing-plan-2026-10-11.md`.

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
   CI runs no Python beyond the browser server. A reference that is a closed
   form, a few lines of arithmetic in the test's own terms, lives in the test
   instead, written from the physics and never from the solver's code
   (amended on 2026-10-13: PH-01's modes and PH-04's implemented law are
   closed forms).
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

**Done on 2026-10-13** (`docs/engineering-log.md`): the device example
`canonical_gpu_complementary_mode`, on the box and on the baffled box, on
the fixed mesh and across the carry, four suite entries. The budget is the
carry's own discretization error, measured and required to fall with the
mesh. The e2e fixture is not built: the driver acts through the app's
handlers alone, and the app has no tool that authors a complementary flux;
a fixture would need a driver-only install, which would make the driver
something other than a user.

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
arrival, and a carried drive takes its phase from the old clock. The running
fixture found a race in the runtime transfer shader on lavapipe the day it
ran there, fixed the same day. Found and reported: the remesh's carry raised
the wall's stored energy by a fifth, 2.3e-2 of the memory outside the new
basis, the carried pole currents being nodal values that hold the square
root of each node's length; fixed the same day, the currents carried as the
flux each pole puts through the wall per unit length, and the fixtures hold
the energy's change to 1e-2.

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

**CPU half done on 2026-10-13** (`docs/engineering-log.md`): one test per
physics model on a reflecting 2 × 1.4 box, so that no mode PH-01 runs is
degenerate with another as on the square, the modes started at a third of a
period with b from its closed form and each model's orientation, and the
mutation `tm-orientation-flipped`. **Device half done the same day**: the
example `canonical_gpu_cavity_modes`, one suite entry per physics model; the
device follows the f64 frequency to its f32 floor of 3e-8, which lies under
every mesh's error, so its spatial convergence shows to the finest mesh.

**PH-02, plane-wave propagation and dispersion.** Left out of this plan when
it was written, though the 11 October plan gives it P0; added on
2026-10-14. Analytic plane waves, with prescribed boundaries or manufactured
forcing where needed, swept over the propagation direction, the wavelength
against the mesh size, constant isotropic and diagonal anisotropic media,
the mesh's orientation and its elements' aspect ratio, and the step against
its stability limit. Measured: the numerical dispersion relation, the group
velocity, the field's reconstruction in each polarization and the phase
error, so that the element's error and the integrator's are told apart.
Passes when they converge to the physical dispersion relation as the spatial
and temporal errors are predicted to. Its design is planned on its own.

**Done on 2026-10-14** (`docs/engineering-log.md`): without periodic sides a
lone plane wave cannot run on the production path, so a reflecting box's
modes, each the sum of four plane waves, carry the wavevectors: seven
directions from 0° to 90° on a 2 × 1.06 box whose modes keep 2.5% apart in
frequency. The mesh's orientation and its elements' aspect ratio are not
built as structured meshes, which the application never makes: the mesher's
triangles have no orientation, which the directions cover, and a diagonal
anisotropic medium of axis ratio `r` on the box stretched to `2√r × 1.06/√r`
is, by the elements' affine invariance, the isotropic medium on the
mesher's elements stretched `r` times, so ratios 2 and 4 read elements of
aspect 2 and 4 on meshes the application builds. The field's reconstruction
in each polarization is PH-01's complementary flux in TM and TE, held in
those tests. The mutation `lumping-midpoints-to-vertices`. With PH-02, stage
A2 is done.

**PH-04, planar reflection against the implemented law.** Promote the
Bloch-strip methodology of `docs/spikes/funfern-boundary-scattering-spike.md`
and the `wave_boundary_reflection` example into a three-level test: angles
0°, 45° and 75°, first-order and second-order walls, propagating modes fitted
to incoming and reflected components for the complex `R`. The implemented
law's own prediction comes from `experiments/material-laws-spike/boundary.py`
as a checked-in table with its generator; `R_exact` from the half-space
relation is reported beside it. `E_discretization` must converge;
`E_approximation` is a number in the report.

**Done on 2026-10-14** (`docs/engineering-log.md`): the solver has no
periodic sides, so the Bloch strip cannot run on the production path, and a
reflecting channel's transverse mode meets the wall at the angle instead. A
narrowband pulse served 0° and 45° and failed at 75°, 3.5% above its mode's
cutoff, so the measurement is a steady state: a sponge in the damping, read
twice a quarter wavelength apart to cancel its own reflection, and a
lock-in. The law is a closed form in the test under the amended rule 7, not
a table from `boundary.py`. The gate holds 0° and 45° on the coarsest mesh;
the nightly job `planar-reflection` runs the three angles on three meshes and
the 45° time series; the mutation `wall-memory-half-kick`.

## 5a. Stage A3: circular walls

**PH-06 and PH-07, the circular outgoing wall and the absorbing hole.**
The 11 October plan gives both P0 and puts them in its milestone A; this plan
had moved them into B, and on 2026-10-14 they came back to A ahead of the
rest of B. An annulus whose outer circle is the wall, or a hole in a square
whose own sides absorb, driven to steady state at one angular order from a
source ring, each order's radial phasor fitted to `A H + B H′` with the
Hankel functions from a checked-in table, and `B/A` read against the
implemented law on a circle; `|B/A_law|`, the law's distance from the exact
exterior DtN, reported beside it.

**Done on 2026-10-15** (`docs/engineering-log.md`): its sizing found a
defect, second-order walls on curves assembled with each edge's nodes out of
order (fixed in d1316a9), and taught the measurement four things: the
circle's shape is meshed at a curve tolerance of 5e-5 or it sets a floor
the mesh cannot lower; one angular order a run, or the strong orders leak
into the weak; the hole's square absorbs, or it rings; and the circle's
small boundary elements set the time step, so each reading is extrapolated
from the step and its half. The gate test reads order 2 at kR 5; the nightly
jobs `outgoing-circle` and `absorbing-hole` run the thirteen propagating
cases of kR 0.8 to 10 under both laws, kR 5 and 10 on three meshes. The
mutation `curve-wall-stiffness-doubled`.

## 6. Milestones B and C

The remaining fixtures of the 11 October plan keep their identifiers and
their content (`funfern-testing-plan-2026-10-11.md`), under three rules: one parameterized Rust test per fixture at
three levels, references offline, sweeps as scripts under `experiments/`
with their reports in `docs/spikes/`.

- **B, physical subsystems:** PH-05 wall passivity across material laws
  and PH-17 energy accounting, together as stage B1 (section 6a)
  (PH-06 and PH-07 moved to stage A3), PH-08 rectangular radiation as a one-off experiment
  rather than a regression test, since no nonlocal operator is planned
  (done on 2026-10-17, `funfern-rectangular-radiation.md`),
  PH-09 the walls' transient response (section 6b), PH-11 Fresnel (done
  on 2026-10-17, section 6c),
  PH-12 cylinder scattering (done on 2026-10-17, section 6d), PH-13 thin
  gaps (done on 2026-10-17, section 6e), PH-14 temporal interfaces (done on
  2026-10-18, section 6f), PH-15 modulated materials (done on 2026-10-18,
  section 6g), PH-16 nonlinear laws (done on 2026-10-18, section 6h), PH-18
  probes and observables (section 6i), with
  PH-12's far field, PH-19 manufactured solutions.
- **C, invariants and stress:** IV-01 to IV-13, AMR-01 to AMR-04, RT-01 to
  RT-08, the numerical edge cases, long runs and resource lifecycle. The
  metamorphic tests and the real-device fuzzing need none of the reference
  machinery and can start whenever a stage leaves room. The P1 invariants,
  IV-01 to IV-05 and IV-08 to IV-12, were planned on 2026-10-31
  (section 6j), the P2 invariants, IV-06, IV-07 and IV-13, on 2026-11-02
  (section 6k).

## 6a. Stage B1: energy accounting and wall passivity

**PH-17 and PH-05**, planned together on 2026-10-15. The solver's ledger
was already checked against itself - its lanes add up to the energy's change
at second order - which cannot see a lane that is wrong in step with the
energy, a loss applied and booked twice. Both fixtures hold the ledger to an
oracle instead: each channel's power written in the test from the authored
law and integrated over the run's own states, lane by lane, at three steps.
PH-05 adds the walls: the test assembles the trace's damping and tangential
stiffness itself, diagonalizes them with its own solver and integrates the
second-order law's three-state equations from
`docs/spikes/funfern-boundary-auxiliary-spike.md` on the solver's trace
field, for the power into the wall, its stored energy and its dissipation,
beside every medium the wall can meet and near the law's awkward
frequencies. The short-wave loss stays out of the oracle. A remesh's own
energy change, which the device books and the CPU did not, comes through a
CPU handoff built for it (cf9ec40). Then a device half and the long runs.

**PH-17 done on 2026-10-15** (`docs/engineering-log.md`): thirteen
configurations in each skin, the matched loss's closed-form decay, the
conservative rows and the remesh row, in the gate. It found a pumped
complementary row with its own loss booking its pump work at first order
(fixed in 3d561a9); its mutations `loss-lanes-crossed`,
`modulation-work-omitted` and `edit-exchange-dropped`.

**PH-05 done on 2026-10-15**: the wall oracle beside every medium in each
skin and a slow near field under the law's pole at zero, in the gate. It
found a source on a wall booking its work at first order, on both paths and
the device (fixed in 204f355); its mutation `wall-loss-without-memory`.

**Device half and long runs done on 2026-10-15.** The device's lanes were
already held to the reference's by the device examples, all but one: the
edit lane, which nothing read. The three handoff examples now hold it to the
energy each handoff moves, each side valued by its own generation; on a
time-driven generation it booked the stores neither generation holds, and
nothing of the restoring potential (fixed in da83bc0, mutation
`handoff-store-authored`). The nightly job `energy-long-runs` holds the
conservative media's energy to its band over a thousand seconds, the passive
media's and the walls' envelope to never rising over four hundred, the slow
near field's ledger over twenty periods, and a Kerr medium's known drift
under the splitting to where it stands. With these, stage B1 is done.

**Time-driven CPU handoff done on 2026-10-16.** B1's CPU handoff stayed
linear while the device's time-driven valuation was in doubt; with that
fixed it followed. The runtime rule moved into core, the device's transfer
table built from it (aaab74e); the CPU hands driven states over, and between
fixed and driven generations either way (951b425), carrying `r` on the
primary map's rows as the device does (9344c76); the time-driven handoff
examples hold the device to it (1052cce); and PH-17's handoff rows run on
every path (d5c163c). Those rows found PH-17's oracle valuing a pump at half
of `Q u`, which holds on a linear row only; on Kerr it is the law's own
`Φ(u)`, and a pump on Kerr joined PH-17's configurations. The mutations
`carrier-restarted-on-handoff` and `driven-edit-valued-authored`.

## 6b. PH-09: the walls' transient response

**Planned on 2026-10-15.** The 11 October plan reads the walls under pulses
against the exact kernel's causal response. The walls implement a rational
law whose distance from the exact DtN PH-04 and PH-06 already report, so
PH-09 reads each wall's transient against its own law at every frequency
its pulse carries, where PH-04 and PH-06 read one: what is under test is the
history, the poles' states the law keeps between steps. Two geometries.
PH-04's channel, where a mode's pulse meets the wall at every angle its band
spans and the reflecting and pinned walls give the incident field and a
unit reflection. And the circle, its curved trace the more bug-prone: a disk
whose circle is the wall, rung from a ring, where two walls' records stand
in a ratio of Bessel functions read from the dense Hankel table (bf01fbe),
whatever the ring and however long the disk rings.

**Done on 2026-10-15** (`docs/engineering-log.md`): broadband, separated,
narrow and near-cutoff pulses in the channel, broadband, low, narrow and two
orders at once on the circle. The channel converges at second order or
better; the circle stops at the drawn polygon's floor, which no mesh
lowers, and is held there. In the gate, separated pulses in the channel and
two orders on the circle; the nightly job `transient-walls` runs the rest.
The mutation `wall-memory-leaks`, the second-order wall's zero pole
leaking, is caught by both gate tests. No defect found.

## 6c. PH-11: planar interfaces

**Done on 2026-10-17** (`docs/engineering-log.md`). A channel's mode meets
planar interfaces at an exact angle in every layer; the field either side is
fitted to a forward and a backward wave, and the four amplitudes held to the
stack's exact scattering matrix, written from the physical fields in each
skin, so that what the ends return is one more incident wave. Normal and
oblique incidence, a matched layer, contrast 25, total internal reflection,
a slab and an anisotropic layer, in Mechanical, TM and TE; errors fall about
fourth order to 1e-5 to 4e-4 on edge 0.05. The gate holds eight cases on
edge 0.1; the nightly job `fresnel-interfaces` the rest on three meshes. The
mutation `frame-rotation-dropped`. No defect found; the sizing taught the
measurement its spacing, its timing beside a cutoff and its frames.

## 6d. PH-12: cylinder scattering

**Done on 2026-10-17** (`docs/engineering-log.md`). A cylinder in a box of
second-order walls, lit by a band source; the field on five circles about it
is split into orders and each fitted to a regular and an outgoing Bessel
wave, so that what the walls return is one more regular field, and the
outgoing over the regular held to the partial-wave ratio from an mpmath
table, with each order's balance, the optical theorem order by order.
Reflecting holes, pinned cylinders and inclusions of index `√2` to `√10`,
`k a` from 0.5 to 5, in Mechanical, TM and TE, three off centre. The mesh's
rim is a polygon inscribed in the circle, which scatters as the circle of
its area, 2e-4 short in radius at the default curve tolerance; the solver is
read against that circle's ratios, through their tabulated slope, and the
geometry against the true circle's. The solver's error falls 2.4 to 3 times
from edge 0.07 to 0.05, to 1.2e-5 at `k a = 2` and 2e-4 at 5; the geometry's
is 8e-4 at the default tolerance and 2e-4 at PH-06's. The gate holds six
cases on edge 0.1; the nightly job `cylinder-scattering` every case on
three meshes and two tolerances. The mutation `curve-sampling-coarsened`.
No defect found; the far field moves to PH-18, as agreed.

## 6e. PH-13: thin gaps

**Done on 2026-10-17** (`docs/engineering-log.md`). PH-11's channel with a
gap at its interface: a spring coupling two reflecting faces, carried into
the transfer matrix as the flux equal to the spring's tension, so that the
scattering matrix holds `R` and `T` as for PH-11, and the gap's own memory -
the jump it stores - and its springs' energy are held to the exact jump.
Weak, half and stiff coupling, oblique incidence, the cut as the limit, a
resonant pair and one off it, and two-sided gaps between different media
in each skin drawn either way. A half gap remeshed with its memory carried
by the handoff, read right after it and again on the new mesh. The memory
carries the production step's bounded second-order offset, 7e-4 of the
stored energy on edge 0.1, which falls with the step while `R` and `T`
stand. The gate holds three cases and the handoff; the nightly job
`thin-gap-interfaces` every case on three meshes and at half the step. The
mutation `gap-history-dropped-on-handoff`. A dissipative gap is not
implemented (`architecture.md`); the impedance faces it would stand for
are PH-04's, PH-06's and PH-09's. No defect found.

## 6f. PH-14: temporal interfaces

**Done on 2026-10-18** (`docs/engineering-log.md`). A uniform medium's mode
evolves alone by `d/dt(m β̇) = −k² s β`, so a box started in one mode is held
through every change to that ODE, integrated in the test from the laws'
definitions; an instant change keeps `B` and `D`, the ratio of the
time-reflected to the transmitted part meets the temporal Fresnel one, and a
slow ramp keeps `E/ω`. Switches on either slot, the impedance kept, both
slots at once, ramps from a tenth of a period to four, slabs and a train of
them, and a gate's slab, in each skin, each at two phases a quarter period
apart, which fix the time-reflection ratio. The field, the stored energy,
the modulation's work and the frequency after the change agree to the
step's second-order offset, 1e-3 at the production step and four times
less at each halving. The gate holds every case; the nightly job
`temporal-interfaces` half and a quarter of the step and a finer mesh. The
mutation `switch-ramp-work-dropped`. No defect found; the oracle first read
Mechanical's stiffness laws as acting on `k`, which they divide, acting on
`s₀ = 1/k₀` (`architecture.md`). Periodic modulation is PH-15's.

## 6g. PH-15: modulated media

**Done on 2026-10-18** (`docs/engineering-log.md`). PH-14's mode equation
with a periodic coefficient is a Hill equation; its monodromy over a
modulation period gives Floquet's multipliers, growth and frequency, and
the box mode under a pump is held to them and to the ODE itself: the trace,
the growth exponent, the energy supplied, the main line and the sidebands.
Mass-side, stiffness-side and both-slot pumps, which keep the impedance and
never grow, parametric amplification at twice the mode's frequency, the
first tongue's edge from inside and outside, the second tongue - closed for
the plain slots, open for a divided stiffness slot - and time crystals, in
each skin. Growth meets Floquet's to 1e-7 of the mode's frequency on the
mass side; the stiffness side carries five times the step's second-order
error. The gate holds every case; the nightly job `modulated-media` half and
a quarter of the step and a finer mesh. The mutation `pump-carrier-turned`.
No defect found. Travelling modulation, not homogeneous, stays with the
gallery's directional test.

## 6h. PH-16: nonlinear laws

**Done on 2026-10-18** (`docs/engineering-log.md`). The constitutive inverse
against an independent root from the smallest subnormal to a declared bound,
with its stored energy and its refusals; it found the f64 inverse refusing
subnormal targets, fixed the same day. A uniform field makes every node one
oscillator, held to RK4 and the closed-form periods at second order in the
step: sine-Gordon from 1e-3 rad to beside the separatrix and rotating, φ⁴ in
a well and over the barrier, a saturable mass and a lossy Kerr mass beside
Klein-Gordon, and van der Pol's cycle from both sides. Klein-Gordon's cutoff
adds exactly to a box mode. On the CPU the step that carries a field past
its bound is refused, the one in which the ODE crosses it. All in the gate,
no nightly; the mutation `sine-gordon-force-linearized`. Saturable
absorption stays gated (C), so its bleaching is not tested. The planned
3.1 rad parts the nodes, the uniform swing beside the separatrix being
unstable to the box's modes, so that case swings to 2.9. CPU against GPU
through the nonlinear stages stays with the device suite's fixtures.

## 6i. PH-18: observables

**Planned on 2026-10-18.** Every probe number on screen comes from a GPU
recorder, and no recorder was held to an analytic field; the far-field
recorder had run only on dummy stencils. Four parts, each its own commit:
1. The CPU consumers, the recorders' twins, against a standing mode and a
   travelling packet in each skin.
2. An end-to-end fixture `observables`: the shipped app on a channel with a
   launcher and a matched wall, with two points, a segment, a boundary probe
   on the wall and a disk. Its host traces are held against the travelling
   wave, the f64 twin and the energy ledger, with the readouts' spectrum and
   Welch transfer, the vector overlay, and probes on against off.
3. The far field: the frequency-domain twin against a lone source, an array
   and a source beside PH-12's cylinders, then an f64 twin of the shader's
   time-domain projection, and a `far-field` fixture running the recorder on
   a real contour. The pattern stays relative.
4. Timestamps: the rate's half-step lag and the recorders' single-f32 clock,
   measured and reported.

**Part 1 done on 2026-10-18:** the consumers read both fields in every skin
at second order, with each skin's units, the flow along the travel and a
segment's flux under its left normal. The mutation
`consumer-flow-orientation-dropped`.

**Part 2 done on 2026-10-18:** the fixture `observables` holds every trace to
its twin on Metal, lavapipe and Chrome. It also checks coverage, stamps,
Poynting round a loop, the readouts' spectrum and transfer, and the probes
off against on. Boundary probes take a drawn curve, not a side of the
domain, so the boundary recorder reads a transmitting loop and a segment
reads the wall. The nightly's browser run has `observables` as a part of
its own, the recorders' part. The mutation `curve-probe-flux-reversed`. The transverse field on
an outgoing wall's elements is listed as worth checking.

**Part 3 done on 2026-10-18:** the far field. The recorder's twin read a
line source's pattern 9.9% off flat. Its contour frames paired the gradient
with a rate half a step older, and centring them on their step fixed it, in
the shader and the twin. The twin now meets a line source's
pattern alone and beside PH-12's cylinders within 1.1e-2, in the gate and
nightly. The fixture `far-field` holds the device's records to it, and the
browser nightly's recorders part runs it. The mutation
`far-field-frame-at-step-end`.

**Part 4 on 2026-10-18:** the stamps.
- **The point rate's half step:** documented, not changed.
- **The recorders' single-f32 stamps:** 1.2e-4 s after an hour, 2e-3 s after
  ten. The point, line and area records now carry two parts, which the host
  adds in f64, as the vector overlay does. The mutation
  `probe-stamps-summed-in-f32`.
- **The far field's frames,** which the device interpolates in time itself,
  carry two-part stamps too: every difference is taken part by part, and the
  ring is keyed to its own count.
- **Late-clock fixtures.** Fixtures started 36 hours in (`far-field-late`,
  `handoff-memory-running-late`) found two defects in the clock itself, both
  fixed:
  - fast math folding the rounding out of its compensated sums at every
    rebase and handoff;
  - handoff candidates packed against a clock at zero.

  The mutations `clock-origin-summed-in-f32`,
  `handoff-candidate-packed-at-zero` and `far-field-stamps-summed-in-f32`.

## 6j. Milestone C: the P1 invariants

**Planned on 2026-10-31.** Ten invariants of the 11 October plan's section
3, each a relation between two runs, so none needs a reference. What each
had and lacked:

| ID | Had | Lacked |
|---|---|---|
| IV-01 superposition | PH-01's four modes and a constant together, against the closed form | A, B and A+B, and αA with α < 0, over every linear channel |
| IV-02 identifiers | nothing | relabelled curves, spans, regions, materials and sources, and reordered material and region lists |
| IV-03 renumbering | renumbered traces in the transfer and plan tests | a shuffled mesh through assembly, compile and stepping |
| IV-04 symmetry | nothing | translated, rotated and reflected scenes in each skin |
| IV-05 observers | probes off, far field off and step batching, each the same bits | the overlays, auto exposure, readouts, recording, pause, resize |
| IV-08 separator | nothing | a transmitting separator between one medium, static and edited mid-run |
| IV-09 curves | reversal at the geometry level | reversed, restarted, reordered and split curves solved |
| IV-10 neutral remesh | totals to roundoff in the transfer maps; the device held to the f64 twin | totals counted from geometry per component, through the app, on both |
| IV-11 repeated remesh | twelve static round trips against fixed bounds | a running field remeshed K times, its error's growth in K |
| IV-12 baffle sides | each side its own values in the transfer tests | a jump across a free-ended baffle through refining, coarsening and a move |

One measurement settles which of them can hold to roundoff. A scene with a
hole, an inclusion and a baffle meshed at edge 0.08 gives the same mesh
with its curves' identifiers relabelled, and a different one, vertices up
to 0.043 apart, with its curves listed in another order, its baffle
reversed, a closed curve started at another breakpoint, or mirrored. The
same document always meshes alike, so this is no defect; it puts IV-04 and
IV-09 at the discretization level.

Four stages, a commit per invariant:

- **C1, exact identities on the CPU:** IV-03, IV-02, IV-01, each to
  roundoff.
- **C2, equivalent scenes at three levels:** IV-04, IV-09 and IV-08's
  static half, both fields read at common points, the difference required
  to fall with the mesh at the scheme's rate from an f64 study's numbers.
- **C3, the remesh family on the CPU and the device:** IV-10, IV-11,
  IV-12 and IV-08 edited mid-run.
- **C4, the device:** an `observers` fixture for IV-05, and IV-01 within
  f32.

**IV-03 done on 2026-10-31** (`crates/funfern-app/tests/invariants.rs`).
Every regression, frozen and gallery scene, 91 in all, is prepared at its
own edge, its mesh shuffled and prepared again through the path an
adaptation's mesh takes. Each run is struck by one pulse and runs 64 steps on
the CPU. The shuffled run matches the unshuffled one within 1e-12 in its
fields, energy, time step and probes, 8.9e-14 at worst. The unshuffled mesh
through that path matches a fresh preparation bit for bit. It found the
point locator taking a point on a shared edge from the first triangle in
the mesh's order: a segment probe along the parametric fiber amplifier's
axis, 13 of its 64 points on edges, read its complementary field 13%
differently on the same mesh renumbered. The locator now takes the
triangle whose centroid lies furthest along a fixed direction; its
mutation `probe-point-read-in-mesh-order`.

**IV-02 done on 2026-10-31**, in the same file. The same 91 scenes are
renamed: every curve, span, junction, material, region and probe gets a new
identifier, wide apart, and the materials, regions, face assignments,
volume sources and probes are listed backwards. The new identifiers zigzag
along each list as it then stands, so that no list is sorted by them. The
background keeps its identifier, which validation requires. 84 mesh and
run to the same bits. The other 7, the double slit, the Talbot carpet, the
Fresnel zone plate and the regression scene of 12 October, whose curves
meet the domain's sides or each other, mesh to the same triangles numbered
otherwise: the curves' identifiers order the mesh's numbering, which IV-03
shows the solution does not see, and they run within 8.5e-16. Its mutation
`materials-searched-as-sorted` finds a region's material by a binary search
of the library, which every scene the other tests build allows, their
libraries sorted by identifier.

**IV-01 done on 2026-10-31**, in the same file, and with it stage C1. Of
the 91 scenes, the 79 whose every material is linear (field laws linear,
loss rates constant, restoring law none or Klein-Gordon) run four times:
struck with their sources running, struck with every source's signal at
zero, their sources alone, and struck with every signal at -2.5 times. The
first is the sum of the next two within 2.8e-15 and the last -2.5 times
the first within 1.1e-14, through second-order walls, thin gaps, losses,
plasma, Klein-Gordon and modulated media; the 12 nonlinear scenes are left
out. Superposition sees a defect in how the state evolves; one in the
sources alone is in both runs it compares and cancels, which leaves it to
the scaling. Of two defects tried, neither was seen by IV-01 alone:
rounding the walls' memory through f32 on the CPU and the Klein-Gordon
force taken through the sine each have other tests too. Its mutation
`klein-gordon-force-through-sine`, which IV-02 and IV-03 pass.

**C2 planned on 2026-10-31**, on the user's go: IV-04, IV-09 and IV-08's
static half, each scene and its equivalent read at a grid of common points
through the production point probe, at three levels, a commit per invariant.

**IV-04 done on 2026-10-31**, in the same file. A motion carries a scene's
domain, curves, junctions, sides, face anchors, frames, source and probes,
and a reflection swaps each separated span's laws, the test's own doing.
Before any run, the moved scene's materials as authored, read in each
region's frame (density, stiffness, axis ratio, loss rates, restoring law
and every drive at three instants), must equal the original's at the
carried point; a scene that is not is the motion's fault. Both runs are
read at a 24 by 24 grid kept a coarse edge from every curve, the
complementary field carried as `R∇ψ` is, a reflection turning it `-M b`.
- **The study first.** Every linear gallery scene through a shift, a quarter
  turn, two mirrors and a turn by 37° at three levels. The moved run's
  distance from the original does not fall at a rate: a shifted scene may
  mesh almost alike at one level and not the next, and the drums' runs do
  not converge between levels at all. What held everywhere is that the
  moved run stands closer to the original than one halving of the mesh
  moves it, 0.56 of that at worst for the shifts, turns and mirrors.
- **The motion's own faults.** A mirror reverses a frame's own `x` or its
  `y`; the travelling modulation runs along a frame's `x`, so the first
  reversed it and four scenes read 0.11 to 1.14 apart at every level. The
  check, once it read the drives, refuses that mirror, and the test takes
  the other. The turn by 37° leaves the outer walls where they are, so it
  applies only where every curve is clear of the sides and the run ends
  before its waves reach them; ten scenes, 0.81 at worst.
- **Why the turn.** A frame turned the wrong way is invisible to a quarter
  turn and to a mirror: a tensor's axes repeat every 180°, and a reflection
  maps the backward turn onto the forward one. Only a generic angle sees it.
- **The gate's set**, 15 cases in 29 s on ten threads: the anisotropic
  crystal through all four motions, the double slit with its baffle
  outgoing on one side through three, Brewster's angle, the acoustic
  whispering gallery, the travelling modulation's mirror, the phased array,
  the echo comb and the skin depth, in each skin; 0.64 at worst. The
  nightly's `invariants` job carries every linear gallery scene.
- **Mutation** `principal-axes-turned-backwards`: a frame's tensor turned
  by the opposite angle. The turn reads the crystal 0.34 apart at every
  level, 107 times a halving's move; the shift, quarter turn and mirror pass
  it.

**IV-09 done on 2026-10-31**, in the same file, on IV-04's comparison with
the scene left where it is. Four rewrites keep a scene's geometry:
- every open curve reversed through the core's `TopologyCurve::reversed`,
  its anchors and boundary probes following;
- every closed curve started at its next breakpoint, its controls,
  intervals, spans and nodes turned round, taken only where it traces the
  same points;
- the curves listed backwards;
- a knot in the middle of every curve's first span, inserted through the
  editor's `insert_control`.

Each meshes otherwise. Over every linear gallery scene and the one-sided
double slit, 110 rewrites read within 0.69 of a halving's move; the gate
carries 8 in 7 s, the nightly every one. Its mutation
`span-laws-kept-on-reversal` leaves a reversed curve's laws where they
were: the one-sided double slit reversed reads 8.7 times a halving's move.

**IV-08's static half done on 2026-11-01**, in the same file, and with it
stage C2. The editor draws into the default box, in each skin, a loop, a
wall-to-wall separator and a free one, each of the box's own material, and
the box is read against itself without them by the same rule. On a medium
of unit coefficients every case read within 0.31 of a halving's move. On a
medium anisotropic in a frame turned by 0.4, the loop and the wall read
0.30 to 0.41 apart at every level, 840 to 910 times a halving's move: the
editor gave a new face of the material it divides a frame of its own,
centred on the face and turned by nothing, so the medium changed across
the line. Such a face now keeps the frame of the region it divides, a face
of another material keeps the centred seed, and the medium of the test,
graded along its frame's axis too, reads within 0.41. The check before the
runs now carries the stiffness tensor in world axes, so a frame's turn
cannot pass it. Its mutation `separator-face-recentred`. Milestone C's
stage C3, the remesh family, is next, planned before it starts.

**C3 planned on 2026-11-01**, on the user's go: IV-10, IV-11, IV-12 and
IV-08 edited mid-run, each through the app's own remesh. The runtime is asked
for the same document again, at another edge or rebuilt, its maps taken
before the generation is published, and the running field crosses as the app
carries it: the runtime transfer's totals, then the core's handoff, which the
device's transfer table is built from. A commit each, each with an f64 study
first and a mutation; the device halves are suite entries.

**IV-10 done on 2026-11-01**, in the same file, its CPU half. The editor
draws into the default box a reflecting baffle from wall to wall and, beside
it, an island inside a reflecting loop: three components. Struck by a pulse
all three take some of, the box runs to one time at its edge and half it, and
is prepared again 0.7 and 1.4 times finer and coarser, or rebuilt.
- **Totals from geometry.** Each part's total of the primary field is counted
  over the nodes of the elements whose centres lie in it, the parts told
  from the baffle's ends and the loop's trace, not from the solver's labels.
  Each must come through to 1e-12.
- **The field.** A rebuild must give it back to 1e-12, and a remesh's, read
  at common points, must stand within one halving's move of the original.
- **What it found.** Two defects, both fixed in its commit:
  - **A wall's nodes taken from its other side.** At edges of 0.04 and finer,
    a remesh fed up to 424 of the island's wall nodes from the field outside
    it, and the island's total moved by up to 5e-3. Each edge traces the
    curve with its own chords, so a new node on a convex wall lies outside
    the old mesh's elements on its side and inside the other side's, which
    the locator's search of every triangle then took. It now takes its own
    side's nearest element when it lies within a quarter of its height past
    it.
  - **A component's total shrunk by its area.** Each component's share of
    its old total was its new area over its old, capped at 1, and
    straight elements give a curved component a slightly different area on
    each mesh: a component a remesh made smaller lost up to 7.3e-4 of its
    total. Each old total now goes out whole among the components that draw
    on it.
- **Numbers.** The gate's 6 cases hold the totals to 1.2e-14 and read 0.44 of
  a halving's move at worst; the nightly's 18, down to edge 0.02, to 3.9e-14
  and 0.44.
- **Mutation** `component-share-over-source-support` puts back the shares
  over the old component's own support.

**IV-10's device half done on 2026-11-01**, the `neutral-remesh` fixture
in `crates/funfern-app/src/ui/e2e.rs`. The cavity at edge 0.04 is parted the
same way and each part struck by a pulse of its own, a pulse staying within
the walls it is placed between. At step 128 the mesh's edge is set to 0.028,
as its slider does, and the device's own state is read just before the
remesh and at the handoff. Each part's total, counted over the nodes of the
elements whose centres lie in it, must come through within 1e-6 of its
magnitude.
- **Numbers.** The M1 Max reads 5.5e-8.
- **What only the totals see.** With either of IV-10's defects put back,
  the device reads 1.1e-4 (the shares) and 1.1e-3 (the wall's sides), while
  its handed state stays within 5e-6 of the reference, which carries the
  field the same way.
- **Mutation** `device-correction-of-the-first-component` gives every node
  the first component's correction in the device's correction pass: 8.0e-4.
  Of the device suite's 30 remesh and handoff entries, only this fixture
  fails under it.

**IV-11 done on 2026-11-01**, `a_repeated_remesh_keeps_the_field_and_its_energy`
in `crates/funfern-app/tests/invariants.rs`. IV-10's box, its outer walls
reflecting too, runs at the finer mesh's time step and is remeshed to 0.7
of its edge and back K times on the way, through the runtime.
- **Checks.** Each part's total must come through every handoff to 1e-12.
  At the end the field, read node by node against the same run left on its
  mesh, must stand within 1.5 times one round trip's distance, and its energy
  no more than 1e-4 above that run's. A still leg is struck, run, and
  remeshed back and forth with no step between; the complementary field's
  energy must never rise over a round trip.
- **What it found: `b` grown by its transfer.** Sixteen round trips at half
  the default edge left Q 4.0 and `b` 43 from the control, and the energy
  255 times the start, every handoff booking what it added. Each new sample
  took the old element's quadratic evaluated at it, and at an element's
  corners the six samples' Lagrange functions sum to 3.55 in magnitude.
  The part of `b` that changes from sample to sample, which no step touches,
  was magnified at every handoff and grew threefold a round trip once it took
  over. A prototype weighed three rules: the old one, a clip to the old
  samples' range, which held but drifted and lost 50 times the accuracy on a
  smooth field, and an L2 projection, which held flat.
- **Fix.** A new element the old mesh covers on its own side, three quarters
  of it at least, takes the L2 projection of the old piecewise quadratic.
  The overlaps are clipped exactly and integrated with the six-point rule,
  exact for the products. The projection reproduces a quadratic and never
  enlarges `b`. Its side is the old elements reached from the one under its
  centre through the edges they share, which a reflecting wall splits. An
  element an edit leaves mostly uncovered still takes its samples one by one;
  IV-12 and IV-08 judge that case. The device's table holds a block of 36
  weights for each old element a new one draws on: 14.6 MB against 8.9 MB
  for 0.04 onto 0.028, and 14.4 against 4.3 back.
- **Numbers.** The gate's eight round trips read 1.14 and 1.09 of one's in Q
  and `b`, the nightly's worst 1.25 and 1.19, sixteen at half the edge. The
  energy rose 3.1e-5 at most, and fell by up to 1.8e-4, the projection
  shedding what the coarser mesh cannot hold. The still legs fall every
  round trip.
- **Mutation** `complementary-evaluated-not-projected` puts the evaluation
  back: eight round trips read 2.23 and 6.82, and the still leg's energy
  multiplies by up to 6.8 in a round trip.

**IV-12 done on 2026-11-02**, its CPU half,
`a_baffle_keeps_its_sides_through_a_remesh_and_a_move` in the same file. The
default box takes a free-ended reflecting baffle, an arc across its middle,
and is struck below it and run until the pulse has come back off it, the
field jumping across the baffle. It is remeshed at 0.7 and 1.4 times its
edge, or its baffle moved up or down by half an edge to two edges through
its control points, and carried over through the runtime.
- **Checks.** Read at 33 points along the baffle's middle, 0.3 of an edge to
  each side, the jump must come through a remesh within 1e-2. At a moved
  baffle it must stand within 0.3 of each side's own values: the old field
  where it stands on the side the baffle moved away from, and the old field
  beside the old baffle on the side that gained ground. No more than 0.1 of
  the jump may be left where the baffle was. Each in the field and the
  complementary field.
- **What it found: a moved wall left its jump behind.** Moved by half an
  edge, the baffle held almost none of the jump, and 0.79 to 0.86 of it
  stood where the baffle had been; moved by one or two edges, all of it.
  The transfer placed each new node and sample by position, so ground the
  wall swept over took its values from across the old wall, and so did the
  moved wall's own nodes past IV-10's quarter of a height.
- **Fix.** Ground a moved separated curve swept over takes nothing from the
  side it lay on before. Its nodes, samples and projected elements extend
  from their own side's nearest old element, however far, and the donor
  must lie on that side too. A point's side is that of the curve's nearest
  point on each mesh's trace, none at a tip, so beyond a free end the old
  rule holds; a point on a wall goes by its labels. Only the curves an edit
  moved are judged, at no measurable cost: a moved baffle's preparation at
  136 thousand nodes takes 2.9 s either way. The device's tables are built
  from these maps.
- **Numbers.** Over the nightly's 16 cases, down to half the edge, the
  remeshes read 1.5e-3 in Q and 4.0e-3 in `b` at worst. The moves read
  0.093 and 0.25 of each side's own, halving with the edge, with 0.022 and
  0.073 left behind; carried by position, 0.95 to 1.11 in Q, with 0.79 to
  1.07 left behind.
- **Mutation** `old-ground-sided-by-the-new-wall` reads the old ground's side
  off the new trace, so that nothing is swept: the moves read as by
  position. The core's `a_moved_wall_keeps_each_side_its_own_value`, a loop
  moved by part of an element and by two, catches it too.

**IV-12's device half done on 2026-11-02**, the `baffle-move` fixture in
`crates/funfern-app/src/ui/e2e.rs`. IV-12's arc is drawn into the cavity at
edge 0.04 and struck below it by a pulse as wide as IV-12's. At step 384,
once the pulse has come back off the baffle, the baffle is dragged up an
edge as the Move tool does. Read on the device's own state just before the
move and at the handoff, the jump at the moved baffle must stand within 0.3
of each side's own values, and no more than 0.1 of it may be left where the
baffle was: IV-12's bounds.
- **Numbers.** The M1 Max reads 0.024 in Q and 0.172 in `b`, with 0.010 and
  0.091 left behind; the handed state stands within 6.7e-7 and 1.2e-6 of
  the reference.
- **What only the device's own state sees.** The device has no code of its
  own for swept ground: its tables are built from the maps. With IV-12's
  defect put back, the device stands within 6.8e-7 and 1.2e-6 of the
  reference, which carries the field the same way, while its own state reads
  1.006 and 0.940 of each side's own, with 1.057 and 0.783 left behind.
- **Mutation** `old-ground-sided-by-the-new-wall-on-the-device` is that
  defect, run against the fixture.

**IV-08 edited mid-run done on 2026-11-02**, its CPU half,
`a_separator_drawn_or_deleted_mid_run_reads_as_none` in the same file. The
static half's box takes its loop, wall or free separator halfway through a
run, loses it halfway, or takes it at a third and loses it at two thirds,
through the editor as a user draws and deletes one. The runtime prepares
each edit, the running field is carried over as the app carries it, and
each generation runs at the step it recommends.
- **Control.** The plan had the control step through the edited run's steps.
  It cannot: the plain box prepared again has no maps to hand over with, and
  a separator can raise the step past what the plain box allows, the wall
  to 0.016518 against 0.016332. The control is the plain box run alone to
  the same time at its own step, as the app runs it unedited, so the step
  change at each handoff is under test too.
- **Check.** At the two finer levels the box must read against the control
  within one halving's move, in the field and the complementary field. The
  gate runs each separator and edit in one skin, a Latin square of 9 cases
  in about 10 s; the nightly runs all 27.
- **Numbers.** All 27 read within 0.51 of a halving's move, a wall deleted;
  drawn, within 0.27. A deletion reads about as its separator standing
  still, 0.37 to 0.42 at the default edge. A separator lowers the step by up
  to 2.4 times (the loop) or raises it by 5% (the wall), and a handoff
  changes the energy by 9.5e-5 at most at twice the edge, 5e-6 at the
  default edge.
- **What the study found: with outgoing walls the box converges at about
  first order.** At a fourth level, a quarter of the edge, the loop and the wall
  level off near 1.1e-4, 0.52 to 0.58 of a halving's move, static too. The
  floor is the plain box's own: a uniform medium shows it, it is spread
  over the box, halving the step leaves it, and with reflecting outer walls
  it goes. The box's own move halves per halving below the default edge
  with all four walls outgoing (5.2e-4, 2.2e-4, 1.1e-4) and with one
  first-order wall alone (3.7e-4, 1.4e-4, 7.0e-5), against 2.5e-4, 4.2e-5
  and 1.5e-5 with every wall reflecting. PH-04, PH-06 and PH-07 read the walls'
  reflection on a channel and on circles, at about second order; none reads
  the field inside a box. The cause is the strike, not the walls (below).
- **Mutation** `complementary-from-its-own-region-only` takes each new
  element's complementary field only from old elements of its own region,
  as until 2026-10-06: the loops and walls read 890 to 1340 times a
  halving's move, the free curves, which divide nothing, unchanged. The
  runtime's `a_new_or_retired_region_keeps_its_complementary_field`, that
  fix's regression test, catches it too, and so does
  `edits_the_field_cannot_tell_leave_it_as_the_mesh_allows_and_a_static_flux_static`.

**The outgoing walls cleared on 2026-11-02.** The first order was the
tests' strike. Their Gaussian, 0.3 wide in the default box, its centre 0.74
from the left side, stands at 2.3e-3 of its peak there at the start. A
strike sets the field with no flux, which the outgoing law cannot hold at
a wall, so a jump leaves the wall at once and travels in, and a jump
converges at first order whatever the scheme.
- **The wall alone.** A plane pulse, 1e-12 at the wall, meets the left wall
  at normal incidence, where the first-order law is exact, between
  reflecting floor and ceiling. Against the exact solution, once its
  left-going half has left, its error falls 9, 6.4 and 4.5 times over the
  halvings from 0.08, to 6e-6 at 0.01: the wall's own second order. With
  every side reflecting it falls 8 times.
- **Where and when.** With the tests' strike and the left wall outgoing, at
  a quarter of the run, 0.02 to 0.01 stands further apart than 0.04 to
  0.02, 2.1e-4 against 1.5e-4, most of it 0.2 to 0.4 from the wall, where
  the jump is by then.
- **A strike clear of the walls.** At width 0.15, 2.7e-11 of its peak at
  the walls, the box's own move per halving from 0.08 reads 2.3e-3, 2.4e-4
  and 3.3e-5 with the left wall first-order outgoing, 3.2e-3, 2.9e-4 and
  4.4e-5 with every wall second-order outgoing, and 2.3e-3, 2.3e-4 and
  2.9e-5 with every wall reflecting.
- **The tests keep their strike.** Struck at width 0.15, IV-08 reads 0.21
  of a halving's move at worst instead of 0.51, but only because the
  narrower pulse is resolved worse: the box's own move at the finest level
  rises from 4.1e-4 to 3.3e-3, so a defect of a given size would stand out
  less. The halving's move IV-04, IV-09 and IV-08 judge against is the
  scheme's under the strike they use.

**IV-08's device half done on 2026-11-02**, the `separator-mid-run` fixture
in `crates/funfern-app/src/ui/e2e.rs`, and with it stage C3. The cavity at
edge 0.04 takes IV-08's medium, anisotropic in a frame turned by 0.4 and
graded along it, and is struck off its middle by a pulse 0.2 wide. IV-08's
loop, a subdomain of the same medium, is drawn through the editor at step
128 and deleted at 192, its face given back to the background as the merge
picker gives it, and the run goes to 256. Each generation runs at its own
step, as the app runs it: at this edge the loop leaves the step as it was,
and the plain cavity meshed again after the deletion steps 1.9% shorter.
- **Checks.** Each handed state, and the endpoint, against the reference
  within 5e-6, as every handoff fixture's. The endpoint, read on the
  device's own state at the centres of a 24 by 24 grid, must stand from the
  unedited run within one halving's move of it, in Q and in `b`. The
  unedited run is the reference held at step 128, on the plain cavity, and
  stepped on to the endpoint's time in whole steps no longer than its own.
  The halving's move is a constant from the fixture's f64 twin: the
  unedited run at edge 0.08 against 0.04, 4.6e-4 in Q and 8.9e-4 in `b`.
- **Numbers.** On the M1 Max the handed states read 4.7e-7 and 5.5e-7 at
  step 128, 5.6e-7 and 6.9e-7 at 192, and the endpoint 6.4e-7 and 7.9e-7.
  The endpoint stands 1.53e-4 in Q and 7.1e-5 in `b` from the unedited run,
  as the f64 twin does, 0.33 and 0.08 of a halving's move.
- **What only the device's own state sees.** The device's tables are built
  from the maps. With the complementary field taken only from a new face's
  own region, as until 2026-10-06, the device stands within 6.5e-7 and
  7.8e-7 of the reference, which carries the field the same way, while its
  own state stands 0.30 and 0.36 from the unedited run.
- **Mutation** `complementary-from-its-own-region-only-on-the-device` is
  that defect, run against the fixture.

**C4 planned on 2026-11-02**, on the user's go: IV-05's `observers` fixture
and IV-01's `superposition` fixture, a commit each. Of the six observers the
11 October plan lists and no fixture ran, one reaches the solver: the
vector overlay binds the solver's state, nodes and tables and samples them
inside the solver's own pass, and opens that pass where no step is pending.
The recording reads the swapchain; auto exposure, the readout windows and
the pause are the CPU's, and a resize re-uploads the overlay's lattice.

**IV-05 done on 2026-11-02**, the `observers` fixture in
`crates/funfern-app/src/ui/e2e.rs`. The cavity with a point probe, struck
off its middle, runs to step 256 twice: once with every observer off, auto
exposure turned off since it is the app's default, and once with all of
them on.
- **The observed run.** The vector overlay, auto exposure and the probe's
  readout window from the start, and a recording started before the pulse,
  to a file in the run's own home natively, through a fixture's destination
  in place of the file dialog, and through MediaRecorder in the browser. At
  step 96, reached running, the run is paused for a second; at step 160 the
  recording stops, since a recording hides the readout windows, and the
  window is resized, natively the window and in the browser the page's
  body, and zoomed by a quarter, as Cmd+= zooms, which uploads the
  overlay's lattice again. The zoom is there because a window manager may
  refuse the resize: in the device suite this machine's kept the window,
  its viewport 756 by 846, where the fixture alone had it resized.
- **Checks.** Each run against its reference within 5e-6, the two
  endpoints the same bits, and each observer must show it took, or the run
  fails: the overlay's samples read back on the run's generation, the
  exposure's level set, the readout window drawn with a sample, the
  recording run a second at least, asked natively for 30 frame slots or
  more, and its file written, the device's step count held through the
  pause's second half, the viewport's size changed.
- **Numbers.** The M1 Max in the device suite: one hash at both endpoints,
  the overlay's 144 samples, the readout to 1.950 s, 2.3 s recorded in 136
  frame slots, paused at step 99, the viewport 756 by 846 zoomed to 605 by
  663. Chrome on the same machine: the same hash, 64 samples, 2.3 s
  recorded, paused at step 100, the viewport 1100 by 689 resized and zoomed
  to 832 by 473.
- **Mutation** `overlay-pass-takes-a-step`: a pass of the solver the overlay
  opens with no steps pending takes a step, so the run steps where nothing
  asked it to: the pulse lands at step 2, not 1. The overlay can reach the
  solver no other way: everything it binds of the solver's is read-only.
  `canonical_gpu_temporal_consumer`, which reads the overlay's arrows
  against their contract, passes with it.

**IV-01 within f32 done on 2026-11-02**, the `superposition` fixture in the
same file, and with it stage C4 and milestone C's P1 invariants. The
`handoff-memory` cavity, its bottom side a second-order outgoing wall and
its top driven at 0.8 Hz, runs to step 384 four times: struck at two
points and driven, struck at the first and driven, struck at the second,
near the wall, with the drive at zero, and all of it negated, the pulses at
amplitude -1, which the pulse slider reaches, and the drive's amplitude
negated.
- **Why by pulses.** A run driven alone, the plan's, leaves the wall almost
  no memory by step 384, the drive's wave just arriving: 1.6e-9 of energy
  against 7.0e-5, so its pole states read 4.3e-4 of their own size from
  the reference, the device's rounding about 2e-6 of the struck runs'. Each
  run now fills the wall's memory, and the drive is still a part of the
  sum. The top side shares no corner with the wall, so the run with the
  drive at zero, which pins its nodes to a zero signal, keeps the wall's
  trace whole.
- **Checks.** Each run against its reference, as every fixture. At the
  endpoints the first run must stand from the sum of the next two within
  1e-5, relative to the largest of the three, in Q, b and the wall's pole
  states; the last must be the first negated, every value, a zero of either
  sign counting as its negation. Rounding to the nearest is symmetric, so
  only an operation of the device's that is not odd can part them.
- **Numbers.** On the M1 Max each run stands 8.0e-7 to 9.4e-7 from its
  reference in Q and the wall's pole states up to 2.8e-6. The sum stands
  9.1e-7 in Q, 1.3e-6 in b and 2.0e-6 in the pole states from its parts'
  runs, and the negated run is the first negated, every value.
- **Mutation** `pulse-rounded-down` converts a pulse's increment to f32
  rounding toward minus infinity: no value moves by more than an ulp, each
  run stands with its reference and the sum with its parts, and 10,701 of
  the 11,121 values of Q in the negated run are not the first's negated.
  The app's 784 library tests pass with it.

## 6k. Milestone C: the P2 invariants

**Planned on 2026-11-02**, on the user's go, as stage C5: IV-07, IV-06 with
a device fixture, and IV-13, a commit each, each with an f64 study first and
a mutation.
- **IV-07, time reversibility.** A conservative run stepped on, reversed and
  stepped as far again comes back to where it started: on the fixed path by
  negating Q, which Störmer's step makes exact algebra, on the time-driven
  path by its signed step. Over every scene's conservative twin, its
  outgoing sides and faces reflecting, its losses gone, its sources silent.
  No device half: the device refuses a negative step, every fixture holds
  its stepping to the reference, and the superposition fixture holds its
  oddness to the bit.
- **IV-06, similarity scaling.** In time: every density divided by s², so
  waves run s times faster, every rate and frequency carried along, the
  step divided by s; at s = 2 the same bits up to powers of two, at another
  s roundoff. In length: coordinates, edge, curve tolerance and time times
  s, every rate divided by s; exact at s = 2 if a doubled scene meshes to
  the original doubled, otherwise IV-04's rule. A `similarity` fixture holds
  the device to the time similarity at s = 2.
- **IV-13, split/merge reversal.** Through the editor and the runtime: a
  separator of the box's own material drawn and deleted, the same detached
  from a wall and welded back, and two reflecting arcs welded tip to tip and
  undone, each K times, read as IV-11 reads: the end field within a multiple
  of one cycle's distance from the unedited run, its energy within 1e-4 of
  that run's, and a still leg never raising the complementary field's
  energy. It probes the elements an edit leaves mostly uncovered, which
  still take their samples one by one. No device half: the device's tables
  are built from the maps.

**IV-07 done on 2026-11-02**, `a_conservative_run_retraces_its_way_back` in
`crates/funfern-app/tests/invariants.rs`. Each of the 91 scenes'
conservative twin, and IV-12's box with its arc a thin gap in each skin,
since no scene carries a gap, is struck and run 256 steps on and 256 back.
A fixed generation goes back through the core's new `time_reversed`, its
primary flux negated before and after; a time-driven one, drives and
nonlinear laws included, by its signed step.
- **Check.** Q, `b`, `r` with the gaps' jumps, and the energy, each against
  the most of it the run held at either end of its way out, within 1e-13;
  within 1e-11 beside φ⁴, whose top grows what roundoff leaves.
- **What it found: sine-Gordon and φ⁴ stepped by a form that does not step
  back.** The production fourth-order step put its correction in the kicks
  beside a stiffness-side field law and in the drift otherwise. The drift
  form's correction reads the restoring law's stiffness `V″(r)` at the
  step's start, which sine-Gordon and φ⁴ change during the step, so a step
  back reads it at the other end. One step there and back missed by
  1.6e-10 to 9.1e-10 in `b`, where linear and Kerr media miss by 1e-17; 256
  steps missed by 4.4e-9 (the Josephson line), 7.3e-2 (the pinned domain
  wall) and 0.98 (the symmetry breaking), while a shifted strike grew 2.7
  times at most. Over 8192 steps the symmetry breaking's energy stood 3.6e-3
  above its start at step 4096 and 9.1e-3 at 8192. The forms' own
  documentation has the kick form exact under any restoring law over a
  linear mass, the drift form only while the other side is quadratic; the
  rule had left the restoring laws out. The core's sine-Gordon reversal
  held a small field to an absolute 1e-10 and passed.
- **Fix.** A restoring law whose `V″` follows `r` picks the kick form, as a
  stiffness-side field law does. The device takes its form from the CPU's
  choice, and its kick form carries the restoring curvature already. The
  three scenes now come back within 6.0e-15, 1.7e-13 and 8.8e-13, and the
  symmetry breaking's energy keeps within 1.0e-3 over 8192 steps, with no
  trend.
- **The trade.** On PH-16's uniform oscillators the kick form's phase is up
  to twice as far off as the drift form's, both second order, and its
  amplitude 15 to 20 times steadier at the larger swings. PH-16's
  sine-Gordon and φ⁴ bounds were measured again, at about twice the kick
  form's readings as they stood over the drift form's. On the gallery
  scenes the errors at four steps are the same either way.
- **Numbers.** Every other scene and the gaps come back within 6.0e-15,
  drives, Kerr, plasma, Klein-Gordon and the travelling modulation
  included: 94 cases in 23 s on ten threads, in the gate.
- **Mutation** `restoring-form-by-field-laws-only` puts the old rule back:
  the Josephson line misses by 8.8e-9, the φ⁴ scenes by 7.3e-2 and 0.98.
  The core's `restoring_media_balance_at_second_order_and_reverse`, now
  displaced to where the laws are nonlinear and held relatively, catches it
  too.

**IV-06 done on 2026-11-02**, `a_similar_scene_runs_as_the_original` and
`a_larger_scene_reads_as_the_original_at_every_level` in the same file.
Two similarities carry a scene.
- **Sooner.** Every medium's primary coefficient divided by `s²`, so its
  waves run `s` times faster; every rate and frequency times `s`, every
  duration and instant divided by it; the step divided by it.
- **Larger.** Its geometry, frames, formulas, source and probes `s` times
  as far from the origin, its mesh edge and curve tolerance with them,
  every rate divided by `s`, the step times it. A formula reads where its
  point stood, `x`, `y` and `r` divided by `s`.
- **What stays.** What a displacement reads stays as it was: `r` and a
  gap's jump in both, and `b` sooner. The primary field comes `s` times as
  strong sooner and `1/s` larger; the plan had it stay, which sine-Gordon
  and φ⁴ cannot carry. So they carry with their scales, and a field law with
  its amplitudes. A source drives the primary field's rate over weights the
  primary mass scales, and a harmonic is the acceleration it integrates:
  sooner, a pulse is `s²` times as strong and a harmonic `s³`. A Neumann
  side is a flux, and a Dirichlet side holds the field.
- **The study.** Run sooner by 4, all 91 scenes run to the original's bits,
  Q a quarter, `b` and `r` as they were; larger by 4, too, Q four times and
  `b` a quarter. Sooner by 2, 22 do and the other 69 stand within 1e-15:
  the second-order wall's modal transform takes the square root of the
  wall's damping, which scales by `1/s`, and `√2` rounds otherwise. With
  first-order or reflecting walls all 91 run to the bit at 2. A scene twice
  or four times as large meshes to the original's mesh scaled, to the bit;
  1.37 times, every scene meshes otherwise. The mesher caps the curve
  tolerance at an absolute 2e-3, ten times `WORLD_TOLERANCE`, which four
  times the app's 5e-4 reaches: a larger factor could not mesh alike.
- **Checks.** Sooner and larger by 4, the same mesh carried, the same step
  carried and the same bits, each quantity by its power of two, and the
  probes within 1e-12; sooner by 1.37 within 1e-12, 2.9e-13 at worst, a
  segment probe across the spatial soliton. Larger by 1.37 at three levels
  by IV-04's rule, its materials read against the original's first: the
  gate's 6 cases within 0.52 of a halving's move, the nightly's 40, every
  linear gallery scene and the one-sided double slit, within 0.53.
- **What it found.** Nothing: no time or length in the solver is absolute.
- **Mutation** `point-profile-reaching-the-default-width` measures a point
  profile's reach in the default source's widths, not its own: a source four
  times as wide is cut off at 2.25 of its widths, and 4 times larger 51 of
  the 91 scenes run apart, up to 1.9e-2. The core's tests of the reach call
  it with each width, and the core's 589 and the app's 784 library tests
  pass.

**IV-06's device half done on 2026-11-03**, the `similarity` fixture in
`crates/funfern-app/src/ui/e2e.rs`. The `handoff-memory` cavity, its bottom
side a second-order outgoing wall and its top driven, runs to step 384, and
again four times sooner: its density divided by 16, its drive four times as
fast and as strong, its pulse four times as strong, at the step the app
takes for it. The plan's factor was two; it is four for the wall's square
root, as on the CPU.
- **Checks.** Each run against its reference, as every fixture. The sooner
  run's step must be the first's over four, and at the endpoints its Q times
  four, and its b and the wall's pole states as they are, the first's, every
  value. The pole states are in energy coordinates and stay as they were,
  which IV-06 on the CPU now checks too, to the bit at 4; by 1.37 they stood
  up to 1.8 apart where the fields and the energy agreed within 1e-15, the
  wall's modal basis rounded otherwise, so there the energy reads them.
- **What the study found: f32's floor.** Struck by the app's usual pulse,
  the sooner run stood 8.2e-7 from the first in Q by step 384, every run
  within its reference. Its first parted values, at step 8, were near
  8e-38: a node the pulse leaves for the stencil to reach holds values down
  to where the GPU flushes them to zero, and the sooner run's Q, a quarter
  of the first's, flushes nodes the first keeps. One bit apart, every
  rounding after it parts, and the distance grew to f32's roundoff by step
  128. Struck by the slider's widest pulse, which reaches every node from
  the start, 1e-11 of its peak at the farthest, the runs are the same to
  the bit.
- **Numbers.** On the M1 Max and in Chrome on the same machine, both runs
  stand 8.5e-7 from their references in Q, 1.1e-6 in b and 1.7e-6 in the
  wall's pole states, and the sooner run is the first, its step and every
  value: 11,121 of Q, 43,860 of b, 240 pole states. The same hashes natively
  and in Chrome.
- **Mutation** `pulse-tail-dropped-below-a-fixed-size` drops a pulse's
  increments below 1e-6 on their way to the device, a size fixed in the
  field's units: each run stands with its reference, and the sooner run's
  pulse, four times as strong, keeps a tail the first drops, 10,740 values
  of Q apart. The `cavity` and `superposition` fixtures and the app's 55
  `canonical_gpu` tests pass with it.

## 7. Infrastructure

- **References.** `tests/physics/references/` holds the tables as JSON, the
  Python that generated each, and a check script that recomputes the
  identities the plan's section 6.4 asks for, precision refinement and
  limiting cases, run by hand when a table changes. A test reads its table.
  A closed form stays in its test (rule 7).
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
  on the device in three physics models; plane waves' numerical dispersion
  converges to the physical relation, the element's part and the
  integrator's told apart; planar reflection converges to the implemented
  law at three angles with the DtN gap reported.
- **A3:** the circular outgoing wall and the absorbing hole meet their law
  on a circle within their budgets and converge where the discretization
  error stands above the method's floor, the DtN gap reported, every hole
  passive.
- **B1:** every energy channel's lane meets an oracle written from its law,
  at second order, in each skin; the walls' power, storage and dissipation
  meet the law's own equations beside every medium; a remesh's energy is
  booked and the ledger closes across it, on the CPU and on the device.
- **B and C:** as the 11 October plan states per fixture, under the three
  rules of section 6.

## 9. First step

A0.1, the configuration sweep, presented as an engagement plan before any
edit: the generator's selection rule, where the sweep lives, its assertions
and its run time at edge 0.2, with the band fill of 1bc6d82 as its first
mutation.
