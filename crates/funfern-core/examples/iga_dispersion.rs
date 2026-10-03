//! IGA feasibility spikes 1 and 2: the dispersion and the stable step of a
//! uniform spline patch under each mass treatment and Gauss rule, read from
//! the periodic symbols. See `docs/spikes/funfern-iga-feasibility-spike.md`.

use funfern_core::{PeriodicSymbols, gauss_rule};

const POINTS_PER_WAVELENGTH: [f64; 9] = [4.0, 5.0, 6.0, 8.0, 10.0, 12.0, 15.0, 20.0, 30.0];

fn treatment_name(sweeps: Option<usize>) -> String {
    match sweeps {
        Some(0) => "lumped".to_string(),
        Some(k) => format!("lumped+{k}"),
        None => "consistent".to_string(),
    }
}

fn main() {
    println!(
        "Relative frequency error ω_h/ω − 1 of a plane wave on a tensor patch with unit material,\n\
         by points (spans) per wavelength, along a parametric axis and along the diagonal.\n\
         dt_max is the leapfrog limit over the periodic band in units of h/c. P2e for comparison:\n\
         at 5 parent edges per wavelength its phase lags 9e-3 rad over t = 10 at ω = 5π, a\n\
         relative frequency error of -5.8e-5, and its node spacing is 19 per wavelength.\n"
    );
    print!(
        "{:<22} {:>8} {:>7} {:>9}",
        "basis / rule / mass", "dt_max", "work", "K rule/4"
    );
    for ppw in POINTS_PER_WAVELENGTH {
        print!(" {:>18}", format!("{ppw} ppw"));
    }
    println!();
    for degree in 1..=3 {
        for points in 1..=4 {
            if degree == 1 && points > 2 {
                continue;
            }
            let rule = gauss_rule(points).unwrap();
            let symbols = PeriodicSymbols::new(degree, 1.0, &rule);
            // Drift work per dof against P2e's two samples of seven nodes.
            let work = (points * points * (degree + 1) * (degree + 1)) as f64 / 14.0;
            for sweeps in [Some(0), Some(1), Some(2), Some(3), None] {
                let label = format!("p={degree} q={points} {}", treatment_name(sweeps));
                let dt_max = 2.0 / symbols.maximum_frequency(sweeps);
                // Each sweep is one more pass over the samples.
                let total_work = sweeps.map_or(f64::NAN, |k| work * (1.0 + k as f64));
                // The least ratio of the rule's stiffness symbol to the
                // four-point rule's over the band, Nyquist included: a rule
                // with a spurious zero-energy mode shows zero, and a rule that
                // integrates the stiffness exactly shows one.
                let full = PeriodicSymbols::new(degree, 1.0, &gauss_rule(4).unwrap());
                let mut least = f64::INFINITY;
                for i in 0..=64 {
                    let theta = std::f64::consts::PI * i as f64 / 64.0;
                    for j in 0..=64 {
                        if i == 0 && j == 0 {
                            continue;
                        }
                        let phi = std::f64::consts::PI * j as f64 / 64.0;
                        let stiffness = |s: &PeriodicSymbols| {
                            s.stiffness(theta) * s.mass(phi) + s.mass(theta) * s.stiffness(phi)
                        };
                        least = least.min(stiffness(&symbols) / stiffness(&full));
                    }
                }
                print!(
                    "{:<22} {:>8.3} {:>7.2} {:>9.3}",
                    label, dt_max, total_work, least
                );
                for ppw in POINTS_PER_WAVELENGTH {
                    let theta = std::f64::consts::TAU / ppw;
                    let axis = symbols.frequency_ratio(theta, 0.0, sweeps) - 1.0;
                    let d = theta / std::f64::consts::SQRT_2;
                    let diagonal = symbols.frequency_ratio(d, d, sweeps) - 1.0;
                    print!(" {:>+8.1e}/{:>+8.1e}", axis, diagonal);
                }
                println!();
            }
            println!();
        }
    }
    println!(
        "work: drift multiply-adds per dof relative to P2e, counting each Jacobi sweep as one\n\
         more pass over the samples (a mass application is a gather and a scatter, as the\n\
         drift and kick together are)."
    );
}
