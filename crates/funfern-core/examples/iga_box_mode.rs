//! IGA feasibility spike 1: the reflecting-box mode of `wave_convergence`,
//! `cos(5π(x+1))` at wavelength 0.4 and wave speed 1 on the 2 × 2 box, on an
//! affine spline patch at the triangle solver's dof count (96² = 9216 against
//! P2e's 9215 at parent h 0.08), by degree, Gauss rule and mass treatment.
//! See `docs/spikes/funfern-iga-feasibility-spike.md`.
//!
//! Flags: `--degree 3 --points 4 --treatment lumped|sweeps:1|consistent
//! --side 96 --fraction 0.8 --targets 2,10 --basis clamped|unclamped`. Without flags it runs the matrix.

use std::time::Instant;

use funfern_core::{
    BoxPatch, MassTreatment, ModeClock, PatchStepper, PeriodicSymbols, Point2, gauss_rule,
    leapfrog_frequency, measure_mode, wrap_angle,
};

const WAVELENGTH: f64 = 0.4;
const WAVE_NUMBER: f64 = std::f64::consts::TAU / WAVELENGTH;

struct Case {
    degree: usize,
    points: usize,
    treatment: MassTreatment,
    side: usize,
    fraction: f64,
    targets: Vec<f64>,
    unclamped: bool,
}

fn treatment_name(treatment: MassTreatment) -> String {
    match treatment {
        MassTreatment::Lumped { sweeps: 0 } => "lumped".to_string(),
        MassTreatment::Lumped { sweeps } => format!("lumped+{sweeps}"),
        MassTreatment::Consistent => "consistent".to_string(),
    }
}

fn parse_treatment(text: &str) -> MassTreatment {
    match text {
        "lumped" => MassTreatment::Lumped { sweeps: 0 },
        "consistent" => MassTreatment::Consistent,
        other => MassTreatment::Lumped {
            sweeps: other
                .strip_prefix("sweeps:")
                .and_then(|k| k.parse().ok())
                .expect("--treatment lumped|sweeps:N|consistent"),
        },
    }
}

fn cases() -> Vec<Case> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        let mut cases = Vec::new();
        for degree in [2, 3] {
            for points in [2, 3] {
                for treatment in [
                    MassTreatment::Lumped { sweeps: 0 },
                    MassTreatment::Lumped { sweeps: 1 },
                    MassTreatment::Lumped { sweeps: 2 },
                    MassTreatment::Lumped { sweeps: 3 },
                    MassTreatment::Consistent,
                ] {
                    cases.push(Case {
                        degree,
                        points,
                        treatment,
                        side: 96,
                        fraction: 0.8,
                        targets: vec![2.0, 10.0],
                        unclamped: false,
                    });
                }
            }
        }
        return cases;
    }
    let value = |flag: &str| {
        args.iter()
            .position(|a| a == flag)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    vec![Case {
        degree: value("--degree").map_or(3, |v| v.parse().unwrap()),
        points: value("--points").map_or(4, |v| v.parse().unwrap()),
        treatment: value("--treatment")
            .map_or(MassTreatment::Lumped { sweeps: 0 }, |v| parse_treatment(&v)),
        side: value("--side").map_or(96, |v| v.parse().unwrap()),
        fraction: value("--fraction").map_or(0.8, |v| v.parse().unwrap()),
        targets: value("--targets").map_or(vec![2.0, 10.0], |v| {
            v.split(',').map(|t| t.parse().unwrap()).collect()
        }),
        unclamped: value("--basis").is_some_and(|v| v == "unclamped"),
    }]
}

fn main() {
    println!(
        "Reflecting-box mode cos(5π(x+1)); wavelength {WAVELENGTH}, wave speed 1; affine spline patch\n\
         on the 2 × 2 box, f64 CPU. P2e at parent h 0.08 (9215 DOFs, 18k samples): dt_max 6.98e-3\n\
         (Gershgorin), phase -5.1e-3 rad at t = 10 for dt = 0.225 dt_max and -8.1e-3 at 0.1125;\n\
         4.4 simulated s per wall s at 0.225 dt_max.\n"
    );
    for case in cases() {
        run(&case);
    }
}

fn run(case: &Case) {
    let spans = case.side - case.degree;
    let build: fn(usize, Point2, Point2, [usize; 2], usize) -> Option<BoxPatch> = if case.unclamped
    {
        BoxPatch::unclamped
    } else {
        BoxPatch::new
    };
    let patch = build(
        case.degree,
        Point2::new(-1.0, -1.0),
        Point2::new(2.0, 2.0),
        [spans, spans],
        case.points,
    )
    .unwrap();
    let h = patch.basis_x().spacing();
    let symbols = PeriodicSymbols::new(case.degree, h, &gauss_rule(case.points).unwrap());
    let eigen_start = Instant::now();
    let (lambda, converged) = patch.largest_eigenvalue(case.treatment, 1500);
    let eigen_seconds = eigen_start.elapsed().as_secs_f64();
    let dt_max = 2.0 / lambda.sqrt();
    let periodic_dt_max = 2.0 / symbols.maximum_frequency(case.treatment.sweeps());
    // The power iteration estimates the top eigenvalue from below, so an
    // unconverged estimate overstates the stable step. The periodic band
    // bounds it unless the walls add outliers, and an isolated outlier is
    // what a power iteration finds fastest.
    let dt = case.fraction * dt_max.min(periodic_dt_max);
    let theta = WAVE_NUMBER * h;
    let omega_h = symbols
        .frequency_squared(theta, 0.0, case.treatment.sweeps())
        .sqrt();
    let predicted = leapfrog_frequency(omega_h, dt);
    let predicted_rate = predicted - WAVE_NUMBER;
    println!(
        "p={} q={} {} {}: {} DOFs, {} samples, {:.1} spans/λ, dt_max {:.4e} ({}; periodic band {:.4e}, ratio {:.3}), eigen {:.1} s",
        case.degree,
        case.points,
        treatment_name(case.treatment),
        if case.unclamped {
            "unclamped"
        } else {
            "clamped"
        },
        patch.degrees_of_freedom(),
        patch.samples(),
        WAVELENGTH / h,
        dt_max,
        if converged {
            "converged"
        } else {
            "NOT converged"
        },
        periodic_dt_max,
        dt_max / periodic_dt_max,
        eigen_seconds,
    );
    let mode = patch.project_separable(|x| (WAVE_NUMBER * (x + 1.0)).cos(), |_| 1.0);
    let mut stepper = PatchStepper::new(&patch, case.treatment, dt, mode.clone());
    let initial_energy = stepper.staggered_energy();
    let mut targets = case.targets.clone();
    targets.sort_by(f64::total_cmp);
    let mut wall = 0.0;
    for target in targets {
        let steps = (target / dt).round().max(1.0) as u64;
        let mark = Instant::now();
        while stepper.steps() < steps {
            stepper.step();
        }
        wall += mark.elapsed().as_secs_f64();
        // The sine component is separated with the predicted discrete
        // frequency; the exact angle, which `wave_convergence` uses, would
        // offset the phase by about the relative frequency error.
        let measured = measure_mode(
            &patch,
            &mode,
            stepper.field(),
            stepper.previous_field(),
            ModeClock {
                omega: WAVE_NUMBER,
                readout: predicted,
                dt,
                time: stepper.time(),
            },
        );
        let energy_drift = (stepper.staggered_energy() - initial_energy).abs() / initial_energy;
        // The measured drift is the leapfrog frequency of the mode less the
        // exact one. Inverting the leapfrog relation takes the temporal part
        // out and leaves the spatial frequency error, walls included.
        let omega_numerical = WAVE_NUMBER + measured.phase_error / stepper.time();
        let omega_spatial = (2.0 / dt) * (0.5 * omega_numerical * dt).sin();
        let spatial = (omega_spatial - WAVE_NUMBER) * stepper.time();
        let spatial_predicted = (omega_h - WAVE_NUMBER) * stepper.time();
        println!(
            "  dt={:.3} dt_max ({:.4e}), target {:>4.1}, reached {:.5}: phase {:+.4e} rad (symbol predicts {:+.4e}); spatial part {:+.4e} rad (symbol {:+.4e}); amplitude {:+.4e}, energy drift {:.1e}, {} steps, {:.2} simulated s/wall s",
            case.fraction,
            dt,
            target,
            stepper.time(),
            measured.phase_error,
            wrap_angle(predicted_rate * stepper.time()),
            spatial,
            spatial_predicted,
            measured.amplitude_error,
            energy_drift,
            stepper.steps(),
            stepper.time() / wall.max(f64::MIN_POSITIVE),
        );
    }
    println!();
}
