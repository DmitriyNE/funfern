use std::time::Instant;

use funfern_core::{
    MeshingOptions, Point2, QuadraticWaveOperator, QuadraticWaveState, Scene, TriMesh,
    WaveCoefficients, WaveOperator, WaveState, mesh_scene,
};

const WAVELENGTH: f64 = 0.4;
const WAVE_NUMBER: f64 = std::f64::consts::TAU / WAVELENGTH;
const TIME_STEP_FRACTIONS: [f64; 2] = [0.225, 0.1125];
const TARGET_TIMES: [f64; 2] = [2.0, 10.0];

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
    println!("Reflecting-box mode cos(5π(x+1)); wavelength {WAVELENGTH}, wave speed 1");
    println!("P2e is the seven-node P2 + cubic-bubble mass-lumped triangle; timings are f64 CPU.");
    println!("Errors use the actual reached time. Parent h is the triangulation edge bound.\n");

    for requested_edge in [0.04, 0.02] {
        let (mesh, mesh_seconds) = make_mesh(requested_edge);
        benchmark_p1(&mesh, mesh_seconds, requested_edge);
    }
    for requested_edge in [0.08, 0.04] {
        let (mesh, mesh_seconds) = make_mesh(requested_edge);
        benchmark_p2e(&mesh, mesh_seconds, requested_edge);
    }
}

fn make_mesh(requested_edge: f64) -> (TriMesh, f64) {
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
    (mesh, mesh_start.elapsed().as_secs_f64())
}

fn benchmark_p1(mesh: &TriMesh, mesh_seconds: f64, requested_edge: f64) {
    let assembly_start = Instant::now();
    let operator = WaveOperator::assemble(mesh, WaveCoefficients::default())
        .unwrap_or_else(|error| panic!("P1 h={requested_edge}: {error}"));
    let assembly_seconds = assembly_start.elapsed().as_secs_f64();
    let gpu_bytes = operator.row_offsets().len() * 4
        + operator.columns().len() * 8
        + operator.degrees_of_freedom() * 32;
    print_case_header(
        "P1",
        mesh,
        requested_edge,
        operator.degrees_of_freedom(),
        gpu_bytes,
        mesh_seconds,
        assembly_seconds,
        operator.maximum_time_step(),
    );
    for time_step_fraction in TIME_STEP_FRACTIONS {
        let dt = time_step_fraction * operator.maximum_time_step();
        for target_time in TARGET_TIMES {
            print_measurement(
                time_step_fraction,
                measure_p1(mesh, &operator, dt, target_time),
            );
        }
    }
    println!();
}

fn benchmark_p2e(mesh: &TriMesh, mesh_seconds: f64, requested_edge: f64) {
    let assembly_start = Instant::now();
    let operator = QuadraticWaveOperator::assemble(mesh, WaveCoefficients::default())
        .unwrap_or_else(|error| panic!("P2e h={requested_edge}: {error}"));
    let assembly_seconds = assembly_start.elapsed().as_secs_f64();
    print_case_header(
        "P2e",
        mesh,
        requested_edge,
        operator.degrees_of_freedom(),
        operator.estimated_gpu_bytes(),
        mesh_seconds,
        assembly_seconds,
        operator.maximum_time_step(),
    );
    for time_step_fraction in TIME_STEP_FRACTIONS {
        let dt = time_step_fraction * operator.maximum_time_step();
        for target_time in TARGET_TIMES {
            print_measurement(time_step_fraction, measure_p2e(&operator, dt, target_time));
        }
    }
    println!();
}

#[allow(clippy::too_many_arguments)]
fn print_case_header(
    family: &str,
    mesh: &TriMesh,
    requested_edge: f64,
    degrees_of_freedom: usize,
    gpu_bytes: usize,
    mesh_seconds: f64,
    assembly_seconds: f64,
    maximum_time_step: f64,
) {
    println!(
        "{family} parent h≤{requested_edge:.2}: {degrees_of_freedom} DOFs, {} triangles, actual h {:.6}, parent λ/h {:.1}, estimated GPU buffers {:.2} MiB",
        mesh.triangles.len(),
        mesh.quality.maximum_edge_length,
        WAVELENGTH / mesh.quality.maximum_edge_length,
        gpu_bytes as f64 / (1024.0 * 1024.0),
    );
    println!(
        "  mesh {:.3} s, operator {:.3} ms, conservative dt max {:.7}",
        mesh_seconds,
        assembly_seconds * 1000.0,
        maximum_time_step,
    );
}

fn print_measurement(time_step_fraction: f64, result: Measurement) {
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

fn measure_p1(
    mesh: &TriMesh,
    operator: &WaveOperator,
    time_step: f64,
    target_time: f64,
) -> Measurement {
    let mode = mesh
        .vertices
        .iter()
        .map(|vertex| analytic_mode(vertex.point))
        .collect::<Vec<_>>();
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
    analyze(
        target_time,
        state.time(),
        state.steps(),
        start.elapsed().as_secs_f64(),
        time_step,
        &mode,
        operator.lumped_mass(),
        state.current(),
        state.previous(),
    )
}

fn measure_p2e(operator: &QuadraticWaveOperator, time_step: f64, target_time: f64) -> Measurement {
    let mode = operator
        .node_points()
        .iter()
        .map(|point| analytic_mode(*point))
        .collect::<Vec<_>>();
    let mut state = QuadraticWaveState::new(
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
    analyze(
        target_time,
        state.time(),
        state.steps(),
        start.elapsed().as_secs_f64(),
        time_step,
        &mode,
        operator.lumped_mass(),
        state.current(),
        state.previous(),
    )
}

#[allow(clippy::too_many_arguments)]
fn analyze(
    target_time: f64,
    actual_time: f64,
    steps: u64,
    wall_seconds: f64,
    time_step: f64,
    mode: &[f64],
    mass: &[f64],
    current: &[f64],
    previous: &[f64],
) -> Measurement {
    let norm = mode
        .iter()
        .zip(mass)
        .map(|(mode, mass)| mass * mode * mode)
        .sum::<f64>();
    let project = |values: &[f64]| {
        values
            .iter()
            .zip(mode)
            .zip(mass)
            .map(|((value, mode), mass)| mass * value * mode)
            .sum::<f64>()
            / norm
    };
    let current_mode = project(current);
    let previous_mode = project(previous);
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
    let relative_l2 = (current
        .iter()
        .zip(mode)
        .zip(mass)
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
        steps,
        wall_seconds,
    }
}

fn analytic_mode(point: Point2) -> f64 {
    (WAVE_NUMBER * (point.x + 1.0)).cos()
}

fn wrap_angle(angle: f64) -> f64 {
    (angle + std::f64::consts::PI).rem_euclid(std::f64::consts::TAU) - std::f64::consts::PI
}
