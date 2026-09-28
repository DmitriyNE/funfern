# Material-law Stage 11: oscillator and active media, and what finishing it found

**Date:** 27 September 2026

**Target:** Apple M1 Max, Metal/WebGPU, f32; CPU reference in f64

**Scope:** the extensions plan §10 left to their own derivations, Gate O.
It covers:
- the Gate O decision (11.0);
- Klein-Gordon, sine-Gordon, φ⁴ and van der Pol on the CPU reference
  (11.1);
- their composition with walls, pins, gaps, loss, sources, the grid
  filter, transfer and the adaptive estimate (11.2);
- the device port, van der Pol's gain lane and the handoff of the new
  state (11.3);
- presets, the material editor, the integrated-field view and the
  readouts (11.4);
- the gallery: four oscillator scenes and a pass over every other scene,
  which grew the catalogue from 12 entries to 36 (11.5);
- the onboarding and layout work that followed on the same branch.

The stage is 87 commits before this report, from the Gate O decision to
the gallery plan's close. Finishing it turned up thirty-odd defects, all fixed and each
behind a gate; the ones that matter to the physics are listed first, then
the ones the gallery and the app turned up.

## Defects found and fixed

| defect | how it surfaced | before | after | gate |
| --- | --- | --- | --- | --- |
| A grid filter that acted on `F` alone parted `b` from `ηC r`, an invariant of the step. | Designing 11.2 | `b − ηC r` 0.113 by 5 s, then held | 1.0e-11; the filter acts on `F + R` and moves `r` by the same `δψ` | `the_grid_filter_keeps_an_oscillator_equilibrium_and_b_equal_to_eta_c_r`; the device reads 1.3e-4 with `r` untouched and fails |
| A handoff carried `b` and `r` separately, so where a moved boundary made them disagree the field settled against a stress it could never shed. | A crack-like wall imprinted where a boundary had moved from, reported by the user | `\|D\|/\|b\|` 0.45 after one drag, 0.60 after six; the wall stopped 0.026 short | 1e-13; `D = b − ηC r` crosses on the vector map and `b` is rebuilt about the new `r`; the wall reaches 0.299 | `the_oscillator_flux_carries_the_invariant_across_a_handoff`; the pinned wall's drag test under 1e-9; `an_identity_handoff_carries_the_integrated_field_exactly` |
| `r` started at zero in ground a moved boundary opened. | Dragging the pinned wall's bumps | +1.20 on an energy of 3.96 | +0.03–0.10; the opened ground is filled from its neighbours | the transfer test checks `r`; `canonical_gpu_oscillator_handoff` `opened` mode |
| Van der Pol gain lased on the mesh's shortest waves. | The self-sustained emitter; the user saw it too | 11.5 Hz at edge 0.08; the ceiling mode grew at 2.00 | a short-wave viscosity, κ = 8; the ceiling mode at −10.1, a smooth mode 2.063 → 2.030; the device without it parts by 8.9e-2 | `the_short_wave_viscosity_damps_the_mesh_ceiling_and_spares_long_waves`, `a_self_oscillating_rim_keeps_to_its_tone` |
| The f32 van der Pol map compounded its rounding below threshold. | The 11.3b junction runs | Q 4.2e-6 at 200 steps; b 3.5e-5 at 1000; lane 7.4e-5 | 3.9e-7; 1.0e-5; 1.2e-7 | `canonical_gpu_oscillator`, van der Pol from 0.05 over 1000 steps |
| The device's loss share `1 − e^{−x}` cancelled at common rates. | The symmetry-breaking scene's device run | 3.4e-5 at 400 steps, 9.6e-5 at 1000 | 8.7e-7 at 400; no throughput change | `canonical_gpu_long_run` on that scene |
| The step's energy change counted gap and pole stores before but not after. | Writing 11.1 | the splitting residual was wrong on every gapped or second-order-walled scene | both sides include the stores | the 11.1 balance tests |
| The device area probe lacked `V(r)`. | 11.4c | missed by 0.999 | 2.1e-7 | `CONSUMER_OSCILLATOR=1`; `an_oscillator_area_probe_over_every_face_reports_the_solver_energy` |
| `Material::time_invariant` ignored restoring laws and field-dependent loss, which would have routed them to the fixed path. | App wiring | refused at assembly | read as time-varying | the 11.4a round trips prepare a time-driven operator |
| A sine switch-on left the field a mean velocity `amplitude·cos(phase)/ω`, a static offset in any sealed region. | The Doppler mirror: lines at 2 and 4 Hz | 0.191 beside a 0.19 carrier, at every probe | 4e-5; the point source, the panel's new volume source and the gallery launcher start on a cosine (`SWITCH_ON_PHASE`) | the Doppler test: the static field under a thousandth of the carrier |
| Mesh decisions went through the platform's `acos`, one ulp apart between macOS and Linux, and reordered refinement. | Linux CI: a carve refill at 4.11° against the 5° floor | platform-dependent meshes | portable functions and `pseudo_angle`; a refill under the floor carves again wider | `welded_baffles_mesh_the_same_on_every_platform` (FNV digests), `portable_acos_matches_the_platform_to_a_last_bit`, `a_refill_under_the_floor_carves_again_with_a_wider_cavity` |
| Two one-ended baffles welded in one face did not mesh; moving a welded baffle fell back to a full rebuild. | Building and dragging the pinned wall | "topology face cycle is incomplete"; "cavity boundary has a dead end" | spur feet seeded as one vertex; `expand` seeds slit feet | `topology_mesher_handles_two_one_ended_baffles_in_one_face`, `a_moved_welded_baffle_is_carved_with_its_foot_split_again` |
| Greedy coarsening left one-segment remainders of 0.004 on curves, which set the step. | The photonic crystal | phased array dt 8.6e-4 | 2.40e-3; three other scenes 1.5–1.6× | `an_even_cut_replaces_a_greedy_remainder` |
| The mesher's caps were not sized to the edge slider. | The user's autosave failed to mesh at 0.02 | "Refinement limit reached" | `sized_for_area`; 40,945 triangles build; a "≈ triangles · DOFs" estimate under the slider | `caps_sized_for_the_domain_mesh`, `the_mesh_estimate_follows_meshes_actually_built` |
| A scene change kept the old field running under the new geometry; Undo carried a field across; `clear` kept the step backlog. | Reported; a mid-run switch check | the old field for seconds; stuck at "Ready for GPU upload" | the outgoing generation is dropped, not paused | `a_replaced_scene_drops_the_outgoing_generation`, `a_cleared_request_owes_no_steps` and three more |
| Auto exposure kept the previous scene's peak, so every quieter scene after the Josephson line painted black. | The view-settings pass | reference 0.35 for a field of 0.07 | `clear_exposures` on replacement, `follow_field_quantity` on the field/integrated switch | `a_replaced_scene_starts_its_scales_from_nothing` |
| `debug_assert!` with a side effect: parameters were not deleted in release. | Reported by the user | a no-op | `delete_parameter` | `a_parameter_is_deleted_unless_a_formula_uses_it`, in release |
| A `workgroupBarrier` in non-uniform control flow: naga accepts it, Chrome does not. | The browser build failed to start | no field in the browser | `workgroupUniformLoad` | `npm run test:shaders` joins the gate |
| The transfer layout version was not bumped in the runtime shader. | The handoff examples | every handoff failed its layout check | bumped in three places | the handoff examples |
| The response matcher called any loss channel Custom; the restoring summary lacked its mass. | 11.4a and 11.4b | a lossy Kerr medium read Custom; `3²·u` | names the rows alone; `R = ρ₀·3²·r` | the preset tests; the summary test in TM and TE |
| The integrated-field view blinked at a handoff and forced a full readback every frame. | Reported | a few frames of `u` | `r` in the receipt and a live stream, within 2.6e-7 | `canonical_gpu_oscillator_handoff` |
| A press on a selected curve went to a control; hidden handles still took clicks. | Reported | presses captured by controls | `topology_hit` | `a_press_on_a_selected_curve_moves_it` |
| The top bar overlapped itself below 1080 px, and the status strip below about 900. | Measuring for a phone | the right end over the left | `ToolbarFold` in six steps, `StatusPlan` | `the_top_bar_fits_from_a_phone_to_a_desktop` (fails at 880 with folding off), `the_status_strip_fits_from_a_phone_to_a_desktop` (fails at 568 uncut) |
| The GRIN collimator's width claim measured where a shoulder fell, not spreading. | Its move to the E_z skin | 0.75 then 0.55, passing by luck in Mechanical | the claim is the power share, 0.78 at x = 0.75, no less than at 0.2 | `the_grin_collimator_sends_out_a_beam_that_does_not_spread` |

Also fixed, with their own log entries: the undo shortcuts dropped in an
earlier UI cut, seven gallery scenes whose probes overlapped, the double
slit's end gaps and reflecting walls, a point probe on a curve, CI running
its tests in debug (over 45 minutes; 118 s in release), and two egui sizing
traps in the gallery window.

## What shipped

- **Gate O** (11.0): restoring laws act on the integrated field
  `r = ∫u dt`, one nodal state beside `Q` and `b` (Option A). The drift
  adds `ṙ = U(Q)`, the kick `Q̇ −= Σ m₀ V′(r)`, the energy `+ Σ m₀ V(r)`.
  The Hamiltonian stays separable, so the step stays Störmer-Verlet and
  every energy consumer gains one term. Option B, a nodal current on the
  displayed `u`, costs the same but conserves another energy. Nothing is
  relabelled and nothing hidden: each skin names the law for what `r` is
  there, `−A_z` in TM.
- **CPU reference** (11.1), each law against its own closed form:
  - **Klein-Gordon:** the box's lowest mode moves 1.57080 → 2.95254, and
    `(ω² − ω_lin²)/ω₀²` = 1.00002; the dispersion is exact in the discrete
    operator.
  - **sine-Gordon:** a kink at rest holds to 9e-8 in 1 s, with energy
    63.957 against 64; launched at 0.5 it runs at 0.4990, contracted
    1.1553 against γ = 1.1547.
  - **φ⁴:** ±1 holds to 1e-12; the tanh wall moves under 0.02 in 1 s; a
    one-signed seed falls wholly into one well and an antisymmetric one
    settles to a wall of 1.0005 × 2σ.
  - **van der Pol:** an exact Bernoulli half map matching a 20,000-step RK
    run to 1e-12; the limit cycle at 0.57767 against Rayleigh's 0.57735;
    its gain lane closes to 1.8e-4.
  - Both conservative laws are reversible to 1e-10.
- **Compositions** (11.2): 29 cases of walls, pins, gaps, loss and sources
  balance at order 1.993–2.003. The grid filter acts on the total force
  and keeps `b = ηC r` to 1e-11. `transfer_integrated_field` carries `r`
  across a remesh (a kink at rest moves −4.0e-7, a moving one runs at
  0.4989). The adaptive estimate reads the restoring store and the trace
  force (residual 2.7e-7 with `R`, 6.9e-4 without); its efficiency spread
  is 1.12× on both conservative laws against 1.18× inert. The outgoing
  wall's reflection on Klein-Gordon, `R = (ω − ck)/(ω + ck)`, was derived
  here and measured in the plasma mirror: 0.289 against 0.288.
- **Device** (11.3): `r` is one auxiliary word per node; the sine-Gordon
  potential is formed as `2 sin²(r/2)`; φ⁴ past its bound fails the step
  with status 6 and the accepted state is untouched (a retry leaves every
  bit unchanged). Van der Pol runs with its own active-gain lane; the
  control block grew from 272 to 304 bytes, layout 5. The handoff carries
  `r` and the invariant in a transfer layout of 10 words, version 3. Cost
  at 15,270 DOFs: driven 433–442 µs a step, with sine-Gordon 425–467, van
  der Pol 650, about 8% over a constant loss. Suites: 84 oscillator runs
  and 120 van der Pol runs within 3e-5, and 56 regression runs.
- **App** (11.4): presets R1–R3 with per-skin text, carried across
  Mechanical↔EM; a Restoring force group in the editor and a
  Constant | Self-oscillating loss kind; a per-skin Integrated field view;
  `m₀V(r)` in the area and energy readouts (CPU 1e-9, device 2.1e-7);
  end-to-end device runs of pump, sine-Gordon and van der Pol documents
  within 3.0e-6. Then, at the user's request: the simple view offers whole
  media, selecting a region selects its material, and each material keeps
  its own Advanced view.
- **Gallery** (11.5): 27 planned scenes and two more, each authored with
  the app's own tools, each behind a CPU claim test at edge 0.08 and a
  device run. The oscillator scenes:

  | scene | claim | measured |
  | --- | --- | --- |
  | Plasma mirror | a Klein-Gordon ramp turns the wave back; the wall reflects as derived | R 0.289 against 0.288; k 10.44 against 10.42 |
  | Josephson line | a biased line sheds one fluxon per 2π of its phase | emission gaps 2.092–2.147 against 2.094; speed 0.466–0.470 |
  | Symmetry breaking | φ⁴ from frozen noise breaks into domains of both signs, then coarsens | both signs at 2 s; one well within 2% at 8 s; λ = 0 keeps `\|r\|` under 0.1 |
  | Pinned domain wall | the wall slides to a channel's waist and follows it when dragged | within 0.03 at 6–8 s; six drags within 0.05 |
  | Self-sustained emitter | a van der Pol disk rings at its cutoff and radiates only through a lower one | tone 2.998 Hz; 1.181 at the centre against 1.155 |
  | Plasma whispering gallery | a vacuum disk in a plasma rings in its m = 5 mode below the cutoff | rim 0.169 on resonance against 0.037 off; decay 0.288 against K₅'s 0.250 |
  | Parametric fiber amplifier | a pump running with the signal amplifies it; a standing one oscillates | 4.85× at 8 s; a uniform pump grows 0.125 → 0.705 |

  The pacemaker failed its exploration (wave-coupled, winner takes all)
  and shipped as the emitter. The other twenty scenes are optics and
  acoustics on the linear core: lenses, guides, crystals, resonators,
  interfaces and diffraction, listed with their numbers in the gallery
  plan and the log. Every launcher scene was measured again when sources
  moved to a cosine start; most moved in the third digit.
- **View settings and onboarding:** every scene opens showing what it is
  about (arrows, overlays, exposure, and probe readouts kept with the
  scene); the catalogue is sectioned into seven groups; an Examples button
  opens a sectioned gallery; a scene card steps through the examples with a
  first-launch hint; Ctrl/Cmd+Z; a top bar that folds in six steps from
  865 px to fit a 360 px phone, with Reset kept; a status strip that fits
  its width; no inspector at launch below 700 px. Checked in the browser
  build at 360–1280 px.

## Refused, deferred or open

| item | state |
| --- | --- |
| Plasmonic guide | proposed by the user; a surface mode needs a negative ε on the in-plane field, which is the complementary row in H_z; Gate O's laws act on the primary row only. Most likely needs a Drude law on the complementary row. Analysis left for later. |
| Mixing amplifiers' mesh-scale excitations | seen by the user in the fiber amplifier and the Kerr scenes; the likely cause is the phase-matched sum-frequency ladder, not measured. Filed under Maintenance in `docs/plan.md`. Next up for discussion. |
| "Topology ear clipping stalled" on a 35-octagon random subset | diagnosed on 27 September, not fixed: a second bridge to a vertex that already carries one is spliced at that vertex's first copy in the polygon, not at the copy whose wedge the new bridge leaves through, so the bridged polygon overlaps itself and no ear passes. The axis-aligned squares reported earlier mesh at every edge now; which mesher change since fixed them was not traced. See the log. |
| Van der Pol beside a field law, on the AMR supplement, or with a named loss on skin conversion | refused |
| Point and line probes | record `u` only, no `r` channel; the point energy density lacks `V(r)` |
| Law patches on an oscillator generation | take a new generation |
| `NONLINEAR_WALL=2` with `NONLINEAR_FORCED=1` | pins on the second-order trace bypass `prescribed_on_trace` and miss by 0.16; predates the stage; awaiting a decision |
| Device `exp_minus_one` above 0.02 per half step | kept as a known inaccuracy: the emitter reads 5.2e-5 at 500 steps; such scenes are bounded at 1e-4 (`docs/plan.md`, "Worth checking sometime") |
| f32 accumulation of `r` | kept: 5.4e-5 in a rebuilt `b` across an opened-ground handoff, bounded at 1e-4 |
| Constitutive compile time | 25.7 s of 28 s at 123,422 DOFs, 11× the time for 4× the DOFs. On the empty 2×2 domain at 0.02 with second-order walls, 30.7 s of 33 s is the outgoing trace's dense eigensolve, against 0.25 s with reflecting walls; whether the reported scene's time is the same is not checked. Under Maintenance in `docs/plan.md` |
| The device's outgoing-trace cap, 1,024 nodes | removed on 28 September: nothing on the device read it, and the device matches the reference at 1,166 and 1,236 trace nodes. The one limit left is the 65,535 workgroups a dispatch launches, one per trace node. See the log |
| Long-run drift past 1000 steps | as at Stage 10, plus: the soliton is chaotic (CPU twins part at about 0.9 per simulated second, the device reads 0.15 at 3000 steps) |
| Filter calibration | deferred to after the milestone, as at Stage 10 |
| Pulses, a true polariton, saturable and polynomial loss | blocked, in the gallery plan |
| A restored autosave shows no scene card | knowing its example needs a file-format change |

## Verification

- Every commit passed rustfmt, clippy `-D warnings`, the workspace tests
  in release, the release build and the wasm32 check; from the browser
  failure on, the Chrome shader compile too. CI runs the tests in release.
- **Device, at the close:** `canonical_gpu_long_run` at 400 steps on all
  36 gallery scenes, run on the M1 Max with an isolated HOME, all exit 0.
  Thirty-five are within 3.5e-6 in both `Q` and `b`; the self-sustained
  emitter reads Q 1.5e-5, b 3.0e-5, inside its 1e-4 bound for a rate past
  0.02. The temporal path carries 14 of them.
- **Device, during the stage:** 84 oscillator runs, 120 van der Pol runs,
  56 regression runs, six handoff modes, every device example with
  `NONLINEAR_WALL=1` and `=2`, and each gallery scene at 400 and 1000
  steps (3000 printed, not judged) on the day it landed.
- **Method:** every claim is a CPU test at edge 0.08, cross-checked at
  0.05 or 0.04 in exploration; the reference oracle was checked before the
  device was blamed (f64 with `Q` rounded to f32: 6.8e-8 at 100 steps); a
  perturbed CPU twin for the chaotic soliton.
- **Not checked by me:** the editor and the views on screen, pointer paths,
  the status sheen and the hover text, which the user reviewed as they
  landed. The user confirmed the crack gone.
