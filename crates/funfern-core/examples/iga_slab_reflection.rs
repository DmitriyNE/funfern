//! IGA feasibility spike 4: materials inside the box patch. A Gaussian pulse
//! crosses a planar step at normal incidence and the reflected energy is
//! compared with the Fresnel value, for a density step (the field stays
//! C¹) and a stiffness step (the field kinks), with the step on a knot
//! line or mid-span, and on a C¹ basis or one with a C⁰ knot at the step.
//! See `docs/spikes/funfern-iga-feasibility-spike.md`.
//!
//! Flags: `--sides 32,64,128 --degree 2 --points 2 --sweeps 2 --sigma 0.1
//! --fraction 0.8`.

use std::time::Instant;

use funfern_core::{
    GeometryMap, Integrator, MassTreatment, PatchStepper, Point2, SplinePatch, UniformBasis,
};

const PULSE_CENTER: f64 = -0.5;
const MEASURE_TIME: f64 = 1.2;

#[derive(Clone, Copy)]
enum Contrast {
    /// Density one to four, stiffness one: wave speed halves, `u` stays C¹.
    Density,
    /// Stiffness one to a quarter, density one: wave speed halves, `u` kinks.
    Stiffness,
}

struct Variant {
    name: &'static str,
    contrast: Contrast,
    /// The step's offset from the knot line at `x = 0`, in spans.
    offset_spans: f64,
    c0_knot: bool,
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let value = |flag: &str, default: usize| {
        args.iter()
            .position(|a| a == flag)
            .and_then(|i| args.get(i + 1))
            .map_or(default, |v| v.parse().unwrap())
    };
    let sides: Vec<usize> = args
        .iter()
        .position(|a| a == "--sides")
        .and_then(|i| args.get(i + 1))
        .map_or(vec![32, 64, 128], |v| {
            v.split(',').map(|s| s.parse().unwrap()).collect()
        });
    let degree = value("--degree", 2);
    let points = value("--points", 2);
    let sweeps = value("--sweeps", 2);
    let real = |flag: &str, default: f64| {
        args.iter()
            .position(|a| a == flag)
            .and_then(|i| args.get(i + 1))
            .map_or(default, |v| v.parse().unwrap())
    };
    let sigma = real("--sigma", 0.1);
    let fraction = real("--fraction", 0.8);
    let fresnel = 1.0 / 9.0;
    println!(
        "A Gaussian pulse (σ {sigma}) from x = {PULSE_CENTER} hits a planar step at x ≈ 0 at normal incidence; impedance 1 → 2 either way, so Fresnel R = 1/9 = {fresnel:.6}.\n\
         Degree {degree}, Gauss {points} × {points}, lumped + {sweeps} sweeps, unclamped bases, fourth-order integrator at {fraction} of its step; energy left of the step at t = {MEASURE_TIME} over the initial energy.\n"
    );
    let variants = [
        Variant {
            name: "density step on a knot line, C¹ basis",
            contrast: Contrast::Density,
            offset_spans: 0.0,
            c0_knot: false,
        },
        Variant {
            name: "density step mid-span (immersed), C¹ basis",
            contrast: Contrast::Density,
            offset_spans: 0.5,
            c0_knot: false,
        },
        Variant {
            name: "density step on a knot line, C⁰ knot there",
            contrast: Contrast::Density,
            offset_spans: 0.0,
            c0_knot: true,
        },
        Variant {
            name: "stiffness step on a knot line, C¹ basis",
            contrast: Contrast::Stiffness,
            offset_spans: 0.0,
            c0_knot: false,
        },
        Variant {
            name: "stiffness step mid-span (immersed), C¹ basis",
            contrast: Contrast::Stiffness,
            offset_spans: 0.5,
            c0_knot: false,
        },
        Variant {
            name: "stiffness step on a knot line, C⁰ knot there",
            contrast: Contrast::Stiffness,
            offset_spans: 0.0,
            c0_knot: true,
        },
    ];
    for variant in &variants {
        println!("{}:", variant.name);
        for &side in &sides {
            run(
                variant, side, degree, points, sweeps, fresnel, sigma, fraction,
            );
        }
        println!();
    }
}

type Coefficient = Box<dyn Fn(Point2) -> f64>;

#[allow(clippy::too_many_arguments)]
fn run(
    variant: &Variant,
    side: usize,
    degree: usize,
    points: usize,
    sweeps: usize,
    fresnel: f64,
    sigma: f64,
    fraction: f64,
) {
    let spans = side - degree;
    assert!(
        spans.is_multiple_of(2),
        "the knot line x = 0 needs an even span count"
    );
    let h = 2.0 / spans as f64;
    let step_x = variant.offset_spans * h;
    let basis_x = if variant.c0_knot {
        UniformBasis::with_c0_knot(degree, spans, h, spans / 2, true)
    } else {
        UniformBasis::unclamped(degree, spans, h)
    };
    let basis_y = UniformBasis::unclamped(degree, spans, h);
    let patch = SplinePatch::from_bases_public(
        basis_x,
        basis_y,
        GeometryMap::Affine {
            origin: Point2::new(-1.0, -1.0),
        },
        points,
    )
    .unwrap();
    let (density, stiffness): (Coefficient, Coefficient) = match variant.contrast {
        Contrast::Density => (
            Box::new(move |p: Point2| if p.x < step_x { 1.0 } else { 4.0 }),
            Box::new(|_| 1.0),
        ),
        Contrast::Stiffness => (
            Box::new(|_| 1.0),
            Box::new(move |p: Point2| if p.x < step_x { 1.0 } else { 0.25 }),
        ),
    };
    let patch = patch.with_material(density, stiffness);
    let treatment = MassTreatment::Lumped { sweeps };
    let integrator = Integrator::FourthOrder;
    let (lambda, _) = patch.largest_eigenvalue(treatment, 1500);
    let dt = fraction * integrator.stability_factor() * 2.0 / lambda.sqrt();
    let pulse = |x: f64| (-((x - PULSE_CENTER) / sigma).powi(2)).exp();
    // A right-moving pulse at unit speed: `b_x = -u / c` at the same instant.
    let u0 = patch.project(|p| pulse(p.x));
    let b0: Vec<Point2> = patch
        .sample_points()
        .iter()
        .map(|p| Point2::new(-pulse(p.x), 0.0))
        .collect();
    let energy_left = |u: &[f64], b: &[Point2]| -> (f64, f64) {
        let values = patch.sample_values(u);
        let mut left = 0.0;
        let mut total = 0.0;
        for ((((value, flux), weight), density), stiffness) in values
            .iter()
            .zip(b)
            .zip(patch.sample_weights())
            .zip(patch.sample_density())
            .zip(patch.sample_stiffness())
        {
            let energy = 0.5 * weight * (density * value * value + stiffness * flux.dot(*flux));
            total += energy;
            // The sample's side of the step, read from its own density or
            // stiffness so an immersed step counts each sample where it is.
            let left_side = match variant.contrast {
                Contrast::Density => *density < 2.0,
                Contrast::Stiffness => *stiffness > 0.5,
            };
            if left_side {
                left += energy;
            }
        }
        (left, total)
    };
    let (_, initial) = energy_left(&u0, &b0);
    let mut stepper = PatchStepper::with_state(&patch, treatment, integrator, dt, u0, Some(b0));
    let steps = (MEASURE_TIME / dt).round() as u64;
    let start = Instant::now();
    while stepper.steps() < steps {
        stepper.step();
    }
    let wall = start.elapsed().as_secs_f64();
    let centered = stepper.centered_flux();
    let (left, total) = energy_left(stepper.field(), &centered);
    let reflected = left / initial;
    println!(
        "  side {side:>3} ({:>5} DOFs, h {:.4}, {:.1} spans per σ): R {:.6}, error {:+.2e} ({:+.2e} relative), energy {:+.1e}, {} steps, dt {:.2e}, {:.1} s",
        patch.degrees_of_freedom(),
        h,
        sigma / h,
        reflected,
        reflected - fresnel,
        (reflected - fresnel) / fresnel,
        total / initial - 1.0,
        stepper.steps(),
        dt,
        wall,
    );
}
