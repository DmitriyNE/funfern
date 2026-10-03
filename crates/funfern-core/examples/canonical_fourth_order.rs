//! Fourth-order step, stage A: the reflecting-box mode `cos(5π(x+1))` of
//! `wave_convergence` on the production CPU path (`CanonicalWaveState`), P2e
//! at the app's parent edge 0.08, by integrator and step fraction.
//!
//! The phase is read by a least-squares fit of the mode's amplitude to
//! `A cos ωt + B sin ωt` over one period centred on the target time, which
//! needs no prediction of the discrete frequency.
//!
//! Flags: `--edge 0.08 --fractions 0.9,0.45,0.225 --targets 2,10
//! --handoffs 25`. With `--handoffs N` the state is handed to a fresh
//! stepper every `N` steps, alternating the step between the fraction and
//! three quarters of it, as a live drag's generations do.

use std::time::Instant;

use funfern_core::{
    CanonicalIntegrator, CanonicalWaveOperator, CanonicalWaveState, MeshingOptions,
    OuterBoundaryCondition, QuadraticWaveOperator, Scene, mesh_scene,
};

const WAVELENGTH: f64 = 0.4;
const WAVE_NUMBER: f64 = std::f64::consts::TAU / WAVELENGTH;

fn flag(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

fn list(text: &str) -> Vec<f64> {
    text.split(',').map(|v| v.parse().unwrap()).collect()
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let edge: f64 = flag(&args, "--edge").map_or(0.08, |v| v.parse().unwrap());
    let fractions = flag(&args, "--fractions").map_or(vec![0.9, 0.45, 0.225], |v| list(&v));
    let targets = flag(&args, "--targets").map_or(vec![2.0, 10.0], |v| list(&v));
    let handoffs: Option<u64> = flag(&args, "--handoffs").map(|v| v.parse().unwrap());

    let scene = Scene::default();
    let mesh = mesh_scene(
        &scene,
        0,
        MeshingOptions {
            curve_tolerance: (edge * 0.02_f64).min(1.5e-3),
            target_edge_length: edge / 1.05,
            minimum_angle_degrees: 12.0,
            max_vertices: 200_000,
            max_triangles: 400_000,
            max_refinement_steps: 200_000,
        },
    )
    .expect("box mesh");
    let quadratic =
        QuadraticWaveOperator::assemble_scene(&mesh, &scene, OuterBoundaryCondition::Reflecting)
            .expect("quadratic operator");
    let operator =
        CanonicalWaveOperator::compile_scene(&mesh, &quadratic, &scene, 1).expect("operator");
    let mode = operator
        .node_points()
        .iter()
        .map(|p| (WAVE_NUMBER * (p.x + 1.0)).cos())
        .collect::<Vec<_>>();
    let mass = operator.primary_mass().to_vec();
    let norm = mode
        .iter()
        .zip(&mass)
        .map(|(phi, m)| m * phi * phi)
        .sum::<f64>();
    println!(
        "Reflecting-box mode cos(5π(x+1)), wavelength {WAVELENGTH}, P2e parent edge {edge}: {} DOFs, dt_max {:.5e} (app runs 0.9 of it)\n",
        operator.degrees_of_freedom(),
        operator.maximum_time_step()
    );
    for fraction in fractions {
        for integrator in [
            CanonicalIntegrator::Leapfrog,
            CanonicalIntegrator::FourthOrder,
        ] {
            let base_dt = fraction * operator.maximum_time_step();
            let mut dt = base_dt;
            let mut state = CanonicalWaveState::from_primary_velocity(
                &operator,
                dt,
                &mode,
                &vec![0.0; mode.len()],
            )
            .expect("state")
            .with_integrator(integrator);
            let initial_energy = state.energy(&operator).unwrap();
            let period = WAVELENGTH;
            let mut line = format!("{integrator:?} dt={fraction} dt_max:");
            let mut wall = 0.0;
            let mut time = 0.0;
            let mut steps = 0u64;
            let mut advance = |state: &mut CanonicalWaveState, time: &mut f64| {
                state.step(&operator).unwrap();
                *time += dt;
                steps += 1;
                if handoffs.is_some_and(|every| steps.is_multiple_of(every)) {
                    dt = if dt == base_dt {
                        0.75 * base_dt
                    } else {
                        base_dt
                    };
                    *state = CanonicalWaveState::new(
                        &operator,
                        dt,
                        state.primary_flux().to_vec(),
                        state.complementary_flux().to_vec(),
                    )
                    .unwrap()
                    .with_integrator(integrator);
                }
            };
            for &target in &targets {
                let start = target - 0.5 * period;
                let end = target + 0.5 * period;
                let mark = Instant::now();
                while time < start {
                    advance(&mut state, &mut time);
                }
                wall += mark.elapsed().as_secs_f64();
                // Normal equations of the fit a ≈ A cos ωt + B sin ωt.
                let (mut cc, mut cs, mut ss, mut ac, mut as_) = (0.0, 0.0, 0.0, 0.0, 0.0);
                loop {
                    let t = time;
                    let field = state.primary_field(&operator).unwrap();
                    let a = field
                        .iter()
                        .zip(&mode)
                        .zip(&mass)
                        .map(|((u, phi), m)| m * u * phi)
                        .sum::<f64>()
                        / norm;
                    let (c, s) = ((WAVE_NUMBER * t).cos(), (WAVE_NUMBER * t).sin());
                    cc += c * c;
                    cs += c * s;
                    ss += s * s;
                    ac += a * c;
                    as_ += a * s;
                    if time >= end {
                        break;
                    }
                    let mark = Instant::now();
                    advance(&mut state, &mut time);
                    wall += mark.elapsed().as_secs_f64();
                }
                let det = cc * ss - cs * cs;
                let a_coefficient = (ac * ss - as_ * cs) / det;
                let b_coefficient = (as_ * cc - ac * cs) / det;
                let phase = -b_coefficient.atan2(a_coefficient);
                let amplitude = a_coefficient.hypot(b_coefficient) - 1.0;
                line +=
                    &format!("  t={target}: phase {phase:+.4e} rad, amplitude {amplitude:+.2e};");
            }
            let drift = (state.energy(&operator).unwrap() - initial_energy).abs() / initial_energy;
            println!(
                "{line} energy drift {drift:.1e}, {:.2} simulated s/wall s",
                time / wall
            );
        }
    }
}
