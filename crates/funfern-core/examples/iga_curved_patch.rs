//! IGA feasibility spike 3: a single untrimmed patch over a curved domain.
//! For each geometry: the Jacobian and lumped-mass ranges, the stable step
//! against an affine box of equal area and dofs, and on exact disks the
//! frequency error of a Neumann mode with a known frequency, stepped by the
//! fourth-order integrator. See `docs/spikes/funfern-iga-feasibility-spike.md`.
//!
//! Flags: `--side 64 --degree 2 --points 2 --sweeps 2 --condense 0,3 --scale 0,4
//! --clamped`: the corner condensation reaches and the mass-scaling caps to
//! try, zero for none; `--clamped` adds the clamped rows.

use std::time::Instant;

use funfern_core::{
    GeometryMap, Integrator, MassTreatment, ModeClock, PatchStepper, PeriodicCubicSpline, Point2,
    SplinePatch, measure_mode,
};

struct Geometry {
    name: &'static str,
    map: GeometryMap,
    /// A Neumann mode with an exactly known frequency, if the domain has one.
    mode: Option<DiskMode>,
}

struct DiskMode {
    center: Point2,
    radius: f64,
    order: usize,
    /// The zero of `J_m'` the mode sits at: `k R`.
    zero: f64,
}

fn bessel_j(order: usize, x: f64) -> f64 {
    let half = 0.5 * x;
    let mut term = half.powi(order as i32) / (1..=order).map(|k| k as f64).product::<f64>();
    let mut sum = term;
    for k in 1..60 {
        term *= -half * half / (k as f64 * (k + order) as f64);
        sum += term;
        if term.abs() < 1.0e-17 * sum.abs() {
            break;
        }
    }
    sum
}

/// `J_m'(x) = (J_{m-1}(x) - J_{m+1}(x)) / 2`.
fn bessel_j_derivative(order: usize, x: f64) -> f64 {
    let lower = if order == 0 {
        -bessel_j(1, x)
    } else {
        bessel_j(order - 1, x)
    };
    0.5 * (lower - bessel_j(order + 1, x))
}

/// The zero of `J_m'` nearest `guess`, by Newton on a secant slope.
fn derivative_zero(order: usize, guess: f64) -> f64 {
    let mut x = guess;
    for _ in 0..60 {
        let f = bessel_j_derivative(order, x);
        let h = 1.0e-6;
        let slope =
            (bessel_j_derivative(order, x + h) - bessel_j_derivative(order, x - h)) / (2.0 * h);
        let next = x - f / slope;
        if (next - x).abs() < 1.0e-14 {
            return next;
        }
        x = next;
    }
    x
}

fn geometries() -> Vec<Geometry> {
    let unit_disk = |order: usize, guess: f64| DiskMode {
        center: Point2::default(),
        radius: 1.0,
        order,
        zero: derivative_zero(order, guess),
    };
    let mut out = vec![
        Geometry {
            name: "affine 2 × 2 box",
            map: GeometryMap::Affine {
                origin: Point2::new(-1.0, -1.0),
            },
            mode: None,
        },
        Geometry {
            name: "exact unit disk, Coons, corners on the diagonals; J1 mode",
            map: GeometryMap::coons_ellipse(Point2::default(), Point2::new(1.0, 1.0)),
            mode: Some(unit_disk(1, 1.84)),
        },
        Geometry {
            name: "exact unit disk, Coons; J3 third mode (17 spans per wavelength at side 64)",
            map: GeometryMap::coons_ellipse(Point2::default(), Point2::new(1.0, 1.0)),
            mode: Some(unit_disk(3, 11.35)),
        },
        Geometry {
            name: "exact 2:1 ellipse, Coons",
            map: GeometryMap::coons_ellipse(Point2::default(), Point2::new(1.0, 0.5)),
            mode: None,
        },
        Geometry {
            name: "gallery rounded circle (8 controls), Coons at the knots 1,3,5,7",
            map: {
                let curve = PeriodicCubicSpline::rounded(Point2::default(), 1.0);
                let period = curve.period();
                GeometryMap::coons_from_curve(
                    &curve,
                    [1.0, 3.0, 5.0, 7.0].map(|knot| knot * period / 8.0),
                )
            },
            mode: None,
        },
        Geometry {
            name: "gallery-like blob (smoothed irregular hexagon), Coons",
            map: {
                let curve = PeriodicCubicSpline::polygon(vec![
                    Point2::new(1.0, 0.0),
                    Point2::new(0.6, 0.9),
                    Point2::new(-0.5, 1.1),
                    Point2::new(-1.2, 0.2),
                    Point2::new(-0.7, -0.8),
                    Point2::new(0.4, -1.0),
                ])
                .unwrap();
                let period = curve.period();
                GeometryMap::coons_from_curve(
                    &curve,
                    [0.5, 2.0, 3.5, 5.0].map(|t| t * period / 6.0),
                )
            },
            mode: None,
        },
    ];
    // A bulged box: the flat surface with its right side's middle pushed out.
    let mut bulged =
        GeometryMap::flat_surface(Point2::new(-1.0, -1.0), Point2::new(2.0, 2.0), [6, 6]);
    if let GeometryMap::Surface {
        basis_x,
        basis_y,
        controls,
    } = &mut bulged
    {
        let ny = basis_y.functions();
        let nx = basis_x.functions();
        for iy in 0..ny {
            let s = iy as f64 / (ny - 1) as f64;
            let bump = (std::f64::consts::PI * s).sin();
            for ix in 0..nx {
                let r = ix as f64 / (nx - 1) as f64;
                controls[ix * ny + iy].x += 0.4 * bump * r * r;
            }
        }
    }
    out.insert(
        1,
        Geometry {
            name: "bulged box (right side pushed out by 0.4)",
            map: bulged,
            mode: None,
        },
    );
    out
}

fn stats(values: &[f64]) -> (f64, f64, f64) {
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    (
        sorted[0],
        sorted[sorted.len() / 2],
        sorted[sorted.len() - 1],
    )
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let value = |flag: &str, default: usize| {
        args.iter()
            .position(|a| a == flag)
            .and_then(|i| args.get(i + 1))
            .map_or(default, |v| v.parse().unwrap())
    };
    let side = value("--side", 64);
    let degree = value("--degree", 2);
    let points = value("--points", 2);
    let sweeps = value("--sweeps", 2);
    let reaches: Vec<usize> = args
        .iter()
        .position(|a| a == "--condense")
        .and_then(|i| args.get(i + 1))
        .map_or(vec![0, 3], |v| {
            v.split(',').map(|r| r.parse().unwrap()).collect()
        });
    let caps: Vec<f64> = args
        .iter()
        .position(|a| a == "--scale")
        .and_then(|i| args.get(i + 1))
        .map_or(vec![0.0], |v| {
            v.split(',').map(|r| r.parse().unwrap()).collect()
        });
    let clamped_too = args.iter().any(|a| a == "--clamped");
    let treatment = MassTreatment::Lumped { sweeps };
    let spans = side - degree;
    println!(
        "Curved single patches, degree {degree}, Gauss {points} × {points}, lumped + {sweeps} sweeps, {spans} × {spans} spans ({} DOFs).\n\
         dt_max is the leapfrog limit from a power iteration; the box is affine with equal area and dofs.\n",
        side * side
    );
    for geometry in geometries() {
        for unclamped in [true, false] {
            if !unclamped && !clamped_too {
                continue;
            }
            for &reach in &reaches {
                for &cap in &caps {
                    run(
                        &geometry, degree, points, treatment, spans, unclamped, reach, cap,
                    );
                }
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn run(
    geometry: &Geometry,
    degree: usize,
    points: usize,
    treatment: MassTreatment,
    spans: usize,
    unclamped: bool,
    reach: usize,
    cap: f64,
) {
    let size = Point2::new(1.0, 1.0);
    let Some(patch) = SplinePatch::curved(
        degree,
        geometry.map.clone(),
        size,
        [spans, spans],
        points,
        unclamped,
    ) else {
        println!(
            "{}: the map folds (a non-positive Jacobian at a sample)\n",
            geometry.name
        );
        return;
    };
    let mut patch = patch.with_condensed_corners(reach);
    if cap > 0.0 {
        patch = patch.with_mass_scaling(cap);
    }
    let area: f64 = patch.sample_weights().iter().sum();
    let determinants = patch.sample_determinants();
    let (det_min, det_median, det_max) = stats(&determinants);
    let (lumped_min, lumped_median, lumped_max) = stats(patch.lumped());
    let start = Instant::now();
    let (lambda, converged) = patch.largest_eigenvalue(treatment, 1500);
    let eigen_seconds = start.elapsed().as_secs_f64();
    let dt_max = 2.0 / lambda.sqrt();
    let box_side = area.sqrt();
    let build: fn(usize, Point2, Point2, [usize; 2], usize) -> Option<SplinePatch> = if unclamped {
        SplinePatch::unclamped
    } else {
        SplinePatch::new
    };
    let reference = build(
        degree,
        Point2::default(),
        Point2::new(box_side, box_side),
        [spans, spans],
        points,
    )
    .unwrap()
    .with_condensed_corners(reach);
    let reference = if cap > 0.0 {
        reference.with_mass_scaling(cap)
    } else {
        reference
    };
    let (box_lambda, _) = reference.largest_eigenvalue(treatment, 1500);
    let box_dt_max = 2.0 / box_lambda.sqrt();
    println!(
        "{} [{}{}]: {} DOFs, area {:.5}, det J min/median/max {:.3e}/{:.3e}/{:.3e}, lumped min/median/max {:.2e}/{:.2e}/{:.2e}, dt_max {:.3e} ({}, {:.0} s) = {:.3} × the equal-area box's {:.3e}",
        geometry.name,
        if unclamped { "unclamped" } else { "clamped" },
        if reach >= 2 {
            format!(", corners condensed to {reach}")
        } else {
            String::new()
        } + &if cap > 0.0 {
            format!(
                ", mass scaled at {} dofs to {cap} × the median bound (largest factor {:.1})",
                patch.scaled_count(),
                patch.largest_scaling_factor()
            )
        } else {
            String::new()
        },
        patch.degrees_of_freedom(),
        area,
        det_min,
        det_median,
        det_max,
        lumped_min,
        lumped_median,
        lumped_max,
        dt_max,
        if converged {
            "converged"
        } else {
            "NOT converged"
        },
        eigen_seconds,
        dt_max / box_dt_max,
        box_dt_max,
    );
    if let Some(mode) = &geometry.mode {
        let k = mode.zero / mode.radius;
        let omega = k;
        let order = mode.order;
        let shape = |p: Point2| {
            let r = p - mode.center;
            let radius = r.norm();
            let angle = r.y.atan2(r.x);
            bessel_j(order, k * radius) * (order as f64 * angle).cos()
        };
        let coefficients = patch.project(shape);
        let integrator = Integrator::FourthOrder;
        let dt = 0.8 * integrator.stability_factor() * dt_max;
        let target = 10.0;
        let steps = (target / dt).round().max(1.0) as u64;
        if steps > 400_000 {
            println!("  mode run skipped: {steps} steps to t = {target}\n");
            return;
        }
        let mut stepper =
            PatchStepper::with_integrator(&patch, treatment, integrator, dt, coefficients.clone());
        stepper.step();
        let initial_energy = stepper.conserved_energy();
        let start = Instant::now();
        while stepper.steps() < steps {
            stepper.step();
        }
        let wall = start.elapsed().as_secs_f64();
        // Read out with the exact frequency, then once more with the
        // measured one, which makes the sine separation consistent.
        let mut readout = omega;
        let mut measured = None;
        for _ in 0..3 {
            let m = measure_mode(
                &patch,
                &coefficients,
                stepper.field(),
                stepper.previous_field(),
                ModeClock {
                    omega,
                    readout,
                    dt,
                    time: stepper.time(),
                },
            );
            readout = omega + m.phase_error / stepper.time();
            measured = Some(m);
        }
        let measured = measured.unwrap();
        let omega_numerical = omega + measured.phase_error / stepper.time();
        let omega_spatial = integrator.spatial_frequency(omega_numerical, dt);
        let energy_drift = (stepper.conserved_energy() - initial_energy).abs() / initial_energy;
        println!(
            "  J{order} mode, kR {:.6}, ω {:.4}, {:.1} spans per wavelength: at t = {:.4} phase {:+.3e} rad, relative frequency error {:+.2e} (spatial part {:+.2e}), amplitude {:+.1e}, energy drift {:.1e}, {} steps, {:.1} simulated s/wall s",
            mode.zero,
            omega,
            2.0 * std::f64::consts::PI / k / (2.0 * mode.radius / spans as f64),
            stepper.time(),
            measured.phase_error,
            (omega_numerical - omega) / omega,
            (omega_spatial - omega) / omega,
            measured.amplitude_error,
            energy_drift,
            stepper.steps(),
            stepper.time() / wall.max(f64::MIN_POSITIVE),
        );
    }
    println!();
}
