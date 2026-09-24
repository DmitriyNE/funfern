# Gate O: oscillator and active media

**Date:** 24 September 2026

**Status:** decided. Option A, the integrated-field state, was chosen by the
user. Nothing is relabelled and nothing is hidden: every skin offers every
preset, named for what its equation actually does.

## The question

The canonical state is `(Q, b)` with `Q = P(u)` and `ḃ = ηC U(Q)`, so
`b = ηCψ` with `ψ = ∫u dt`. The linear core therefore solves the wave equation
for `ψ`, and the displayed field `u` is `ψ̇`. Both satisfy the same linear
equation, which is why the difference has never mattered. A nonlinear
restoring law makes it matter: sine-Gordon in `u` and sine-Gordon in `ψ` are
different physics. Plan §10 already warns that adding `−V′(ψ)` to the flux
equation is not a nonlinear acceleration of the displayed field.

- **Option A** restores the integrated field `r = ∫u dt`, carried as one nodal
  state.
- **Option B** restores the displayed `u` itself, through a nodal current `J`
  with `J̇ = M V′(u)`.

Each adds one nodal scalar (an accepted and a candidate copy) and one `V′`
evaluation per node per step, so their costs are the same. They differ in
the energy they conserve.
- **A** keeps the canonical energy and adds one term. Every energy consumer
  (the filter's commit test, loss accounting, the energy and area readouts,
  AMR, handoff checks) gains a term and is otherwise unchanged.
- **B** conserves the energy of the `u` system,
  `½ pᵀM⁻¹p + ½uᵀKu + Σ m V(u)` with `p = −(F(b) + J)`, so each consumer
  would need a second version.

A was chosen.

## Equations (Option A)

Per node `i`, the restoring force sums every material meeting there. Each
contribution `c` carries its lumped authored mass `m₀_c`, meaning its
geometric weight times its base mass coefficient, and its own law:

```text
R_i(r) = Σ_c m₀_c V_c′(r_i)          (zero where no law is set)

ṙ = U(Q)
Q̇ = −F(b, t) − R(r) + sources − loss
ḃ = ηC U(Q) − loss
```

With `b = ηC r`, lossless and linear, this is `M₀ r̈ + K r + M₀ V′(r) = 0`, the
restoring equation in `r`. The displayed `u = ṙ` is its rate.

| Law | `V(r)` | `V′(r)` | `V″` bound |
| --- | --- | --- | --- |
| Klein–Gordon | `½ω₀² r²` | `ω₀² r` | `ω₀²` |
| sine-Gordon | `ω₀²(1 − cos r)` | `ω₀² sin r` | `ω₀²` |
| φ⁴ | `¼λ(r² − 1)²` | `λ(r³ − r)` | `λ·max(|3B² − 1|, 1)` with the authored `amplitude_bound` `B` |

**The authored mass, on purpose.** `ω₀` is then the cutoff of the medium as
authored. A drive on the mass row moves the cutoff as `ω₀·√(m₀/m(t))`, which
is honest and stated in the preset's help. It also keeps the restoring energy
free of explicit time dependence, so it adds no temporal work. A field law on
the same row does not touch the restoring coefficient either.

## Discrete step and energy

The drift advances `r` with `b`, on the same midpoint field and over the same
interval:

```text
kick   Q ← Q − (h/2)[F(b, t₀) + R(r)]      (+ sources and pins, as now)
drift  b ← b + h ηC U(Q),   r ← r + h U(Q)   (+ gaps, as now)
kick   Q ← Q − (h/2)[F(b, t₁) + R(r)]
```

The Hamiltonian stays separable:

```text
H = T(Q, t) + Φ(b, t) + Σ_i Σ_c m₀_c V_c(r_i)
```

- **Kick:** reads only `(b, r)`.
- **Drift:** reads only `Q`.

So the split stays Störmer–Verlet, time-reversible, and second order in its
energy balance. Loss composes by acting on `Q` and `b` as it does now, which
gives a damped sine-Gordon. `r` has no loss lane of its own, and complementary
loss lets `b` part from `ηC r`, which is the direct state's usual freedom.

**Walls.** Both outgoing walls impose `∂ₙu = −u_t/c` at normal incidence,
which is exact only for a non-dispersive wave. A Klein–Gordon plane wave with
`ck = √(ω² − ω₀²)` is reflected with amplitude `R = (ω − ck)/(ω + ck)`: 0.07
at `2ω₀`, 0.29 at `1.2ω₀`, and total at cutoff. The walls compose and
balance; they just do not absorb near the cutoff, and the presets say so.

**The grid filter.** The step keeps `b = ηC r` exactly when there is no
complementary loss. The filter's complementary correction is `δb = ηC δψ`,
so `r` takes the same `δψ`, and the filter acts on the total force on `ψ`:

```text
δψ = −s A K A (F(b) + R(r)),   b ← b + ηC δψ,   r ← r + δψ
```

Its first-order energy change is `−s (F + R)ᵀ A K A (F + R) ≤ 0`, and it is
zero at an equilibrium (a static kink, a wall or a well, where `F + R = 0`
with `F` itself nonzero). The commit test includes `V(r)`. The `Q`
correction is unchanged. A filter on `F` alone that left `r` where it was
would part `b` from `ηC r` for good.

## Timestep

Verlet on `M⁻¹K + V″` is stable for `h²(λ_max + V″_max)/4 ≤ 1`, so the
restoring law tightens the trajectory ceiling:

```text
1/dt² = 1/dt_bound² + V″_max/4
```

- Klein–Gordon and sine-Gordon know their bound up front.
- φ⁴ needs its authored amplitude bound. A node whose `|r|` passes it fails
  the step with the domain status, as defocusing Kerr does. No clipping.
- `V″ < 0` (φ⁴ near `r = 0`) is a physical instability, the unstable top of
  the double well, not a numerical one.

## Initialization and transfer

- **The uniform part of `r` is physical** for every law here, so `r` is
  authoritative state, not derived. A fresh generation starts at `r = 0`: the
  vacuum of Klein–Gordon and sine-Gordon, and the unstable top of φ⁴, which
  then breaks symmetry into domains. That is the φ⁴ demonstration, not a
  defect.
- **A handoff** carries `r` as a nodal field, interpolated like the displayed
  field, not conserved like `Q`. A static or driven source generation without
  `r` hands over `r = 0`. A target without restoring laws discards it, and
  says so in the handoff record (`transfer_integrated_field`). `b` keeps its
  own reconstruction rather than being rebuilt as `ηC r`, so a remesh leaves
  `b` and `ηC r` apart by the difference of two interpolation errors, which
  the step then keeps. Rebuilding `b` would remove that offset, but it would
  be wrong wherever complementary loss has parted them legitimately, and it
  measured no better.
- **Probes and snapshots** carry `r` beside `Q` and `b`. The GPU state gains
  one word per node for its accepted and candidate copies.

## Van der Pol (D3)

Van der Pol is a loss channel on the primary field, with rate
`γ(u) = γ₀(u²/a² − 1)`. That means gain below the threshold `a` and loss above
it. Per node over a half step the channel is exactly
`Q̇ = γ₀(1 − Q²/(M²a²)) Q`, a Bernoulli equation in `Q²`:

```text
Q(t)² = Q₀² e^{2γ₀t} / (1 + (Q₀²/(M²a²)) (e^{2γ₀t} − 1))
```

The sign of `Q` is kept. It replaces the passive half map at the same Strang
positions, so the step stays second order. The energy it adds or removes has
a lane of its own, "active gain", which may take either sign. Passivity is
replaced by boundedness of the map itself: the gain half map alone moves `|u|`
monotonically towards `a` and never past `max(|u₀|, a)`. The coupled wave
system has no such guarantee, and its behaviour is what the tests measure.
The authored `amplitude_bound` refuses the channel only where the threshold
would sit beyond it.

Beside Klein–Gordon on the same material, it makes a lattice of
self-oscillators with a limit cycle near `ω₀`. Alone it is a saturable-gain
wave medium.

## Skins and names

The equation is the same in every skin; only what `u`, and therefore `r`,
names changes. Presets say which.

| Skin | `u` | `r = ∫u dt` | Klein–Gordon | sine-Gordon | φ⁴ |
| --- | --- | --- | --- | --- | --- |
| TM | `E_z` | `−A_z`, the vector potential | cold plasma: a cutoff at `ω₀` | Josephson line: kinks are fluxons, `E_z` their voltage pulses | double-well integrated field |
| TE | `H_z` | `∫H_z dt` | the dual plasma | kinks in `∫H_z dt` | double-well integrated field |
| Mechanical | displacement | `∫u dt`, the time integral of displacement | a cutoff on the displacement | kinks in the integrated displacement | double-well integrated displacement |

Every law's hover states the equation, `M₀ r̈ + K r + M₀V′(r) = 0`, the
symbol `r = ∫u dt`, and that a static kink shows `u = 0`. The app gains an
"integrated field `r`" display, because that is where kinks and domains are
visible.

## What each sub-stage must show before it is enabled

1. **Klein–Gordon:** the discrete dispersion `ω² = ω_k² + ω₀²` on cavity
   modes, which is exact in the discrete operator, and the linear cutoff
   visible in `u`.
2. **sine-Gordon:** a static kink `4·atan(e^{x/ℓ})`, `ℓ = c/ω₀`, that holds
   still, and a moving one at its launched speed with the Lorentz
   contraction `ℓ√(1 − v²/c²)`.
3. **φ⁴:** the equilibria `±1`, the domain wall `tanh(x/(√2 ℓ))` with
   `ℓ = c/√λ`, and symmetry breaking from `r = 0`.
4. **van der Pol:** the exact half map against its closed form, growth from
   small amplitude, and saturation at the threshold.
5. **Every one:** a second-order energy balance, with the gain lane included
   for van der Pol, and reversibility for the conservative laws.
