use std::time::Instant;

use funfern_core::{
    MeshingOptions, OuterBoundaryCondition, Point2, QuadraticWaveOperator, QuadraticWaveState,
    Scene, WaveCoefficients, mesh_scene,
};

const TEMPORAL_FRACTION: f64 = 0.225;

#[derive(Clone, Copy)]
struct Packet {
    wavelength: f64,
    angle_degrees: f64,
    origin: Point2,
    longitudinal_width: f64,
    transverse_width: f64,
}

fn main() {
    println!("Outer-boundary reflection benchmark");
    println!("P2e finite Gaussian packets, wave speed 1, dt={TEMPORAL_FRACTION} dt_max");
    println!("Measured |R| is sqrt(outgoing residual energy / reflecting residual energy).");
    println!("Ideal |R| is the corresponding continuous plane-wave result.\n");

    for (wavelength, parent_edge, width) in [(0.4, 0.08, 0.24), (0.2, 0.04, 0.18)] {
        let start = Instant::now();
        let mesh = mesh_scene(
            &Scene::default(),
            0,
            MeshingOptions {
                curve_tolerance: (parent_edge * 0.02_f64).min(1.5e-3),
                target_edge_length: parent_edge / 1.05,
                minimum_angle_degrees: 12.0,
                max_vertices: 50_000,
                max_triangles: 100_000,
                max_refinement_steps: 50_000,
            },
        )
        .unwrap_or_else(|error| panic!("h={parent_edge}: {error}"));
        let reflecting = QuadraticWaveOperator::assemble_with_boundary(
            &mesh,
            WaveCoefficients::default(),
            OuterBoundaryCondition::Reflecting,
        )
        .unwrap();
        let outgoing = [
            OuterBoundaryCondition::FirstOrderOutgoing,
            OuterBoundaryCondition::SecondOrderOutgoing,
        ]
        .map(|boundary| {
            QuadraticWaveOperator::assemble_with_boundary(
                &mesh,
                WaveCoefficients::default(),
                boundary,
            )
            .unwrap()
        });
        println!(
            "λ={wavelength:.2}, parent h≤{parent_edge:.2}: {} DOFs, {} triangles, mesh/operators {:.3} s",
            reflecting.degrees_of_freedom(),
            mesh.triangles.len(),
            start.elapsed().as_secs_f64()
        );

        for (angle_degrees, origin_y) in [(0.0, 0.0), (30.0, -0.45)] {
            let packet = Packet {
                wavelength,
                angle_degrees,
                origin: Point2::new(0.0, origin_y),
                longitudinal_width: width,
                transverse_width: 0.16,
            };
            let angle = angle_degrees.to_radians();
            let hit_time = 1.0 / angle.cos();
            let target_time = hit_time + 3.5 * width;
            for operator in &outgoing {
                let measurement = measure(packet, target_time, &reflecting, operator);
                let cosine = angle.cos();
                let impedance = match operator.outer_boundary() {
                    OuterBoundaryCondition::FirstOrderOutgoing => 1.0,
                    OuterBoundaryCondition::SecondOrderOutgoing => 1.0 - 0.5 * angle.sin().powi(2),
                    OuterBoundaryCondition::Reflecting
                    | OuterBoundaryCondition::Neumann { .. }
                    | OuterBoundaryCondition::Dirichlet { .. } => unreachable!(),
                };
                let ideal = ((cosine - impedance) / (cosine + impedance)).abs();
                println!(
                    "  {:<22} angle {angle_degrees:>4.0}°, t={:.3}: |R| measured {:.4}, ideal {:.4}; energy {:.3e}, reflecting {:.6}",
                    operator.outer_boundary().label(),
                    measurement.time,
                    measurement.reflection_amplitude,
                    ideal,
                    measurement.outgoing_energy_ratio,
                    measurement.reflecting_energy_ratio,
                );
            }
        }
        println!();
    }

    let long_time = long_time_check();
    println!(
        "Long-time second-order normal packet at λ=.40, t={:.3}: energy ratio {:.3e}, finite {}",
        long_time.time, long_time.energy_ratio, long_time.finite
    );
}

struct Measurement {
    time: f64,
    reflection_amplitude: f64,
    outgoing_energy_ratio: f64,
    reflecting_energy_ratio: f64,
}

fn measure(
    packet: Packet,
    target_time: f64,
    reflecting: &QuadraticWaveOperator,
    outgoing: &QuadraticWaveOperator,
) -> Measurement {
    let dt = TEMPORAL_FRACTION * reflecting.maximum_time_step();
    let (displacement, velocity) = initial_packet(reflecting, packet);
    let mut reflecting_state =
        QuadraticWaveState::new(reflecting, dt, displacement.clone(), velocity.clone()).unwrap();
    let mut outgoing_state = QuadraticWaveState::new(outgoing, dt, displacement, velocity).unwrap();
    let reflecting_initial = reflecting_state.energy(reflecting).unwrap();
    let outgoing_initial = outgoing_state.energy(outgoing).unwrap();
    let steps = (target_time / dt).round().max(1.0) as u64;
    for _ in 0..steps {
        reflecting_state.step(reflecting, &[]).unwrap();
        outgoing_state.step(outgoing, &[]).unwrap();
    }
    let reflecting_ratio = reflecting_state.energy(reflecting).unwrap() / reflecting_initial;
    let outgoing_ratio = outgoing_state.energy(outgoing).unwrap() / outgoing_initial;
    Measurement {
        time: outgoing_state.time(),
        reflection_amplitude: (outgoing_ratio / reflecting_ratio).max(0.0).sqrt(),
        outgoing_energy_ratio: outgoing_ratio,
        reflecting_energy_ratio: reflecting_ratio,
    }
}

struct LongTimeMeasurement {
    time: f64,
    energy_ratio: f64,
    finite: bool,
}

fn long_time_check() -> LongTimeMeasurement {
    let mesh = mesh_scene(
        &Scene::default(),
        1,
        MeshingOptions {
            curve_tolerance: 1.5e-3,
            target_edge_length: 0.08 / 1.05,
            minimum_angle_degrees: 12.0,
            max_vertices: 50_000,
            max_triangles: 100_000,
            max_refinement_steps: 50_000,
        },
    )
    .unwrap();
    let operator = QuadraticWaveOperator::assemble_with_boundary(
        &mesh,
        WaveCoefficients::default(),
        OuterBoundaryCondition::SecondOrderOutgoing,
    )
    .unwrap();
    let packet = Packet {
        wavelength: 0.4,
        angle_degrees: 0.0,
        origin: Point2::new(0.0, 0.0),
        longitudinal_width: 0.24,
        transverse_width: 0.16,
    };
    let (displacement, velocity) = initial_packet(&operator, packet);
    let dt = TEMPORAL_FRACTION * operator.maximum_time_step();
    let mut state = QuadraticWaveState::new(&operator, dt, displacement, velocity).unwrap();
    let initial = state.energy(&operator).unwrap();
    let steps = (10.0 / dt).round() as u64;
    for _ in 0..steps {
        state.step(&operator, &[]).unwrap();
    }
    let energy = state.energy(&operator).unwrap();
    LongTimeMeasurement {
        time: state.time(),
        energy_ratio: energy / initial,
        finite: energy.is_finite() && state.current().iter().all(|value| value.is_finite()),
    }
}

fn initial_packet(operator: &QuadraticWaveOperator, packet: Packet) -> (Vec<f64>, Vec<f64>) {
    let angle = packet.angle_degrees.to_radians();
    let direction = Point2::new(angle.cos(), angle.sin());
    let transverse = Point2::new(-angle.sin(), angle.cos());
    let wave_number = std::f64::consts::TAU / packet.wavelength;
    operator
        .node_points()
        .iter()
        .map(|point| {
            let offset = *point - packet.origin;
            let s = offset.dot(direction);
            let q = offset.dot(transverse);
            let envelope = (-0.5
                * (s * s / packet.longitudinal_width.powi(2)
                    + q * q / packet.transverse_width.powi(2)))
            .exp();
            let phase = wave_number * s;
            let displacement = envelope * phase.cos();
            let velocity = envelope
                * (s / packet.longitudinal_width.powi(2) * phase.cos() + wave_number * phase.sin());
            (displacement, velocity)
        })
        .unzip()
}
