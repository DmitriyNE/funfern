//! What a time-driven medium costs the core, per composition.
//!
//! Stage 7's exit criterion asks for the actual-core incremental cost to be
//! recorded, and the boundary-composition work turned up a specific reason to
//! measure it per capability rather than once: the second-order outgoing
//! boundary's Schur complement is built from the nodal mass, so a fixed
//! generation factorizes once at construction while a driven one rebuilds it
//! at every stage.
//!
//! The measurement found that expected difference and something larger that
//! was not expected. The driven bulk alone runs about twenty times the fixed
//! bulk, before any boundary capability is involved. That is not the
//! coefficient evaluation being intrinsically expensive; it is the same
//! instantaneous factor being recomputed from scratch by each helper that
//! wants it. One step calls `energy`, `complementary_energy_and_rate` twice,
//! `force_at` twice, `primary_mass_at` at least three times,
//! `primary_energy_and_rate`, `drift_at` and `energy_at` - roughly a dozen
//! full walks over every contribution and sample, each evaluating a
//! transcendental per entry, where the fixed path reads a table. Caching the
//! stage's factors once and passing them down is the obvious remedy and is
//! recorded rather than assumed here.
//!
//! Both sides run the same mesh, the same operator and the same forcing at the
//! same timestep, so the only difference measured is the coefficient work.
//!
//! `--nonlinear` replaces the drive with a field-dependent medium (Kerr on the
//! mass row, saturable on the stiffness row) at an amplitude where both maps
//! depart from linear by tens of percent. That column is the Stage 8 cost of a
//! bracketed inverse at every node and sample, plus the discrete-gradient kick
//! on a nonlinear trace.

use std::time::Instant;

use funfern_core::{
    CanonicalForcing, CanonicalSource, CanonicalTemporalWaveOperator, CanonicalTemporalWaveState,
    CanonicalWaveState, CoefficientLaw, FieldLaw, InternalBoundary, InternalBoundaryCoupling,
    InternalBoundaryId, InternalBoundaryLaw, MeshingOptions, OpenCubicSpline,
    OuterBoundaryCondition, Point2, QuadraticWaveOperator, ScalarField, Scene, TimeDrive,
    TimeSignal, mesh_scene,
};

const STEPS: u64 = 64;
const EDGE: f64 = 0.1;

fn main() {
    let nonlinear = std::env::args().any(|argument| argument == "--nonlinear");
    println!(
        "{} incremental cost on the CPU core",
        if nonlinear {
            "Field-dependent"
        } else {
            "Time-driven"
        }
    );
    println!("Same mesh, operator, forcing and timestep on both sides; h={EDGE}, {STEPS} steps.\n");
    println!(
        "{:<26} {:>7} {:>11} {:>11} {:>8}",
        "composition", "DOFs", "fixed us/st", "driven us/st", "ratio"
    );

    for (label, boundary, gap, source) in [
        ("bulk", OuterBoundaryCondition::Reflecting, false, false),
        (
            "bulk + source",
            OuterBoundaryCondition::Reflecting,
            false,
            true,
        ),
        ("thin gap", OuterBoundaryCondition::Reflecting, true, false),
        (
            "first-order wall",
            OuterBoundaryCondition::FirstOrderOutgoing,
            false,
            false,
        ),
        (
            "second-order wall",
            OuterBoundaryCondition::SecondOrderOutgoing,
            false,
            false,
        ),
    ] {
        measure(label, boundary, gap, source, nonlinear);
    }

    println!(
        "\nThe bulk ratio is the floor: it is what recomputing a stage's coefficients\n\
         in every helper that wants them costs, with no boundary capability involved.\n\
         The second-order wall now sits under that floor. Its trace system carries no\n\
         nodal mass, so a driven generation prepares it once and sweeps it with the\n\
         stage's own mass instead of refactorizing. A fixed generation still inverts\n\
         once and solves in a single pass, which is why its column is unchanged."
    );
}

fn measure(
    label: &str,
    boundary: OuterBoundaryCondition,
    gap: bool,
    source: bool,
    nonlinear: bool,
) {
    let mut scene = Scene::default();
    if gap {
        scene.internal_boundaries.push(InternalBoundary {
            id: InternalBoundaryId(1),
            spline: OpenCubicSpline::uniform(vec![
                Point2::new(-0.65, 0.0),
                Point2::new(-0.2, 0.0),
                Point2::new(0.2, 0.0),
                Point2::new(0.65, 0.0),
            ])
            .unwrap(),
            region: funfern_core::BACKGROUND_REGION,
            span_laws: vec![InternalBoundaryLaw {
                coupling: InternalBoundaryCoupling::ThinGap {
                    stiffness_ratio: 120.0,
                },
                ..InternalBoundaryLaw::REFLECTING
            }],
        });
    }
    if nonlinear {
        scene.materials[0].mass_law.field = FieldLaw::Polynomial {
            chi1: ScalarField::constant(0.0),
            chi2: ScalarField::constant(0.8),
            amplitude_bound: None,
        };
        scene.materials[0].stiffness_law.field = FieldLaw::Saturable {
            chi: ScalarField::constant(6.0),
            saturation: ScalarField::constant(0.3),
        };
    } else {
        scene.materials[0].mass_law.drive = TimeDrive::ParametricPump {
            depth: ScalarField::constant(0.2),
            frequency_hz: ScalarField::constant(1.1),
            phase_radians: ScalarField::constant(0.3),
        };
    }
    let amplitude = if nonlinear { 10.0 } else { 1.0 };

    let mut fixed_scene = scene.clone();
    for material in &mut fixed_scene.materials {
        material.mass_law = CoefficientLaw::linear();
        material.stiffness_law = CoefficientLaw::linear();
    }
    let mesh = mesh_scene(
        &fixed_scene,
        17,
        MeshingOptions {
            target_edge_length: EDGE,
            minimum_angle_degrees: 14.0,
            ..MeshingOptions::default()
        },
    )
    .expect("timing mesh");
    let quadratic = QuadraticWaveOperator::assemble_scene(&mesh, &fixed_scene, boundary)
        .expect("timing scalar operator");
    let operator = CanonicalTemporalWaveOperator::compile_scene(&mesh, &quadratic, &scene, 1)
        .expect("timing temporal operator");
    let base = operator.base();

    let mut forcing = CanonicalForcing::none(base);
    if source {
        forcing
            .push_source(
                CanonicalSource::direct(
                    base,
                    base.primary_mass().to_vec(),
                    TimeSignal::harmonic(0.0, 0.5, 1.3, 0.2),
                )
                .expect("timing source"),
            )
            .expect("push timing source");
    }

    // The driven ceiling is the lower of the two, so both sides run it.
    let time_step = 0.4 * operator.maximum_time_step();
    let primary = base
        .node_points()
        .iter()
        .map(|point| amplitude * 0.05 * (1.4 * point.x - 0.9 * point.y).sin())
        .collect::<Vec<_>>();
    let potential = base
        .node_points()
        .iter()
        .map(|point| amplitude * 0.03 * (0.9 * point.x + 1.2 * point.y).cos())
        .collect::<Vec<_>>();
    let complementary = base.compatible_flux(&potential).expect("timing flux");

    let mut fixed =
        CanonicalWaveState::new(base, time_step, primary.clone(), complementary.clone())
            .expect("fixed timing state");
    let mut driven = CanonicalTemporalWaveState::new(&operator, time_step, primary, complementary)
        .expect("driven timing state");

    // One untimed step each, so a first-touch page fault or a lazily built
    // cache is not charged to the measurement.
    fixed
        .step_with_forcing(base, &forcing)
        .expect("fixed warmup");
    driven
        .step_with_forcing(&operator, &forcing)
        .expect("driven warmup");

    let start = Instant::now();
    for _ in 0..STEPS {
        fixed.step_with_forcing(base, &forcing).expect("fixed step");
    }
    let fixed_micros = start.elapsed().as_secs_f64() * 1.0e6 / STEPS as f64;

    let start = Instant::now();
    for _ in 0..STEPS {
        driven
            .step_with_forcing(&operator, &forcing)
            .expect("driven step");
    }
    let driven_micros = start.elapsed().as_secs_f64() * 1.0e6 / STEPS as f64;

    println!(
        "{label:<26} {:>7} {fixed_micros:>11.1} {driven_micros:>11.1} {:>8.2}",
        base.degrees_of_freedom(),
        driven_micros / fixed_micros
    );
}
