# The material law catalogue

Every law a material can carry, what it does to a wave, how it is stored, and
whether it runs today. This is the shared vocabulary: the preset selector names
these rows, the engineering log refers to them by ID, and section 5.1 of
[the material-laws plan](spikes/funfern-material-laws-plan.md) is the contract
they are built against.

A coefficient composes as `c(x,t,field) = c0(x) · g_field · h_drive · h_switch`,
with at most one of each per row. `inverted` makes the whole multiplier divide
instead of multiply.

Status is one of:

- **runs** - authored, compiled and stepped, with a gate in `docs/checks.md`.
- **authored** - the type and its persistence exist and round-trip, but
  assembly refuses a material carrying it, so a document using it does not run.
- **CPU reference** - compiled and stepped by the f64 reference with its
  gates in the Stage 8 report; the app and device refuse it until Stage 9.
- **gated** - not admitted until the named design gate closes.

## Slot M — mass multiplier, `m -> m·g`

Per node, exact, stepped as `q = m·u`. Stored as `Material::mass_law`.

### Time-driven

| ID | Law | Phenomenon | Stored as | Status |
| --- | --- | --- | --- | --- |
| M-T1 | step `1 -> 1+Δ` at `t0`, optional ramp `τ` | temporal refraction, time reflection | `alternate` + `switch_ramp` | runs |
| M-T2 | harmonic `1 + A sin(2πft + φ)` | parametric amplification at `f ≈ 2f0`, time-crystal band gaps | `TimeDrive::ParametricPump` | runs |
| M-T3 | travelling `1 + A sin(2πft − q·x + φ)` | non-reciprocity, one-way bands, indirect frequency conversion | `TimeDrive::TravellingModulation` | runs |
| M-T4 | smoothed square / pulse train | the Floquet-standard modulation; sharper gaps than a sinusoid | `TimeDrive::TimeCrystal` | runs |

M-T1's `t0` is not authored. A Switch is stamped by the GPU at its own commit
boundary, so the material carries the shape and the runtime carries the moment;
`MaterialSwitchRuntime` is what a consumer reads back.

M-T3 needs only a two-float wavevector, and node positions are already in the
node table, so it costs no buffer.

The variant named `TimeCrystal` is M-T4, the smoothed square. Band gaps are
M-T2's phenomenon. Controls must use the phenomenon names in this table rather
than the Rust variant names.

### Field-driven

| ID | Law | Phenomenon | Stored as | Status |
| --- | --- | --- | --- | --- |
| M-F1 | Kerr `1 + χu²` | self-focusing, filamentation, self-phase modulation | `FieldLaw::Polynomial` with `chi1 = 0` | runs (direct; presets are self-focusing; χ < 0 needs `amplitude_bound`) |
| M-F2 | saturable Kerr `1 + χu²/(1 + u²/u_s²)` | stable filaments without collapse - the right default | `FieldLaw::Saturable` | runs (direct; `χu_s² > −8/9`) |
| M-F3 | quadratic `1 + χu` | asymmetric steepening into shocks, second-harmonic generation | `FieldLaw::Polynomial` with `chi2 = 0` | gated (C) |

Note the sign luck: self-focusing needs the permittivity to rise where the field
is strong, so `m` rises, so the local speed falls, and the CFL bound moves the
safe way. The flagship nonlinear demonstration cannot destabilize the step.
`χ < 0` and M-F3 can, and need a positivity guard and an amplitude monitor -
which is what `FieldLaw::Polynomial::amplitude_bound` is for.

## Slot K — stiffness multiplier, `K -> K·h`

Stored as `Material::stiffness_law`, the same `CoefficientLaw` shape as slot M.

**The time-driven half of this slot is not deferred; Stage 7 shipped it.**
`canonical_temporal.rs` evaluates `stiffness_law.drive` and
`temporal_amr_calibration` crosses a driven mass row against a driven stiffness
row against both. K-T1 to K-T4 are M-T1 to M-T4 on this row, and all run.

That makes the one thing only this slot can do available now. A time interface
in `m` alone changes both the speed and the impedance `Z = √(mK)`; modulating
`m` and `K` together so `Z` is unchanged gives a **reflectionless time
interface**, the direct contrast against M-T1's reflecting one. The pairing is
the same drive on both rows with **neither** inverted: every stiffness-row law
already divides the scalar `K` (it multiplies μ, or the mechanical reciprocal
stiffness `s₀`), so `m·h` with `K/h` holds `√(mK)`. Until 2026-09-24 the preset
inverted the stiffness row, which held the speed and moved the impedance by the
whole factor - the interface that reflects most. The claim is falsifiable and
now measured: `a_constant_impedance_modulation_sends_nothing_back` sends
< 1e-3 of a pulse's energy back under the pair against 4.4% under the old,
impedance-only pairing.

The field-driven half follows slot M's: Kerr and saturable run on the CPU
reference on this row too. They act on the direct coefficient `s₀` (ε in TE),
at quadrature, on the magnitude of the independent complementary field, which
is the field argument the constitutive map needs. There is no symmetric nodal
flux argument like `h((u_i + u_j)/2)`. An anisotropic medium refuses a
nonlinear law on this row (gate C).

## Slot R — additive restoring term on the integrated field, `−m₀V′(r)`

Gate O decided the argument: the law acts on the integrated primary field
`r = ∫u dt`, carried as one nodal state, not on the displayed `u`
([the decision](spikes/funfern-gate-o.md)). The equation is
`M₀r̈ + Kr + M₀V′(r) = 0`, and the displayed field is `u = ṙ`, so a static
kink shows `u = 0`; the Integrated field view shows `r`. Stored as
`Material::restoring`, default `None`. A restoring preset is its own family
beside the response presets, so a material can carry one beside any response.

| ID | Law | Phenomenon | Stored as | Status |
| --- | --- | --- | --- | --- |
| R1 | Klein-Gordon `ω0²r` (linear) | dispersion `ω² = c²k² + ω0²`, cutoff frequency | `RestoringLaw::KleinGordon` | runs |
| R2 | sine-Gordon `ω0² sin r` | kinks, antikinks, breathers - the soliton showcase | `RestoringLaw::SineGordon` | runs |
| R3 | `φ⁴`: `λ(r³ − r)` | domain walls, symmetry breaking from `r = 0` | `RestoringLaw::Phi4` | runs (needs its amplitude bound) |

Each skin names the law for what `r` is there: in TM `r = −A_z`, so R1 is a
cold plasma and R2 a Josephson line whose kinks are fluxons; in TE
`r = ∫H_z dt`; in Mechanical the time integral of the displacement. R1's
dispersion is exact in the discrete operator. R2's `V''` is bounded, so its
step contribution is known up front; R3's is bounded by its authored
amplitude, and a node past it fails the step.

## Slot D — damping multiplier, `d -> d·w`

Per node, as cheap as M. Stored as `Material::electric_loss` and
`magnetic_loss`, independent channels, each a `LossChannel { base_rate, law }`
whose `DampingLaw` carries a rate law and its own time drive.

| ID | Law | Phenomenon | Stored as | Status |
| --- | --- | --- | --- | --- |
| D1 | time-modulated loss | loss-driven parametric effects, PT-symmetry-flavoured pairs with gain | `DampingLaw::drive` over `RateLaw::Constant` | runs (Advanced view; no preset) |
| D2 | saturable absorption `1/(1 + u²/u_s²)` | self-limiting, passive mode-locking flavour | `RateLaw::SaturableAbsorption` | gated (C) |
| D3 | van der Pol `γ₀(u²/a² − 1)` | self-oscillation, spontaneous pattern formation | `RateLaw::VanDerPol` | runs (primary row, undriven, beside a linear response) |

A constant loss channel runs on the fixed path. D1 runs on the time-driven one:
the CPU reference since Stage 7, and the device since 24 September 2026. The
device reads each loss stage's rate from per-site loss records at the stage's
own instant, and weighs a node shared by several materials by the masses in
force. Before that it packed authored rates at compile time and dropped the
drive; `canonical_gpu_long_run` with `LONG_RUN_LOSS` is the gate. Each row has
one loss in the editor, the channel on its physical field. Legacy `damping` is
the channel on the primary field, and the first loss edit moves it there.

D3 is a deliberate instability, so it is explicit: its own loss kind on the
primary row, with its energy in an active-gain lane of either sign rather than
in loss. Its half map is the exact Bernoulli solution, on the CPU and the
device.

## Where the refusals live

| Path | Refuses |
| --- | --- |
| `canonical_wave.rs`, `linear_material_sample` | a non-linear `mass_law` or `stiffness_law`, and any restoring law. Admits loss channels. `is_linear()` includes `drive.is_none()`, which is why a driven generation assembles from a stripped model and compiles its laws separately. |
| `wave.rs`, `evaluate_timed_directional_material_library_at` | any loss channel, so there is no adaptive estimate for a driven medium carrying loss. A restoring law changes no coefficient and is read past (the size rule can only over-resolve a Klein-Gordon medium); its store and force are in the canonical supplement. Field laws are read at their small-signal limit; the size rule applies the primary row's tangent separately. |
| `geometry.rs`, `Material::evaluate_static` | anything but constant loss channels beside linear rows, and legacy damping beside a named channel. It reads the primary field's channel as the static scalar operator's damping. |

A preset whose law is not **runs** is filtered out of the selector rather than
offered and refused, so every document a user can author assembles.
