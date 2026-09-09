use std::time::Instant;

use femfun_core::{
    MeshingOptions, Point2, Scene, TriMesh, WaveCoefficients, WaveOperator, WaveState, mesh_scene,
};

const WAVELENGTH: f64 = 0.4;
const WAVE_NUMBER: f64 = std::f64::consts::TAU / WAVELENGTH;

struct Measurement {
    target_time: f64,
    actual_time: f64,
    phase_error: f64,
    amplitude_error: f64,
    relative_l2: f64,
    steps: u64,
    wall_seconds: f64,
}

fn main() {
    println!("P1 reflecting-box mode cos(5π(x+1)); wavelength {WAVELENGTH}, wave speed 1");
    println!("Errors use the actual reached time; timing is the f64 CPU reference path.\n");
    for requested_edge in [0.04, 0.02] {
        let mesh_start = Instant::now();
        let mesh = mesh_scene(
            &Scene::default(),
            0,
            MeshingOptions {
                curve_tolerance: (requested_edge * 0.02_f64).min(1.5e-3),
                target_edge_length: requested_edge / 1.05,
                minimum_angle_degrees: 12.0,
                max_vertices: 50_000,
                max_triangles: 100_000,
                max_refinement_steps: 50_000,
            },
        )
        .unwrap_or_else(|error| panic!("h={requested_edge}: {error}"));
        let mesh_seconds = mesh_start.elapsed().as_secs_f64();
        let assembly_start = Instant::now();
        let operator = WaveOperator::assemble(&mesh, WaveCoefficients::default())
            .unwrap_or_else(|error| panic!("h={requested_edge}: {error}"));
        let assembly_seconds = assembly_start.elapsed().as_secs_f64();
        let gpu_bytes = operator.row_offsets().len() * 4
            + operator.columns().len() * 8
            + operator.degrees_of_freedom() * 32;
        println!(
            "h≤{requested_edge:.2}: {} DOFs, {} triangles, actual h {:.6}, λ/h {:.1}, GPU buffers {:.2} MiB",
            operator.degrees_of_freedom(),
            mesh.triangles.len(),
            mesh.quality.maximum_edge_length,
            WAVELENGTH / mesh.quality.maximum_edge_length,
            gpu_bytes as f64 / (1024.0 * 1024.0),
        );
        println!(
            "  mesh {:.3} s, operator {:.3} ms, conservative dt max {:.7}",
            mesh_seconds,
            assembly_seconds * 1000.0,
            operator.maximum_time_step(),
        );
        for time_step_fraction in [0.45, 0.225] {
            let dt = time_step_fraction * operator.maximum_time_step();
            for target_time in [2.0, 10.0] {
                let result = measure(&mesh, &operator, dt, target_time);
                println!(
                    "  dt={:.4} dt_max, target {:>4.1}, reached {:.5}: phase {:+.4e} rad, amplitude {:+.4e}, L2 {:.4e}, {} steps, {:.2} simulated s/wall s",
                    time_step_fraction,
                    result.target_time,
                    result.actual_time,
                    result.phase_error,
                    result.amplitude_error,
                    result.relative_l2,
                    result.steps,
                    result.actual_time / result.wall_seconds.max(f64::MIN_POSITIVE),
                );
            }
        }
        println!();
    }
}

fn measure(
    mesh: &TriMesh,
    operator: &WaveOperator,
    time_step: f64,
    target_time: f64,
) -> Measurement {
    let mode: Vec<_> = mesh
        .vertices
        .iter()
        .map(|vertex| analytic_mode(vertex.point))
        .collect();
    let mut state = WaveState::new(
        operator,
        time_step,
        mode.clone(),
        vec![0.0; operator.degrees_of_freedom()],
    )
    .unwrap();
    let target_steps = (target_time / time_step).round().max(1.0) as u64;
    let start = Instant::now();
    for _ in 0..target_steps {
        state.step(operator, &[]).unwrap();
    }
    let wall_seconds = start.elapsed().as_secs_f64();
    let actual_time = state.time();
    let norm = mode
        .iter()
        .zip(operator.lumped_mass())
        .map(|(mode, mass)| mass * mode * mode)
        .sum::<f64>();
    let project = |values: &[f64]| {
        values
            .iter()
            .zip(&mode)
            .zip(operator.lumped_mass())
            .map(|((value, mode), mass)| mass * value * mode)
            .sum::<f64>()
            / norm
    };
    let current_mode = project(state.current());
    let previous_mode = project(state.previous());
    let angle = WAVE_NUMBER * time_step;
    let sine = if angle.sin().abs() > 1.0e-12 {
        (previous_mode - current_mode * angle.cos()) / angle.sin()
    } else {
        0.0
    };
    let numerical_phase = sine.atan2(current_mode);
    let exact_phase = WAVE_NUMBER * actual_time;
    let phase_error = wrap_angle(numerical_phase - exact_phase);
    let amplitude_error = current_mode.hypot(sine) - 1.0;
    let exact_factor = exact_phase.cos();
    let relative_l2 = (state
        .current()
        .iter()
        .zip(&mode)
        .zip(operator.lumped_mass())
        .map(|((actual, mode), mass)| mass * (actual - exact_factor * mode).powi(2))
        .sum::<f64>()
        / norm)
        .sqrt();
    Measurement {
        target_time,
        actual_time,
        phase_error,
        amplitude_error,
        relative_l2,
        steps: state.steps(),
        wall_seconds,
    }
}

fn analytic_mode(point: Point2) -> f64 {
    (WAVE_NUMBER * (point.x + 1.0)).cos()
}

fn wrap_angle(angle: f64) -> f64 {
    (angle + std::f64::consts::PI).rem_euclid(std::f64::consts::TAU) - std::f64::consts::PI
}
