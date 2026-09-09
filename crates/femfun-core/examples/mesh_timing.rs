//! Native CPU timings; run with `cargo run -p femfun-core --release --example mesh_timing`.
use femfun_core::*;
use std::collections::BTreeMap;
use std::time::Instant;

fn main() {
    let frame_slices = std::env::args().any(|arg| arg == "--slices");
    for count in [1, 8, 32] {
        let scene = if count == 1 {
            Scene::initial()
        } else {
            let columns = if count == 8 { 4 } else { 8 };
            let rows = count / columns;
            Scene {
                obstacles: (0..count)
                    .map(|i| Obstacle {
                        id: ObstacleId(i as u64 + 1),
                        spline: PeriodicCubicSpline::rounded(
                            Point2::new(
                                -0.85 + 1.7 * (i % columns) as f64 / (columns - 1) as f64,
                                -0.65 + 1.3 * (i / columns) as f64 / (rows - 1) as f64,
                            ),
                            if count == 8 { 0.12 } else { 0.07 },
                        ),
                    })
                    .collect(),
            }
        };
        let options = MeshingOptions {
            curve_tolerance: 1.5e-3,
            target_edge_length: 0.16,
            minimum_angle_degrees: 12.0,
            max_vertices: 8_000,
            max_triangles: 16_000,
            max_refinement_steps: 5_000,
        };
        let start = Instant::now();
        let mut job = MeshingJob::new(scene, 1, options);
        let mut maximum = 0.0_f64;
        let mut calls = 0;
        let mut phases = BTreeMap::<&str, (f64, f64)>::new();
        loop {
            let phase = job.phase();
            let slice = Instant::now();
            let mut result = job.advance(1);
            if frame_slices {
                for _ in 1..100_000 {
                    if result.is_some() || slice.elapsed().as_secs_f64() >= 0.002 {
                        break;
                    }
                    result = job.advance(1);
                }
            }
            let ms = slice.elapsed().as_secs_f64() * 1000.0;
            maximum = maximum.max(ms);
            if !frame_slices {
                let timing = phases.entry(phase).or_default();
                timing.0 += ms;
                timing.1 = timing.1.max(ms);
            }
            calls += 1;
            if let Some(result) = result {
                let mesh = result.expect("benchmark scene must mesh");
                println!(
                    "{count} obstacles: {:.2} ms total, {maximum:.3} ms max call, {calls} calls, {} vertices, {} triangles",
                    start.elapsed().as_secs_f64() * 1000.0,
                    mesh.vertices.len(),
                    mesh.triangles.len(),
                );
                for (phase, (total, max)) in phases {
                    println!("  {phase}: {total:.2} ms work, {max:.3} ms max unit");
                }
                println!("  {:?}", job.stats());
                break;
            }
        }
    }
}
