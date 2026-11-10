//! The numerical edge cases of the 11 October plan's section 5.2 on the CPU:
//! the step at and past its ceiling, steps far below it, amplitudes from
//! zero through the subnormals to past each law's bound, coefficient
//! contrast and anisotropy, narrow gaps and short edges, resources left
//! empty, and sums that cancel. A valid case must run finite and keep what
//! its law keeps; an invalid one must be refused with the state it had.
//!
//! The studies (C8.1 of the testing campaign, 2026-11-10) are kept ignored
//! and print what they measure; the checks at the end hold what they found.

mod support;
use support::*;

use funfern_app::canonical_gpu::{CanonicalGpuClock, CanonicalGpuPlan};
use funfern_app::document::ProbeSamplingPreset;
use funfern_app::topology_editor::{
    ClosedCurvePurpose, TopologyDocument, TopologyEditor, TopologyProbeTarget,
};
use funfern_app::topology_runtime::PreparedTopology;
use funfern_core::{
    BACKGROUND_REGION, CanonicalIntegrator, CanonicalStationaryFluxJob, CanonicalTemporalWaveState,
    CanonicalWaveState, DampingLaw, FieldLaw, LossChannel, Material, MaterialFrame,
    MaterialFrameAttachment, OuterBoundaryCondition, OuterBoundaryConditions, PeriodicCubicSpline,
    Point2, PointSource, RateLaw, RestoringLaw, RestoringLawValues, ScalarField, TimeDrive,
    TimeSignal, VolumeSource,
};
use std::time::Instant;

/// The default box, its point source off and every wall `walls`, edited by
/// `edit`.
fn document(
    walls: OuterBoundaryCondition,
    edit: impl FnOnce(&mut TopologyEditor) -> Result<(), String>,
) -> TopologyDocument {
    try_document(walls, edit).unwrap()
}

/// [`document`], an edit the editor refuses returned.
fn try_document(
    walls: OuterBoundaryCondition,
    edit: impl FnOnce(&mut TopologyEditor) -> Result<(), String>,
) -> Result<TopologyDocument, String> {
    let mut editor = TopologyEditor::default();
    edit(&mut editor)?;
    settle(&mut editor);
    let mut document = editor.document;
    let walls = OuterBoundaryConditions { sides: [walls; 4] };
    document.model.draft.outer_boundaries = walls;
    document.model.accepted.outer_boundaries = walls;
    Ok(document)
}

/// The background's material edited by `author`.
fn medium(editor: &mut TopologyEditor, author: impl FnOnce(&mut Material)) -> Result<(), String> {
    let mut material = editor.document.model.draft.materials[0].clone();
    author(&mut material);
    editor.update_material(material)
}

fn constant(value: f64) -> ScalarField {
    ScalarField::constant(value)
}

fn kerr(chi2: f64, bound: Option<f64>) -> FieldLaw {
    FieldLaw::Polynomial {
        chi1: constant(0.0),
        chi2: constant(chi2),
        amplitude_bound: bound.map(constant),
    }
}

/// The largest step `prepared` admits under the leapfrog's bound.
fn maximum(prepared: &PreparedTopology) -> f64 {
    match &prepared.canonical_temporal_operator {
        Some(temporal) => temporal.maximum_time_step(),
        None => prepared.canonical_operator.maximum_time_step(),
    }
}

/// At rest at `time_step` under `integrator`.
fn at_rest(
    prepared: &PreparedTopology,
    time_step: f64,
    integrator: CanonicalIntegrator,
) -> Result<Run, String> {
    match &prepared.canonical_temporal_operator {
        Some(temporal) => CanonicalTemporalWaveState::zero(temporal, time_step)
            .map(|state| Run::Temporal(state.with_integrator(integrator))),
        None => CanonicalWaveState::zero(&prepared.canonical_operator, time_step)
            .map(|state| Run::Fixed(state.with_integrator(integrator))),
    }
    .map_err(|error| error.to_string())
}

/// Struck by a field increment given node by node.
fn strike_with(
    run: &mut Run,
    prepared: &PreparedTopology,
    increment: &[f64],
) -> Result<(), String> {
    let forcing = &prepared.canonical_forcing;
    match (run, &prepared.canonical_temporal_operator) {
        (Run::Fixed(state), _) => state
            .apply_primary_pulse(&prepared.canonical_operator, forcing, increment)
            .map(|_| ()),
        (Run::Temporal(state), Some(temporal)) => state
            .apply_primary_pulse(temporal, forcing, increment)
            .map(|_| ()),
        (Run::Temporal(_), None) => unreachable!("a temporal state on a fixed generation"),
    }
    .map_err(|error| error.to_string())
}

/// The field at the nodes.
fn field(run: &Run, prepared: &PreparedTopology) -> Vec<f64> {
    read_field(run, prepared).unwrap()
}

fn read_field(run: &Run, prepared: &PreparedTopology) -> Result<Vec<f64>, String> {
    match (run, &prepared.canonical_temporal_operator) {
        (Run::Fixed(state), _) => state.primary_field(&prepared.canonical_operator),
        (Run::Temporal(state), Some(temporal)) => {
            temporal.primary_field_at(state.primary_flux(), state.time(), state.runtime())
        }
        (Run::Temporal(_), None) => unreachable!("a temporal state on a fixed generation"),
    }
    .map_err(|error| error.to_string())
}

fn time(run: &Run) -> f64 {
    match run {
        Run::Fixed(state) => state.time(),
        Run::Temporal(state) => state.time(),
    }
}

/// A rough field: every node its own value in `[−height, height]`, from a
/// seeded generator.
fn rough(nodes: usize, height: f64, seed: u64) -> Vec<f64> {
    let mut state = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
    (0..nodes)
        .map(|_| {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            height * ((state >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0)
        })
        .collect()
}

fn pulse_increment(prepared: &PreparedTopology, pulse: Pulse) -> Vec<f64> {
    prepared
        .canonical_operator
        .node_points()
        .iter()
        .map(|point| pulse.at(*point))
        .collect()
}

/// The stationary part's solve on `flux`, as the app runs it: its
/// iterations, its limit and whether it found one, and how long it took.
fn stationary_solve(prepared: &PreparedTopology, flux: Vec<Point2>) -> (String, f64) {
    let operator = prepared.canonical_operator.clone();
    let started = Instant::now();
    let mut job = match CanonicalStationaryFluxJob::new(operator, flux) {
        Ok(job) => job,
        Err(error) => return (format!("refused to start: {error}"), 0.0),
    };
    let (limit, tolerance) = (job.limit(), job.tolerance());
    let found = loop {
        if let Some(found) = job.advance(1 << 16) {
            break found;
        }
    };
    let seconds = started.elapsed().as_secs_f64();
    match found {
        Ok(found) => (
            format!(
                "{} of {limit} iterations to {tolerance:.1e}, stationary {:.2e} of the flux",
                found.iterations,
                found.energy / found.flux_energy.max(f64::MIN_POSITIVE)
            ),
            seconds,
        ),
        Err(error) => (format!("FAILED: {error}"), seconds),
    }
}

/// A complementary flux's stored energy.
fn flux_energy(operator: &funfern_core::CanonicalWaveOperator, flux: &[Point2]) -> f64 {
    operator
        .constitutive_samples()
        .iter()
        .zip(flux)
        .map(|(sample, value)| {
            0.5 * sample.integration_weight * value.dot(sample.complementary_inverse.apply(*value))
        })
        .sum()
}

/// A rough flux, every sample its own vector.
fn rough_flux(prepared: &PreparedTopology, seed: u64) -> Vec<Point2> {
    let samples = prepared
        .canonical_operator
        .complementary_degrees_of_freedom();
    let values = rough(2 * samples, 1.0, seed);
    values
        .chunks(2)
        .map(|pair| Point2::new(pair[0], pair[1]))
        .collect()
}

/// Steps `run` `steps` times, the energy read every `every`: the largest
/// departure from the first over each quarter of the run, or where a step
/// was refused and whether the refusal left the state's bits alone.
fn energy_quarters(
    run: &mut Run,
    prepared: &PreparedTopology,
    steps: usize,
    every: usize,
) -> Result<[(f64, f64); 4], String> {
    let first = run.energy(prepared)?;
    let mut quarters = [(f64::INFINITY, 0.0_f64); 4];
    for step in 0..steps {
        let before = run.bits();
        if let Err(error) = run.step(prepared) {
            let kept = run.bits() == before;
            return Err(format!(
                "refused at step {step}: {error}; the state {}",
                if kept { "kept to the bit" } else { "CHANGED" }
            ));
        }
        if step % every == 0 || step + 1 == steps {
            let energy = run.energy(prepared)?;
            if !energy.is_finite() {
                return Err(format!("energy {energy} at step {step}"));
            }
            let quarter = (4 * step / steps).min(3);
            let ratio = energy / first;
            quarters[quarter] = (
                quarters[quarter].0.min(ratio),
                quarters[quarter].1.max(ratio),
            );
        }
    }
    Ok(quarters)
}

fn quarters_text(quarters: Result<[(f64, f64); 4], String>) -> String {
    match quarters {
        Ok(q) => format!(
            "E/E0 by quarter [{:.4}, {:.4}] [{:.4}, {:.4}] [{:.4}, {:.4}] [{:.4}, {:.4}]",
            q[0].0, q[0].1, q[1].0, q[1].1, q[2].0, q[2].1, q[3].0, q[3].1
        ),
        Err(error) => error,
    }
}

// ---------------------------------------------------------------------------
// The step's ceiling.
// ---------------------------------------------------------------------------

fn ceiling_scenes() -> Vec<(&'static str, TopologyDocument)> {
    use OuterBoundaryCondition::*;
    vec![
        ("reflecting", document(Reflecting, |_| Ok(()))),
        (
            "second-order walls",
            document(SecondOrderOutgoing, |_| Ok(())),
        ),
        (
            "defocusing Kerr",
            document(Reflecting, |editor| {
                medium(editor, |m| m.mass_law.field = kerr(-0.5, Some(0.8)))
            }),
        ),
        (
            "sine-Gordon",
            document(Reflecting, |editor| {
                medium(editor, |m| {
                    m.restoring = RestoringLaw::SineGordon {
                        omega0: constant(3.0),
                    }
                })
            }),
        ),
        (
            "pumped",
            document(Reflecting, |editor| {
                medium(editor, |m| {
                    m.mass_law.drive = TimeDrive::ParametricPump {
                        depth: constant(0.3),
                        frequency_hz: constant(1.0),
                        phase_radians: constant(0.0),
                    }
                })
            }),
        ),
    ]
}

/// Each integrator at its ceiling and one float past it, a smooth and a
/// rough field run at the ceiling, and a state switched to the leapfrog
/// above the leapfrog's bound.
#[test]
#[ignore = "C8.1's study: cargo test --release -p funfern-app --test edge_cases -- --ignored --nocapture study_ceiling"]
fn study_ceiling() {
    let steps: usize = std::env::var("EDGE_STEPS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(20_000);
    let wanted = std::env::var("EDGE_SCENE").ok();
    for (name, document) in ceiling_scenes() {
        if wanted
            .as_deref()
            .is_some_and(|wanted| !wanted.split(',').any(|w| w == name))
        {
            continue;
        }
        let (_, _, prepared) = prepare_at(&document, 0.1).unwrap();
        let maximum = maximum(&prepared);
        let nodes = prepared.canonical_operator.degrees_of_freedom();
        let domain = prepared.bundle.authored.geometry.domain;
        eprintln!(
            "STUDY ceiling {name}: {nodes} nodes, maximum {maximum:.6e}, recommended {:.4} of it",
            prepared.recommended_time_step() / maximum
        );
        for integrator in [
            CanonicalIntegrator::Leapfrog,
            CanonicalIntegrator::FourthOrder,
        ] {
            let ceiling = integrator.ceiling() * maximum;
            let past = at_rest(&prepared, ceiling.next_up(), integrator);
            let mut past_step = "no state".to_string();
            if let Ok(mut run) = at_rest(&prepared, ceiling.next_up(), integrator) {
                strike_with(
                    &mut run,
                    &prepared,
                    &pulse_increment(&prepared, Pulse::off_centre(domain, 1.0)),
                )
                .unwrap();
                let before = run.bits();
                past_step = match run.step(&prepared) {
                    Ok(()) => "STEPPED".into(),
                    Err(error) => format!(
                        "refused ({error}), the state {}",
                        if run.bits() == before {
                            "kept"
                        } else {
                            "CHANGED"
                        }
                    ),
                };
            }
            eprintln!(
                "  {integrator:?}: one float past the ceiling {}; its step {past_step}",
                match past {
                    Ok(_) => "BUILT".to_string(),
                    Err(error) => format!("refused ({error})"),
                }
            );
            for (shape, increment) in [
                (
                    "smooth",
                    pulse_increment(&prepared, Pulse::off_centre(domain, 1.0)),
                ),
                ("rough", rough(nodes, 0.1, 7)),
            ] {
                let mut run = match at_rest(&prepared, ceiling, integrator) {
                    Ok(run) => run,
                    Err(error) => {
                        eprintln!("  {integrator:?} {shape} at the ceiling: REFUSED {error}");
                        continue;
                    }
                };
                strike_with(&mut run, &prepared, &increment).unwrap();
                let first = run.energy(&prepared).unwrap();
                let started = Instant::now();
                let quarters = energy_quarters(&mut run, &prepared, steps, 10);
                let last = run.energy(&prepared).unwrap_or(f64::NAN) / first;
                // The same span at an eighth of the step, where the energy
                // is not the law's to keep.
                let reference = if shape == "smooth" && std::env::var("EDGE_REFERENCE").is_ok() {
                    let mut fine = at_rest(&prepared, ceiling / 8.0, integrator).unwrap();
                    strike_with(&mut fine, &prepared, &increment).unwrap();
                    for _ in 0..8 * steps {
                        fine.step(&prepared).unwrap();
                    }
                    format!(
                        ", an eighth of the step ends at {:.4}",
                        fine.energy(&prepared).unwrap() / first
                    )
                } else {
                    String::new()
                };
                eprintln!(
                    "  {integrator:?} {shape} at the ceiling, {steps} steps: {}, ends at {last:.4}{reference} ({:.1} s)",
                    quarters_text(quarters),
                    started.elapsed().as_secs_f64()
                );
            }
        }
        // Built under the fourth-order step between the two bounds, then
        // switched to the leapfrog.
        let between = 1.1 * maximum;
        if let Ok(run) = at_rest(&prepared, between, CanonicalIntegrator::FourthOrder) {
            let mut run = match run {
                Run::Fixed(state) => {
                    Run::Fixed(state.with_integrator(CanonicalIntegrator::Leapfrog))
                }
                Run::Temporal(state) => {
                    Run::Temporal(state.with_integrator(CanonicalIntegrator::Leapfrog))
                }
            };
            strike_with(
                &mut run,
                &prepared,
                &pulse_increment(&prepared, Pulse::off_centre(domain, 1.0)),
            )
            .unwrap();
            let before = run.bits();
            let outcome = run.step(&prepared);
            eprintln!(
                "  switched to the leapfrog at 1.1 of its bound: {}, the state {}",
                match outcome {
                    Ok(()) => "STEPPED".to_string(),
                    Err(error) => format!("refused ({error})"),
                },
                if run.bits() == before {
                    "kept"
                } else {
                    "moved"
                }
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Steps far below the ceiling, and a late clock.
// ---------------------------------------------------------------------------

/// The reflecting box and the pumped one at the recommended step and at
/// 1e-2, 1e-4 and 1e-6 of it over the same span, against a step 64 times
/// smaller than the recommended one; and a pumped run started at 1e6 s
/// against one started at zero with the pump's phase moved to match.
#[test]
#[ignore = "C8.1's study: cargo test --release -p funfern-app --test edge_cases -- --ignored --nocapture study_small_steps"]
fn study_small_steps() {
    let span = 0.05;
    for (name, document) in ceiling_scenes()
        .into_iter()
        .filter(|(name, _)| ["reflecting", "pumped", "second-order walls"].contains(name))
    {
        let (_, _, prepared) = prepare_at(&document, 0.2).unwrap();
        let domain = prepared.bundle.authored.geometry.domain;
        let recommended = prepared.recommended_time_step();
        let increment = pulse_increment(&prepared, Pulse::off_centre(domain, 1.0));
        let run_at = |share: f64| -> (Vec<f64>, f64, f64, f64, f64) {
            let steps = (span / (share * recommended)).ceil() as u64;
            let dt = span / steps as f64;
            let mut run = at_rest(&prepared, dt, CanonicalIntegrator::FourthOrder).unwrap();
            strike_with(&mut run, &prepared, &increment).unwrap();
            let first = run.energy(&prepared).unwrap();
            let started = Instant::now();
            for _ in 0..steps {
                run.step(&prepared).unwrap();
            }
            let energy = run.energy(&prepared).unwrap();
            (
                field(&run, &prepared),
                energy / first - 1.0,
                time(&run) - span,
                started.elapsed().as_secs_f64(),
                dt,
            )
        };
        let (reference, ..) = run_at(1.0 / 64.0);
        let norm = reference.iter().map(|u| u * u).sum::<f64>().sqrt();
        for share in [1.0, 0.25, 1.0e-2, 1.0e-4, 1.0e-6] {
            let (ended, energy, clock, seconds, dt) = run_at(share);
            let apart = ended
                .iter()
                .zip(&reference)
                .map(|(u, r)| (u - r) * (u - r))
                .sum::<f64>()
                .sqrt();
            eprintln!(
                "STUDY small steps {name} at {share:e} of the recommended step ({dt:.3e}): field {:.2e} from the reference, energy {energy:+.2e}, clock {clock:+.2e} ({seconds:.1} s)",
                apart / norm
            );
        }
    }
    // A late clock: the pump at 1e6 s against the pump at zero, its phase
    // moved by what the clock carries.
    let start = 1.0e6_f64;
    let frequency = 1.0_f64;
    let pumped = |phase: f64| {
        document(OuterBoundaryCondition::Reflecting, |editor| {
            medium(editor, |m| {
                m.mass_law.drive = TimeDrive::ParametricPump {
                    depth: constant(0.3),
                    frequency_hz: constant(frequency),
                    phase_radians: constant(phase),
                }
            })
        })
    };
    let moved = (std::f64::consts::TAU * frequency * start).rem_euclid(std::f64::consts::TAU);
    let late = pumped(0.0);
    let early = pumped(moved);
    let (_, _, late) = prepare_at(&late, 0.2).unwrap();
    let (_, _, early) = prepare_at(&early, 0.2).unwrap();
    let dt = late.recommended_time_step();
    let domain = late.bundle.authored.geometry.domain;
    let run_from = |prepared: &PreparedTopology, t0: f64| {
        let temporal = prepared.canonical_temporal_operator.as_ref().unwrap();
        let base = temporal.base();
        let state = CanonicalTemporalWaveState::new_at(
            temporal,
            dt,
            vec![0.0; base.degrees_of_freedom()],
            vec![Point2::default(); base.complementary_degrees_of_freedom()],
            t0,
        )
        .unwrap();
        let mut run = Run::Temporal(state);
        strike_with(
            &mut run,
            prepared,
            &pulse_increment(prepared, Pulse::off_centre(domain, 1.0)),
        )
        .unwrap();
        for _ in 0..2_000 {
            run.step(prepared).unwrap();
        }
        (field(&run, prepared), time(&run) - t0)
    };
    let (at_late, late_span) = run_from(&late, start);
    let (at_early, early_span) = run_from(&early, 0.0);
    let norm = at_early.iter().map(|u| u * u).sum::<f64>().sqrt();
    let apart = at_late
        .iter()
        .zip(&at_early)
        .map(|(a, b)| (a - b) * (a - b))
        .sum::<f64>()
        .sqrt();
    eprintln!(
        "STUDY late clock: the pump from {start:e} s against from zero, 2000 steps: field {:.2e} apart, spans {late_span:.12} and {early_span:.12}",
        apart / norm
    );
}

// ---------------------------------------------------------------------------
// Amplitudes.
// ---------------------------------------------------------------------------

/// A law the amplitude sweep runs, and the field amplitude past which it is
/// meant to refuse, if it has one.
struct Law {
    name: &'static str,
    author: Box<dyn Fn(&mut Material)>,
    bound: Option<f64>,
}

fn laws() -> Vec<Law> {
    let loss = |rate: f64, law: RateLaw| LossChannel {
        base_rate: constant(rate),
        law: DampingLaw {
            rate: law,
            drive: TimeDrive::None,
            gate: None,
        },
    };
    vec![
        Law {
            name: "linear",
            author: Box::new(|_| {}),
            bound: None,
        },
        Law {
            name: "focusing Kerr on the mass",
            author: Box::new(|m| m.mass_law.field = kerr(0.5, None)),
            bound: None,
        },
        Law {
            name: "defocusing Kerr on the mass",
            author: Box::new(|m| m.mass_law.field = kerr(-0.5, Some(0.8))),
            bound: Some(0.8),
        },
        Law {
            name: "focusing Kerr on the stiffness",
            author: Box::new(|m| m.stiffness_law.field = kerr(0.5, None)),
            bound: None,
        },
        Law {
            name: "defocusing Kerr on the stiffness",
            author: Box::new(|m| m.stiffness_law.field = kerr(-0.5, Some(0.8))),
            bound: Some(0.8),
        },
        Law {
            name: "quadratic on the mass",
            author: Box::new(|m| {
                m.mass_law.field = FieldLaw::Polynomial {
                    chi1: constant(0.3),
                    chi2: constant(0.0),
                    amplitude_bound: Some(constant(1.5)),
                }
            }),
            bound: Some(1.5),
        },
        Law {
            name: "saturable mass",
            author: Box::new(|m| {
                m.mass_law.field = FieldLaw::Saturable {
                    chi: constant(0.8),
                    saturation: constant(0.5),
                }
            }),
            bound: None,
        },
        Law {
            name: "defocusing saturable mass",
            author: Box::new(|m| {
                m.mass_law.field = FieldLaw::Saturable {
                    chi: constant(-0.8),
                    saturation: constant(0.5),
                }
            }),
            bound: None,
        },
        Law {
            name: "saturable stiffness",
            author: Box::new(|m| {
                m.stiffness_law.field = FieldLaw::Saturable {
                    chi: constant(0.8),
                    saturation: constant(0.5),
                }
            }),
            bound: None,
        },
        Law {
            name: "Klein-Gordon",
            author: Box::new(|m| {
                m.restoring = RestoringLaw::KleinGordon {
                    omega0: constant(3.0),
                }
            }),
            bound: None,
        },
        Law {
            name: "sine-Gordon",
            author: Box::new(|m| {
                m.restoring = RestoringLaw::SineGordon {
                    omega0: constant(3.0),
                }
            }),
            bound: None,
        },
        Law {
            name: "phi4",
            author: Box::new(|m| {
                m.restoring = RestoringLaw::Phi4 {
                    lambda: constant(4.0),
                    amplitude_bound: constant(2.0),
                }
            }),
            bound: Some(2.0),
        },
        Law {
            name: "van der Pol",
            author: Box::new(move |m| {
                m.restoring = RestoringLaw::KleinGordon {
                    omega0: constant(3.0),
                };
                m.magnetic_loss = Some(loss(
                    0.5,
                    RateLaw::VanDerPol {
                        threshold: constant(0.5),
                        amplitude_bound: constant(2.0),
                    },
                ));
            }),
            bound: Some(2.0),
        },
        Law {
            name: "pumped mass",
            author: Box::new(|m| {
                m.mass_law.drive = TimeDrive::ParametricPump {
                    depth: constant(0.3),
                    frequency_hz: constant(1.0),
                    phase_radians: constant(0.0),
                }
            }),
            bound: None,
        },
    ]
}

/// Each law struck by a pulse whose field peaks at amplitudes from zero
/// through the subnormals to 1e300, and at shares of its bound around it,
/// run 2 s at the recommended step: refused or not, at what step, the state
/// kept; finite; the energy's departure; and below 1e-6 the field over its
/// amplitude against the same medium's at 1e-100.
#[test]
#[ignore = "C8.1's study: cargo test --release -p funfern-app --test edge_cases -- --ignored --nocapture study_amplitudes"]
fn study_amplitudes() {
    let wanted = std::env::var("EDGE_LAW").ok();
    for law in laws() {
        if wanted
            .as_deref()
            .is_some_and(|wanted| !law.name.contains(wanted))
        {
            continue;
        }
        let document = document(OuterBoundaryCondition::Reflecting, |editor| {
            medium(editor, |m| (law.author)(m))
        });
        let prepared = match prepare_at(&document, 0.2) {
            Ok((_, _, prepared)) => prepared,
            Err(error) => {
                eprintln!("STUDY amplitudes {}: REFUSED ({error})", law.name);
                continue;
            }
        };
        let dt = prepared.recommended_time_step();
        let steps = (2.0 / dt).ceil() as usize;
        let domain = prepared.bundle.authored.geometry.domain;
        let shape = Pulse {
            height: 1.0,
            ..Pulse::off_centre(domain, 1.0)
        };
        let unit = pulse_increment(&prepared, shape);
        let peak = unit.iter().fold(0.0_f64, |m, v| m.max(*v));
        eprintln!(
            "STUDY amplitudes {}: {} nodes, step {dt:.4e} ({:.3} of the maximum), {steps} steps",
            law.name,
            prepared.canonical_operator.degrees_of_freedom(),
            dt / maximum(&prepared)
        );
        let mut amplitudes = vec![
            0.0,
            f64::from_bits(1),
            1.0e-310,
            1.0e-300,
            1.0e-250,
            1.0e-200,
            1.0e-160,
            1.0e-150,
            1.0e-100,
            1.0e-30,
            1.0e-6,
            1.0e-2,
            0.5,
            1.0,
            3.0,
            10.0,
            1.0e3,
            1.0e10,
            1.0e100,
            1.0e300,
        ];
        if let Some(bound) = law.bound {
            amplitudes.retain(|a| *a < 0.5 * bound);
            amplitudes.extend([0.5, 0.9, 0.99, 1.0, 1.01, 2.0].map(|share| share * bound));
        }
        // The medium's own run at 1e-100, deep in its linear regime and far
        // from the subnormals, per unit of amplitude: the shape every small
        // amplitude must keep.
        let small_shape = {
            let tiny = 1.0e-100;
            let increment = unit.iter().map(|v| v * tiny / peak).collect::<Vec<_>>();
            let mut run = at_rest(&prepared, dt, CanonicalIntegrator::FourthOrder).unwrap();
            strike_with(&mut run, &prepared, &increment).unwrap();
            for _ in 0..steps {
                run.step(&prepared).unwrap();
            }
            field(&run, &prepared)
                .iter()
                .map(|u| u / tiny)
                .collect::<Vec<_>>()
        };
        for amplitude in amplitudes {
            // A pulse is a field increment: its peak brings the field at
            // rest to `amplitude`.
            let height = amplitude / peak;
            let increment = unit.iter().map(|v| v * height).collect::<Vec<_>>();
            let mut run = at_rest(&prepared, dt, CanonicalIntegrator::FourthOrder).unwrap();
            let before = run.bits();
            if let Err(error) = strike_with(&mut run, &prepared, &increment) {
                eprintln!(
                    "  {amplitude:e}: the pulse refused ({error}), the state {}",
                    if run.bits() == before {
                        "kept"
                    } else {
                        "CHANGED"
                    }
                );
                continue;
            }
            let first = run.energy(&prepared);
            let mut outcome = String::new();
            let mut largest_departure = 0.0_f64;
            for step in 0..steps {
                let before = run.bits();
                if let Err(error) = run.step(&prepared) {
                    outcome = format!(
                        "refused at step {step} ({error}), the state {}",
                        if run.bits() == before {
                            "kept"
                        } else {
                            "CHANGED"
                        }
                    );
                    break;
                }
                if step % 20 == 0
                    && let (Ok(first), Ok(energy)) = (&first, run.energy(&prepared))
                    && *first > 0.0
                {
                    largest_departure = largest_departure.max((energy / first - 1.0).abs());
                }
            }
            let ended = match read_field(&run, &prepared) {
                Ok(ended) => ended,
                Err(error) => {
                    eprintln!("  {amplitude:e}: {outcome}; the field UNREADABLE ({error})");
                    continue;
                }
            };
            let finite = ended.iter().all(|u| u.is_finite())
                && run.primary().iter().all(|q| q.is_finite())
                && run
                    .complementary()
                    .iter()
                    .all(|b| b.x.is_finite() && b.y.is_finite());
            let largest = ended.iter().fold(0.0_f64, |m, u| m.max(u.abs()));
            let zero = ended.iter().all(|u| *u == 0.0);
            let shape_gap = if (1.0e-300..=1.0e-6).contains(&amplitude) && amplitude != 1.0e-100 {
                let norm = small_shape.iter().map(|u| u * u).sum::<f64>().sqrt();
                let gap = ended
                    .iter()
                    .zip(&small_shape)
                    .map(|(u, l)| (u / amplitude - l).powi(2))
                    .sum::<f64>()
                    .sqrt();
                format!(", shape {:.2e} from its own at 1e-100", gap / norm)
            } else {
                String::new()
            };
            eprintln!(
                "  {amplitude:e}: {}{}; finite {finite}, largest field {largest:.3e}{}, energy {}{}",
                if outcome.is_empty() { "ran" } else { "" },
                outcome,
                if zero { " (all zero)" } else { "" },
                match first {
                    Ok(first) => format!("{first:.3e}, departed {largest_departure:.2e}"),
                    Err(error) => format!("unreadable ({error})"),
                },
                shape_gap
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Contrast and anisotropy.
// ---------------------------------------------------------------------------

/// A disc of radius 0.4 at the middle, of density `density` and stiffness
/// `stiffness` over the background's unit medium.
fn inclusion(density: f64, stiffness: f64) -> TopologyDocument {
    document(OuterBoundaryCondition::Reflecting, |editor| {
        let id = editor.add_material()?;
        let mut material = editor
            .document
            .model
            .draft
            .materials
            .iter()
            .find(|m| m.id == id)
            .unwrap()
            .clone();
        material.mass_density = constant(density);
        material.stiffness = constant(stiffness);
        editor.update_material(material)?;
        editor.create_closed_curve(
            PeriodicCubicSpline::rounded(Point2::new(0.0, 0.0), 0.4),
            ClosedCurvePurpose::Subdomain { material: id },
        )?;
        Ok(())
    })
}

/// The whole box at axis ratio `ratio`, its frame turned 0.5 rad.
fn anisotropic(ratio: f64) -> TopologyDocument {
    document(OuterBoundaryCondition::Reflecting, |editor| {
        medium(editor, |m| m.axis_ratio = constant(ratio))?;
        editor.set_region_frame(
            BACKGROUND_REGION,
            MaterialFrame {
                origin: Point2::new(0.0, 0.0),
                angle_radians: 0.5,
                attachment: MaterialFrameAttachment::World,
            },
        )
    })
}

/// One conditioning case: its step, a struck run's energy over 2 s, the
/// stationary solve on the run's flux and on a rough one, and the oracle's.
fn conditioning_case(label: &str, document: &TopologyDocument, edge: f64) {
    let started = Instant::now();
    let (_, _, prepared) = match prepare_at(document, edge) {
        Ok(prepared) => prepared,
        Err(error) => {
            eprintln!("STUDY conditioning {label} at {edge}: REFUSED {error}");
            return;
        }
    };
    let prepared_in = started.elapsed().as_secs_f64();
    let dt = prepared.recommended_time_step();
    let domain = prepared.bundle.authored.geometry.domain;
    let steps = ((2.0 / dt).ceil() as usize).min(40_000);
    let mut run = at_rest(&prepared, dt, CanonicalIntegrator::FourthOrder).unwrap();
    strike_with(
        &mut run,
        &prepared,
        &pulse_increment(&prepared, Pulse::off_centre(domain, 1.0)),
    )
    .unwrap();
    let quarters = energy_quarters(&mut run, &prepared, steps, 20.max(steps / 200));
    let (own, own_seconds) = stationary_solve(&prepared, run.complementary().to_vec());
    let (rough, rough_seconds) = stationary_solve(&prepared, rough_flux(&prepared, 3));
    // Printed before the oracle, which at a ratio of 1e6 and edge 0.025 ran
    // for hours: it allows forty times the node count of iterations.
    eprintln!(
        "STUDY conditioning {label} at {edge}: {} nodes (prepared in {prepared_in:.1} s), maximum step {:.3e}, {steps} steps: {}; the run's flux: {own} ({own_seconds:.1} s); a rough flux: {rough} ({rough_seconds:.1} s)",
        prepared.canonical_operator.degrees_of_freedom(),
        maximum(&prepared),
        quarters_text(quarters)
    );
    let operator = &prepared.canonical_operator;
    let share = |flux: &[Point2]| {
        operator
            .stationary_complementary_component(flux)
            .map(|stationary| {
                format!(
                    "{:.2e}",
                    flux_energy(operator, &stationary) / flux_energy(operator, flux)
                )
            })
            .unwrap_or_else(|error| format!("FAILED {error}"))
    };
    eprintln!(
        "STUDY conditioning {label} at {edge}: the oracle found {} on the run's flux and {} on the rough one",
        share(run.complementary()),
        share(&rough_flux(&prepared, 3))
    );
}

/// Inclusions of contrast 1 to 1e6 in density and stiffness together (the
/// same speed, the impedance stepped) and in stiffness alone (the speed
/// stepped), and the box at axis ratios 1 to 1000, at edges 0.1 and 0.05.
#[test]
#[ignore = "C8.1's study: cargo test --release -p funfern-app --test edge_cases -- --ignored --nocapture study_conditioning"]
fn study_conditioning() {
    let edges: Vec<f64> = std::env::var("EDGE_EDGES")
        .ok()
        .map(|v| v.split(',').map(|e| e.parse().unwrap()).collect())
        .unwrap_or(vec![0.1, 0.05]);
    let only = std::env::var("EDGE_KIND").ok();
    for &edge in &edges {
        if only.as_deref().is_none_or(|k| k == "impedance") {
            for contrast in [1.0, 25.0, 100.0, 1.0e3, 1.0e4, 1.0e6] {
                conditioning_case(
                    &format!("impedance {contrast:e}"),
                    &inclusion(contrast, contrast),
                    edge,
                );
            }
        }
        if only.as_deref().is_none_or(|k| k == "speed") {
            for contrast in [25.0, 100.0, 1.0e3, 1.0e4, 1.0e6] {
                conditioning_case(
                    &format!("stiffness {contrast:e}"),
                    &inclusion(1.0, contrast),
                    edge,
                );
                conditioning_case(
                    &format!("compliance {contrast:e}"),
                    &inclusion(1.0, 1.0 / contrast),
                    edge,
                );
            }
        }
        if only.as_deref().is_none_or(|k| k == "axis") {
            let ratios: Vec<f64> = std::env::var("EDGE_RATIOS")
                .ok()
                .map(|v| v.split(',').map(|r| r.parse().unwrap()).collect())
                .unwrap_or(vec![1.0, 4.0, 8.0, 16.0, 64.0, 256.0, 1.0e3]);
            for ratio in ratios {
                conditioning_case(&format!("axis ratio {ratio}"), &anisotropic(ratio), edge);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Narrow gaps, short edges.
// ---------------------------------------------------------------------------

/// The box with `holes` cut, or the editor's refusal.
fn holes(holes: Vec<PeriodicCubicSpline>) -> Result<TopologyDocument, String> {
    try_document(OuterBoundaryCondition::Reflecting, move |editor| {
        for spline in holes {
            editor.create_closed_curve(spline, ClosedCurvePurpose::Hole)?;
        }
        Ok(())
    })
}

/// The extreme `x` a closed curve reaches, sampled densely: a B-spline's
/// curve stands inside its controls.
fn reach_x(spline: &PeriodicCubicSpline, sign: f64) -> f64 {
    let period = spline.period();
    (0..8192)
        .map(|i| sign * spline.evaluate(period * i as f64 / 8192.0).x)
        .fold(f64::NEG_INFINITY, f64::max)
        * sign
}

/// A rounded hole of radius `radius` whose curve comes `gap` from the line
/// `x = wall`, on the side `sign` points to.
fn disc_off(wall: f64, sign: f64, gap: f64, radius: f64) -> PeriodicCubicSpline {
    let probe = PeriodicCubicSpline::rounded(Point2::new(0.0, 0.0), radius);
    let reach = reach_x(&probe, sign).abs();
    PeriodicCubicSpline::rounded(Point2::new(wall - sign * (reach + gap), 0.0), radius)
}

/// A polygon hole: the square `[0.4, 1 − gap] × [−0.3, 0.3]`, a channel
/// `gap` wide along the right wall.
fn channel(gap: f64) -> PeriodicCubicSpline {
    PeriodicCubicSpline::polygon(vec![
        Point2::new(0.4, -0.3),
        Point2::new(1.0 - gap, -0.3),
        Point2::new(1.0 - gap, 0.3),
        Point2::new(0.4, 0.3),
    ])
    .unwrap()
}

/// A pentagonal hole one of whose edges is `length` long.
fn short_edge(length: f64) -> PeriodicCubicSpline {
    PeriodicCubicSpline::polygon(vec![
        Point2::new(-0.3, -0.25),
        Point2::new(0.3, -0.25),
        Point2::new(0.35, 0.1),
        Point2::new(0.0, 0.3),
        Point2::new(-0.35, 0.1 + length),
        Point2::new(-0.35, 0.1),
    ])
    .unwrap()
}

/// A rounded hole's controls with the first two drawn `gap` apart.
fn pinched(gap: f64) -> PeriodicCubicSpline {
    let mut controls = (0..8)
        .map(|i| {
            let angle = i as f64 * std::f64::consts::TAU / 8.0;
            Point2::new(0.3 * angle.cos(), 0.3 * angle.sin())
        })
        .collect::<Vec<_>>();
    let (a, b) = (controls[0], controls[1]);
    let middle = (a + b) * 0.5;
    let along = (b - a) * (1.0 / (b - a).norm());
    controls[0] = middle - along * (0.5 * gap);
    controls[1] = middle + along * (0.5 * gap);
    PeriodicCubicSpline::uniform(controls).unwrap()
}

/// The area a closed curve encloses, by the shoelace over dense samples.
fn enclosed_area(spline: &PeriodicCubicSpline) -> f64 {
    let samples = 1 << 16;
    let points = (0..samples)
        .map(|k| spline.evaluate(spline.period() * k as f64 / samples as f64))
        .collect::<Vec<_>>();
    0.5 * points
        .iter()
        .zip(points.iter().cycle().skip(1))
        .map(|(a, b)| a.x * b.y - b.x * a.y)
        .sum::<f64>()
        .abs()
}

fn geometry_case(label: &str, cut: Vec<PeriodicCubicSpline>, edge: f64, reference_step: f64) {
    let started = Instant::now();
    let expected_area = 4.0 - cut.iter().map(enclosed_area).sum::<f64>();
    let document = holes(cut);
    let document = match &document {
        Ok(document) => document,
        Err(error) => {
            eprintln!("STUDY geometry {label}: the editor REFUSED it ({error})");
            return;
        }
    };
    let (_, _, prepared) = match prepare_at(document, edge) {
        Ok(prepared) => prepared,
        Err(error) => {
            eprintln!("STUDY geometry {label}: REFUSED {error}");
            return;
        }
    };
    let prepared_in = started.elapsed().as_secs_f64();
    let mesh = &prepared.mesh;
    let shortest = mesh
        .triangles
        .iter()
        .flat_map(|triangle| {
            let v = triangle.vertices.map(|i| mesh.vertices[i].point);
            [
                (v[0] - v[1]).norm(),
                (v[1] - v[2]).norm(),
                (v[2] - v[0]).norm(),
            ]
        })
        .fold(f64::INFINITY, f64::min);
    let meshed_area = mesh
        .triangles
        .iter()
        .map(|triangle| {
            let [a, b, c] = triangle.vertices.map(|i| mesh.vertices[i].point);
            0.5 * ((b.x - a.x) * (c.y - a.y) - (c.x - a.x) * (b.y - a.y)).abs()
        })
        .sum::<f64>();
    let dt = prepared.recommended_time_step();
    let domain = prepared.bundle.authored.geometry.domain;
    let steps = ((2.0 / dt).ceil() as usize).min(40_000);
    let mut run = at_rest(&prepared, dt, CanonicalIntegrator::FourthOrder).unwrap();
    let pulse = Pulse {
        centre: Point2::new(-0.6, -0.6),
        ..Pulse::off_centre(domain, 1.0)
    };
    strike_with(&mut run, &prepared, &pulse_increment(&prepared, pulse)).unwrap();
    let quarters = energy_quarters(&mut run, &prepared, steps, 20.max(steps / 200));
    let (own, _) = stationary_solve(&prepared, run.complementary().to_vec());
    let (rough, _) = stationary_solve(&prepared, rough_flux(&prepared, 3));
    eprintln!(
        "STUDY geometry {label}: {} triangles, worst angle {:.2}°, shortest edge {shortest:.2e}, area {:+.1e} off the authored (prepared in {prepared_in:.1} s); step {:.3} of the empty box's, {steps} steps: {}; the run's flux {own}; a rough flux {rough}",
        mesh.triangles.len(),
        mesh.quality.minimum_angle_degrees,
        meshed_area / expected_area - 1.0,
        dt / reference_step,
        quarters_text(quarters)
    );
}

/// A hole of radius 0.3 from 0.2 to 1e-6 off the right wall, two holes as
/// close to each other, and a hole two of whose controls nearly coincide.
#[test]
#[ignore = "C8.1's study: cargo test --release -p funfern-app --test edge_cases -- --ignored --nocapture study_geometry"]
fn study_geometry() {
    let edge = 0.1;
    let empty = document(OuterBoundaryCondition::Reflecting, |_| Ok(()));
    let (_, _, empty) = prepare_at(&empty, edge).unwrap();
    let reference = empty.recommended_time_step();
    let only = std::env::var("EDGE_KIND").ok();
    // The editor joins curves closer than 1e-4 of the domain, 2e-4 here.
    let gaps = [0.2, 0.05, 1.0e-2, 1.0e-3, 3.0e-4, 1.0e-4, 1.0e-6];
    let wanted = |kind: &str| only.as_deref().is_none_or(|k| k == kind);
    for gap in gaps.into_iter().filter(|_| wanted("disc")) {
        geometry_case(
            &format!("a disc {gap:e} off the wall"),
            vec![disc_off(1.0, 1.0, gap, 0.3)],
            edge,
            reference,
        );
    }
    for gap in gaps.into_iter().filter(|_| wanted("channel")) {
        geometry_case(
            &format!("a channel {gap:e} wide"),
            vec![channel(gap)],
            edge,
            reference,
        );
    }
    for gap in gaps.into_iter().filter(|_| wanted("discs")) {
        geometry_case(
            &format!("two discs {gap:e} apart"),
            vec![
                disc_off(-0.5 * gap, 1.0, 0.0, 0.3),
                disc_off(0.5 * gap, -1.0, 0.0, 0.3),
            ],
            edge,
            reference,
        );
    }
    for length in [1.0e-2, 1.0e-3, 1.0e-4, 1.0e-6]
        .into_iter()
        .filter(|_| wanted("short"))
    {
        geometry_case(
            &format!("a polygon with an edge {length:e} long"),
            vec![short_edge(length)],
            edge,
            reference,
        );
    }
    for gap in [1.0e-2, 1.0e-4, 1.0e-6, 1.0e-9]
        .into_iter()
        .filter(|_| wanted("pinched"))
    {
        geometry_case(
            &format!("a disc with two controls {gap:e} apart"),
            vec![pinched(gap)],
            edge,
            reference,
        );
    }
}

// ---------------------------------------------------------------------------
// Resources left empty.
// ---------------------------------------------------------------------------

/// Every combination of no source, the point source and a volume source;
/// no probe, a point, a segment and a disc; reflecting, first- and
/// second-order walls; no history, a loss and a restoring law; and a fixed
/// and a driven generation: prepared, its device plan compiled, and 40 steps
/// on the CPU.
#[test]
#[ignore = "C8.1's study: cargo test --release -p funfern-app --test edge_cases -- --ignored --nocapture study_empty"]
fn study_empty() {
    use OuterBoundaryCondition::*;
    let mut failures = 0;
    let mut cases = 0;
    for source in ["none", "point", "volume"] {
        for probe in ["none", "point", "segment", "disc"] {
            for walls in [Reflecting, FirstOrderOutgoing, SecondOrderOutgoing] {
                for history in ["none", "loss", "restoring"] {
                    for driven in [false, true] {
                        cases += 1;
                        let label = format!(
                            "source {source}, probe {probe}, walls {walls:?}, history {history}, {}",
                            if driven { "driven" } else { "fixed" }
                        );
                        let document = document(walls, |editor| {
                            match source {
                                "point" => editor.set_point_source(PointSource {
                                    enabled: true,
                                    ..PointSource::default()
                                })?,
                                "volume" => editor.set_volume_source(
                                    BACKGROUND_REGION,
                                    Some(VolumeSource {
                                        region: BACKGROUND_REGION,
                                        enabled: true,
                                        profile: ScalarField::formula(
                                            "exp(-((x - 0.3)^2 + y^2) / 0.05)",
                                        )
                                        .map_err(|e| e.to_string())?,
                                        parameters: vec![],
                                        signal: TimeSignal::harmonic(0.0, 1.0, 1.5, 0.0),
                                    }),
                                )?,
                                _ => {}
                            }
                            let target = match probe {
                                "point" => Some(TopologyProbeTarget::Point(Point2::new(0.2, 0.1))),
                                "segment" => Some(TopologyProbeTarget::Segment {
                                    start: Point2::new(-0.5, -0.2),
                                    end: Point2::new(0.5, 0.3),
                                    preset: ProbeSamplingPreset::Medium,
                                }),
                                "disc" => Some(TopologyProbeTarget::AreaDisk {
                                    center: Point2::new(0.1, -0.2),
                                    radius: 0.3,
                                }),
                                _ => None,
                            };
                            if let Some(target) = target {
                                editor.create_probe("probe".into(), [200, 80, 80], target)?;
                            }
                            medium(editor, |m| {
                                match history {
                                    "loss" => {
                                        m.magnetic_loss = Some(LossChannel {
                                            base_rate: constant(0.2),
                                            law: DampingLaw {
                                                rate: RateLaw::Constant,
                                                drive: TimeDrive::None,
                                                gate: None,
                                            },
                                        })
                                    }
                                    "restoring" => {
                                        m.restoring = RestoringLaw::KleinGordon {
                                            omega0: constant(2.0),
                                        }
                                    }
                                    _ => {}
                                }
                                if driven {
                                    m.mass_law.drive = TimeDrive::ParametricPump {
                                        depth: constant(0.2),
                                        frequency_hz: constant(1.0),
                                        phase_radians: constant(0.0),
                                    };
                                }
                            })
                        });
                        let outcome = (|| -> Result<String, String> {
                            let (_, _, prepared) = prepare_at(&document, 0.2)?;
                            let dt = prepared.recommended_time_step();
                            let clock =
                                CanonicalGpuClock::initial(dt).map_err(|e| format!("{e:?}"))?;
                            let plan = match &prepared.canonical_temporal_operator {
                                Some(temporal) => {
                                    let state = CanonicalTemporalWaveState::zero(temporal, dt)
                                        .map_err(|e| e.to_string())?;
                                    CanonicalGpuPlan::compile_temporal(
                                        temporal,
                                        &state,
                                        &prepared.canonical_forcing,
                                        clock,
                                    )
                                }
                                None => {
                                    let state =
                                        CanonicalWaveState::zero(&prepared.canonical_operator, dt)
                                            .map_err(|e| e.to_string())?;
                                    CanonicalGpuPlan::compile_with_quadratic(
                                        &prepared.canonical_operator,
                                        &prepared.operator,
                                        &state,
                                        &prepared.canonical_forcing,
                                        clock,
                                    )
                                }
                            }
                            .map_err(|e| format!("the plan: {e:?}"))?;
                            let empty_buffers = plan
                                .buffer_bytes()
                                .iter()
                                .filter(|(_, bytes)| *bytes == 0)
                                .map(|(name, _)| *name)
                                .collect::<Vec<_>>();
                            plan.layout().map_err(|e| format!("the layout: {e:?}"))?;
                            let mut run = at_rest(&prepared, dt, CanonicalIntegrator::FourthOrder)?;
                            let domain = prepared.bundle.authored.geometry.domain;
                            strike_with(
                                &mut run,
                                &prepared,
                                &pulse_increment(&prepared, Pulse::off_centre(domain, 1.0)),
                            )?;
                            for _ in 0..40 {
                                run.step(&prepared)?;
                            }
                            let finite = field(&run, &prepared).iter().all(|u| u.is_finite());
                            Ok(format!(
                                "{} probes compiled, driven {}, finite {finite}, empty buffers {empty_buffers:?}",
                                prepared.probes.len(),
                                prepared.driven()
                            ))
                        })();
                        match outcome {
                            Ok(text) => eprintln!("STUDY empty {label}: {text}"),
                            Err(error) => {
                                failures += 1;
                                eprintln!("STUDY empty {label}: FAILED {error}");
                            }
                        }
                    }
                }
            }
        }
    }
    eprintln!("STUDY empty: {failures} of {cases} failed");
}

// ---------------------------------------------------------------------------
// Sums that cancel.
// ---------------------------------------------------------------------------

/// Neumaier's compensated sum.
fn compensated(values: impl Iterator<Item = f64>) -> f64 {
    let (mut sum, mut carry) = (0.0_f64, 0.0_f64);
    for value in values {
        let next = sum + value;
        carry += if sum.abs() >= value.abs() {
            (sum - next) + value
        } else {
            (value - next) + sum
        };
        sum = next;
    }
    sum + carry
}

/// Two packets of opposite sign in each ceiling scene, half a unit high to
/// sit inside the Kerr scene's bound: the primary flux's total, which only a
/// source, a wall or a restoring law moves, read naively and compensated
/// after every 100 steps over 4 s, against its own magnitude.
#[test]
#[ignore = "C8.1's study: cargo test --release -p funfern-app --test edge_cases -- --ignored --nocapture study_cancellation"]
fn study_cancellation() {
    for (name, document) in ceiling_scenes() {
        let (_, _, prepared) = prepare_at(&document, 0.1).unwrap();
        let dt = prepared.recommended_time_step();
        let mut run = at_rest(&prepared, dt, CanonicalIntegrator::FourthOrder).unwrap();
        let points = prepared.canonical_operator.node_points();
        let increment = points
            .iter()
            .map(|p| {
                let a = Pulse {
                    centre: Point2::new(-0.4, 0.1),
                    width: 0.2,
                    height: 0.5,
                };
                let b = Pulse {
                    centre: Point2::new(0.4, -0.1),
                    width: 0.2,
                    height: -0.5,
                };
                a.at(*p) + b.at(*p)
            })
            .collect::<Vec<_>>();
        strike_with(&mut run, &prepared, &increment).unwrap();
        let magnitude = run.primary().iter().map(|q| q.abs()).sum::<f64>();
        let first = compensated(run.primary().iter().copied());
        let (mut naive_worst, mut exact_worst) = (0.0_f64, 0.0_f64);
        let steps = (4.0 / dt).ceil() as usize;
        for step in 0..steps {
            if run.step(&prepared).is_err() {
                eprintln!("STUDY cancellation {name}: refused at {step}");
                break;
            }
            if step % 100 == 0 {
                let naive = run.primary().iter().sum::<f64>();
                let exact = compensated(run.primary().iter().copied());
                naive_worst = naive_worst.max((naive - exact).abs());
                exact_worst = exact_worst.max((exact - first).abs());
            }
        }
        eprintln!(
            "STUDY cancellation {name}: total {first:+.3e} of {magnitude:.3e}; over {steps} steps it moved {:.2e} of the magnitude, and the naive sum stood {:.2e} of it from the compensated",
            exact_worst / magnitude,
            naive_worst / magnitude
        );
    }
}

// ---------------------------------------------------------------------------
// The gate's checks, from what the studies found.
// ---------------------------------------------------------------------------

/// The cleanup's solve on a smooth potential's flux, which holds no
/// stationary part: the share of it the solve found stationary, in how many
/// iterations of how many allowed.
fn cleanup_of_a_potentials_flux(
    prepared: &PreparedTopology,
) -> Result<(f64, usize, usize), String> {
    let operator = prepared.canonical_operator.clone();
    let domain = prepared.bundle.authored.geometry.domain;
    let potential = pulse_increment(prepared, Pulse::off_centre(domain, 1.0));
    let flux = operator
        .compatible_flux(&potential)
        .map_err(|error| error.to_string())?;
    let mut job =
        CanonicalStationaryFluxJob::new(operator, flux).map_err(|error| error.to_string())?;
    let limit = job.limit();
    let found = loop {
        if let Some(found) = job.advance(1 << 16) {
            break found.map_err(|error| error.to_string())?;
        }
    };
    Ok((found.energy / found.flux_energy, found.iterations, limit))
}

/// The cleanup on an anisotropic medium and on a mesh graded down to a
/// narrow channel. Both took more than the twenty times the node count's
/// root and 1,000 that were allowed, the box at axis ratio 64 about 4,300
/// iterations on 5,485 nodes, and failed past them as a non-finite state
/// (C8.1, 2026-11-10).
#[test]
fn the_cleanup_converges_on_anisotropic_media_and_graded_meshes() {
    for (name, document) in [
        ("axis ratio 64", anisotropic(64.0)),
        ("a channel 3e-4 wide", holes(vec![channel(3.0e-4)]).unwrap()),
    ] {
        let (_, _, prepared) = prepare_at(&document, 0.1).unwrap();
        let (share, iterations, limit) = cleanup_of_a_potentials_flux(&prepared)
            .unwrap_or_else(|error| panic!("{name}: {error}"));
        eprintln!("{name}: {share:.2e} stationary in {iterations} of {limit} iterations");
        assert!(
            share <= 1.0e-11,
            "{name}: {share:e} of a potential's flux read as stationary"
        );
    }
}

/// At axis ratio 1e4 the isotropic tolerance, `1e-6` of the flux's force,
/// left a potential's flux partly stationary (C8.1, 2026-11-10). It tightens
/// with the ratio's root up to there and no further: at 1e6 the solve to
/// `1e-9` outgrew its limit on a mesh of 87,451 nodes.
#[test]
fn the_cleanup_tightens_with_the_axis_ratio() {
    let (_, _, prepared) = prepare_at(&anisotropic(1.0e4), 0.1).unwrap();
    let (share, iterations, limit) = cleanup_of_a_potentials_flux(&prepared).unwrap();
    eprintln!("{share:.2e} stationary in {iterations} of {limit} iterations");
    assert!(
        share <= 1.0e-11,
        "{share:e} of a potential's flux read as stationary"
    );
    let (_, _, extreme) = prepare_at(&anisotropic(1.0e6), 0.1).unwrap();
    let operator = extreme.canonical_operator.clone();
    let flux = vec![Point2::default(); operator.complementary_degrees_of_freedom()];
    let tolerance = CanonicalStationaryFluxJob::new(operator, flux)
        .unwrap()
        .tolerance();
    assert!(
        (tolerance / 1.0e-8 - 1.0).abs() < 1.0e-9,
        "tolerance {tolerance:e} at axis ratio 1e6"
    );
}

/// A stiffness law's vector inverse and van der Pol's map, struck at 1e-200,
/// where a flux's square underflows, against the same medium struck at
/// 1e-100, per unit of amplitude. Until C8.1 (2026-11-10) the CPU reference
/// took either flux for none: the stiffness law's field for zero, van der
/// Pol's node to zero.
#[test]
fn a_pulse_too_small_to_square_keeps_its_shape() {
    for name in ["focusing Kerr on the stiffness", "van der Pol"] {
        let law = laws().into_iter().find(|law| law.name == name).unwrap();
        let document = document(OuterBoundaryCondition::Reflecting, |editor| {
            medium(editor, |m| (law.author)(m))
        });
        let (_, _, prepared) = prepare_at(&document, 0.2).unwrap();
        let dt = prepared.recommended_time_step();
        let domain = prepared.bundle.authored.geometry.domain;
        let unit = pulse_increment(&prepared, Pulse::off_centre(domain, 1.0));
        let peak = unit.iter().fold(0.0_f64, |m, v| m.max(*v));
        let shape = |amplitude: f64| {
            let increment = unit
                .iter()
                .map(|v| v * amplitude / peak)
                .collect::<Vec<_>>();
            let mut run = at_rest(&prepared, dt, CanonicalIntegrator::FourthOrder).unwrap();
            strike_with(&mut run, &prepared, &increment).unwrap();
            for _ in 0..100 {
                run.step(&prepared).unwrap();
            }
            field(&run, &prepared)
                .iter()
                .map(|u| u / amplitude)
                .collect::<Vec<_>>()
        };
        let (small, tiny) = (shape(1.0e-100), shape(1.0e-200));
        let norm = small.iter().map(|u| u * u).sum::<f64>().sqrt();
        let gap = tiny
            .iter()
            .zip(&small)
            .map(|(t, s)| (t - s).powi(2))
            .sum::<f64>()
            .sqrt();
        eprintln!("{name}: {:.2e} from its shape at 1e-100", gap / norm);
        assert!(
            gap <= 1.0e-12 * norm,
            "{name}: {:e} from its shape at 1e-100",
            gap / norm
        );
    }
}

/// Sine-Gordon's potential at small amplitudes against its series: formed
/// as `1 − cos r`, it was 9e-5 off at 1e-6 and zero below 1e-8 (C8.1,
/// 2026-11-10).
#[test]
fn sine_gordons_potential_keeps_its_digits_at_small_amplitudes() {
    let law = RestoringLawValues::SineGordon { omega0: 3.0 };
    for r in [1.0e-3, 1.0e-6, 1.0e-9, 1.0e-100] {
        let series = 4.5 * r * r * (1.0 - r * r / 12.0);
        let potential = law.potential(r);
        assert!(
            (potential - series).abs() <= 1.0e-13 * series,
            "at {r:e}: {potential:e} against {series:e}"
        );
    }
}

/// A pulse the state cannot hold, its energy past the largest float, is
/// refused with the state it had. The fixed state kept the flux it was
/// refused for until C8.1 (2026-11-10).
#[test]
fn a_refused_pulse_leaves_the_state_alone() {
    let document = document(OuterBoundaryCondition::Reflecting, |_| Ok(()));
    let (_, _, prepared) = prepare_at(&document, 0.2).unwrap();
    assert!(
        prepared.canonical_temporal_operator.is_none(),
        "a fixed generation"
    );
    let dt = prepared.recommended_time_step();
    let domain = prepared.bundle.authored.geometry.domain;
    let unit = pulse_increment(&prepared, Pulse::off_centre(domain, 1.0));
    let mut run = at_rest(&prepared, dt, CanonicalIntegrator::FourthOrder).unwrap();
    strike_with(&mut run, &prepared, &unit).unwrap();
    for _ in 0..10 {
        run.step(&prepared).unwrap();
    }
    let before = run.bits();
    let huge = unit.iter().map(|v| v * 1.0e300).collect::<Vec<_>>();
    let refused = strike_with(&mut run, &prepared, &huge);
    assert!(refused.is_err(), "a pulse 1e300 times the unit landed");
    assert!(run.bits() == before, "the refused pulse changed the state");
    run.step(&prepared).unwrap();
}
