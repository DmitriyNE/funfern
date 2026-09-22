//! Native Stage 2/3 timing harness for the linear direct `(Q, b)` CPU core.
//! Pass `--second-order` to include passive boundary preparation and trace
//! solves. This is a CPU correctness-core measurement, not a prediction of the
//! later f32 GPU implementation.

use std::time::Instant;

use funfern_core::{
    CanonicalWaveOperator, CanonicalWaveState, MeshingOptions, OuterBoundaryCondition,
    QuadraticWaveOperator, Scene, mesh_scene,
};

fn main() {
    let second_order = std::env::args().any(|argument| argument == "--second-order");
    let scene = Scene::initial();
    let mesh = mesh_scene(
        &scene,
        1,
        MeshingOptions {
            target_edge_length: 0.08,
            ..MeshingOptions::default()
        },
    )
    .expect("standard timing mesh");
    let boundary = if second_order {
        OuterBoundaryCondition::SecondOrderOutgoing
    } else {
        OuterBoundaryCondition::Reflecting
    };
    let quadratic = QuadraticWaveOperator::assemble_scene(&mesh, &scene, boundary)
        .expect("scalar comparison operator");

    let compile_start = Instant::now();
    let operator = CanonicalWaveOperator::compile_scene(&mesh, &quadratic, &scene, 1)
        .expect("canonical operator");
    let compile_time = compile_start.elapsed();
    let dt = 0.9 * operator.maximum_time_step();
    let factor_start = Instant::now();
    let factor_state = CanonicalWaveState::zero(&operator, dt).expect("boundary factor");
    let factor_time = factor_start.elapsed();
    let factor_measurement = factor_state.outgoing_midpoint_factor().map(|factor| {
        let right = (0..factor.dimension())
            .map(|index| (0.13 * index as f64 + 0.2).sin())
            .collect::<Vec<_>>();
        let repetitions = 64;
        let solve_start = Instant::now();
        let mut checksum = 0.0;
        for _ in 0..repetitions {
            checksum += factor
                .solve(
                    operator.outgoing_boundary().unwrap(),
                    operator.primary_mass(),
                    &right,
                )
                .expect("trace solve")[0];
        }
        (
            solve_start.elapsed().as_secs_f64() * 1.0e6 / repetitions as f64,
            factor.estimated_bytes(),
            checksum,
        )
    });
    let primary = operator
        .node_points()
        .iter()
        .map(|point| (-80.0 * ((point.x + 0.45).powi(2) + point.y.powi(2))).exp())
        .collect::<Vec<_>>();
    let mut state = CanonicalWaveState::from_primary_velocity(
        &operator,
        dt,
        &primary,
        &vec![0.0; operator.degrees_of_freedom()],
    )
    .expect("compatible initial state");

    let steps = 128;
    let step_start = Instant::now();
    for _ in 0..steps {
        state.step(&operator).expect("finite canonical step");
    }
    let step_time = step_start.elapsed();
    let simulated = dt * steps as f64;
    println!(
        "{} DOFs, {} elements, {} vector samples",
        operator.degrees_of_freedom(),
        operator.element_nodes().len(),
        operator.complementary_degrees_of_freedom()
    );
    println!(
        "compile {:.2} ms, state/factor prepare {:.2} ms, operator {:.2} MiB, state {:.2} MiB",
        compile_time.as_secs_f64() * 1_000.0,
        factor_time.as_secs_f64() * 1_000.0,
        operator.estimated_operator_bytes() as f64 / (1024.0 * 1024.0),
        state.estimated_state_bytes() as f64 / (1024.0 * 1024.0)
    );
    println!(
        "{steps} KDK steps {:.2} ms, {:.2} simulated seconds per wall second, energy {:.9}",
        step_time.as_secs_f64() * 1_000.0,
        simulated / step_time.as_secs_f64(),
        state.energy(&operator).expect("finite energy")
    );
    if let Some(boundary) = operator.outgoing_boundary() {
        let (solve_us, factor_bytes, checksum) = factor_measurement.unwrap();
        println!(
            "second-order trace {} DOFs, {} auxiliary scalars, {:.2} MiB modal transform, {:.2} MiB factor, {:.2} us/solve ({checksum:.3})",
            boundary.trace_nodes().len(),
            boundary.auxiliary_count(),
            boundary.dense_transform_bytes() as f64 / (1024.0 * 1024.0),
            factor_bytes as f64 / (1024.0 * 1024.0),
            solve_us,
        );
    }
}
