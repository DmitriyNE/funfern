# Material-law Stage 10: the product, and what finishing it found

**Date:** 24 September 2026

**Target:** Apple M1 Max, Metal/WebGPU, f32; CPU reference in f64

**Scope:** the second release boundary. It covers:
- the material editor's Simplified and Advanced views;
- readouts and the Switch control;
- authoring helpers;
- gallery demonstrations;
- the carried compositions (the grid filter and area readout on nonlinear
  media, and an amplitude-aware size rule);
- compatibility and documentation.

Finishing the product surface turned up five defects, all fixed and each
now covered by a gate. Three sat in paths every earlier stage had tested
(the device clock, device loss, and the reflectionless preset). Two blocked
this stage's own work (named loss channels in the app, and the Mechanical
formula text). They are listed first.

## Defects found and fixed

| defect | how it surfaced | before | after | gate |
| --- | --- | --- | --- | --- |
| The device's epoch-local time was an f32 running sum, so every step's rounding stayed. | GPU checks of the gallery scenes | pump scene Q 6.1e-5 at 400 steps; 0.12 s behind over a full epoch (1.9 rad at 2.5 Hz) | 1.5e-6; clock-sensitive gates 5–10× tighter | `canonical_gpu_long_run` (exits 1 with the old clock) |
| The device read loss as authored constants: a loss drive was dropped, and a node shared by materials whose masses move kept the authored weighting. | Writing the README's loss claim | 2.3e-2 driven, 3.2e-4 constant, at 200 steps | ≤ 7.3e-6 at 400 steps | `canonical_gpu_long_run` with `LONG_RUN_LOSS` |
| Named loss channels did not prepare in the app: the scalar assembly counted a loss channel as a law. | The regrouped editor's tests, and the user's autosave | "Cannot assemble wave operator" | prepares; rates identical to legacy damping | `a_static_evaluation_reads_the_primary_loss_channel_as_damping`, `a_complementary_loss_prepares_and_reaches_the_solver` |
| The reflectionless time interface inverted its stiffness row, holding the speed and moving the impedance. | Placing each formula in its row | `√(mK)` ×1.5, speed ×1.0 | `√(mK)` ×1.000, speed moves; backward wave 0.0000 against 0.0439 | `the_reflectionless_pair_holds_the_impedance_and_moves_the_speed`, `a_constant_impedance_modulation_sends_nothing_back` |
| The Mechanical stiffness row's effective-law text read `k = k₀ · (1 + …)`, but the law divides `k`. | Same | wrong direction | `s = s₀ · (…)` | `each_row_reads_its_own_law` |

## What shipped

- **Grid filter on every composition** (10.1).
  - **CPU:** the time-driven and tangent filters compose with walls of
    both orders, pins, gaps and loss. With no drive or law they equal the
    fixed filter to 1e-12. On driven and Kerr media all 150 filters commit
    and only remove energy.
  - **Device:** a site pass freezes the maps at the event instant.
    Energies are formed in parallel passes, and a nonlinear candidate that
    gains energy is skipped.
    - Parity is within 2.7e-6 across 10 media and compositions; with the
      device filter off the mismatch is 2.3e-2.
    - Cost is +2% a step on driven linear media and +1% on nonlinear. The
      first cut cost +30–35%.
- **Readouts** (10.2):
  - each nonlinear row's peak coefficient change;
  - what sets the step ceiling, by row.
- **Switch** (10.3): a button and the S hotkey. It is a run-time act with no
  undo entry; its ramp travels with the runtime bank, as every Switch has
  since Stage 7.
- **Material editor** (10.4, and the redesign that followed):
  - One group per coefficient: base value (`s₀` in Mechanical Advanced),
    one loss rate for that row's physical field, the preset's values or
    every law slot, the row's formula at the top, and its live readout.
  - Advanced adds a Names | Numbers toggle.
  - Legacy damping shows as the primary row's loss and moves to its named
    channel on first edit.
  - Only laws the solver runs are offered, and a slot holding one it does
    not run is left alone.
- **Helpers** (10.5):
  - `= 2 × source` for pump frequencies, with a menu when there are
    several sources;
  - parameter rename that is all-or-nothing;
  - a Laws section in the formula help, tied to the preset catalogue by a
    test.
- **Gallery** (10.6): four scenes, each with a CPU test that measures its
  claim.

  | scene | claim | measured |
  | --- | --- | --- |
  | Kerr slab | third harmonic at the receiver | 26% of the fundamental, against 0.03% at χ = 0 |
  | Parametric pump | gain at 2f set by the pump's phase | 1.2–2.3× across phases; detuned 0.87–0.96×, flat |
  | Time crystal | sidebands reaching f + 3f_m | 3.4–4.6× a sinusoidal pump's |
  | Travelling modulation | conversion only with the modulation's direction | 6–14× forward over the mirrored run |

  Self-focusing is not claimed. A pump at 6.9 Hz also amplifies; the log
  records it.
- **Carried compositions** (10.7):
  - The area readout on nonlinear media on both sides. It splits each
    node's store exactly over its materials. A full-coverage probe equals
    the solver's energy to 1e-9; device parity is 1.3e-7.
  - The size rule divides its wavelength floor by the primary row's
    tangent at the field envelope. It is phase-independent and never
    coarsens.
- **Compatibility** (10.8):
  - Every preset in all three skins survives the file and prepares, with a
    time-driven operator exactly where it writes a law.
  - Malformed laws are refused at load.
  - Version-22 files still load.

## Refused, deferred or open

| item | state |
| --- | --- |
| Filter strength calibration | deferred to after the milestone, by decision |
| Long-run device drift past 1000 steps on driven and nonlinear scenes | characterized, not fixed: at 4000 steps the fixed path is 5.8e-6 and growing linearly, the pump 4.0e-5, Kerr 1.05e-4. Step rounding and state storage are ruled out. The pump is partly the size of f32 phase arguments; Kerr is likely its sensitivity plus the 4ε₃₂ inverse. The remedy would be compensated phase arithmetic. |
| Size rule for complementary-row field laws | small-signal limit; the estimator does not read that field through its inverse |
| AMR estimate on a driven medium carrying loss | refused, as since Stage 7 |
| Pulses and live law patches on nonlinear plans; live law patches that change a loss | take a new generation |
| Defocusing presets, signed χ₁, reciprocal field laws, field-dependent loss, prescribed data on an outgoing trace | refused (gates C and O) |
| The Switch temporal interface as a gallery scene | needs a run-time act a document cannot hold |

## Verification

- Every commit passed rustfmt, clippy `-D warnings`, the workspace tests,
  the release build and the wasm32 check. One commit was amended to meet
  clippy after its tests had passed.
- **Device suite:** 39 modes, all exit 0, plus `canonical_gpu_long_run` over
  the gallery with and without loss, run on the M1 Max with an isolated
  HOME.
- **Not checked by me:** the new UI on screen, which the user reviewed as it
  landed.
