//! Native Stage 2 timing harness for the linear direct `(Q, b)` CPU core.
//! It measures the real enriched-quadratic compile and KDK path; it is not a
//! prediction of the later f32 GPU implementation.

use std::time::Instant;

use funfern_core::{
    CanonicalWaveOperator, CanonicalWaveState, MeshingOptions, OuterBoundaryCondition,
    QuadraticWaveOperator, Scene, mesh_scene,
};

fn main() {
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
    let quadratic =
        QuadraticWaveOperator::assemble_scene(&mesh, &scene, OuterBoundaryCondition::Reflecting)
            .expect("scalar comparison operator");

    let compile_start = Instant::now();
    let operator = CanonicalWaveOperator::compile_scene(&mesh, &quadratic, &scene, 1)
        .expect("canonical operator");
    let compile_time = compile_start.elapsed();
    let dt = 0.9 * operator.maximum_time_step();
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

    const STEPS: usize = 128;
    let step_start = Instant::now();
    for _ in 0..STEPS {
        state.step(&operator).expect("finite canonical step");
    }
    let step_time = step_start.elapsed();
    let simulated = dt * STEPS as f64;
    println!(
        "{} DOFs, {} elements, {} vector samples",
        operator.degrees_of_freedom(),
        operator.element_nodes().len(),
        operator.complementary_degrees_of_freedom()
    );
    println!(
        "compile {:.2} ms, operator {:.2} MiB, state {:.2} MiB",
        compile_time.as_secs_f64() * 1_000.0,
        operator.estimated_operator_bytes() as f64 / (1024.0 * 1024.0),
        state.estimated_state_bytes() as f64 / (1024.0 * 1024.0)
    );
    println!(
        "{STEPS} KDK steps {:.2} ms, {:.2} simulated seconds per wall second, energy {:.9}",
        step_time.as_secs_f64() * 1_000.0,
        simulated / step_time.as_secs_f64(),
        state.energy(&operator).expect("finite energy")
    );
}
