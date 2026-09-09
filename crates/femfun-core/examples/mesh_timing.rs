//! Native CPU timings; run with `cargo run -p femfun-core --release --example mesh_timing`.
use femfun_core::*;
use std::collections::BTreeMap;
use std::time::Instant;

fn main() -> std::process::ExitCode {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args
        .iter()
        .any(|arg| !matches!(arg.as_str(), "--slices" | "--wave"))
    {
        eprintln!("Usage: mesh_timing [--wave] [--slices]");
        return std::process::ExitCode::FAILURE;
    }
    let mut failed = false;
    let frame_slices = args.iter().any(|arg| arg == "--slices");
    let wave = args.iter().any(|arg| arg == "--wave");
    let resolutions: &[f64] = if wave { &[0.04, 0.02] } else { &[0.16] };
    println!("Meshing only; P1 DOFs = vertex count. No wave operator or time stepping is timed.");
    for &max_edge in resolutions {
        for count in [1, 8, 32] {
            let scene = if count == 1 {
                Scene::initial()
            } else {
                let columns = if count == 8 { 4 } else { 8 };
                let rows = count / columns;
                Scene {
                    obstacles: (0..count)
                        .map(|i| {
                            Obstacle::hole(
                                ObstacleId(i as u64 + 1),
                                PeriodicCubicSpline::rounded(
                                    Point2::new(
                                        -0.85 + 1.7 * (i % columns) as f64 / (columns - 1) as f64,
                                        -0.65 + 1.3 * (i / columns) as f64 / (rows - 1) as f64,
                                    ),
                                    if count == 8 { 0.12 } else { 0.07 },
                                ),
                            )
                        })
                        .collect(),
                    ..Scene::default()
                }
            };
            let options = MeshingOptions {
                curve_tolerance: (max_edge * 0.02).min(1.5e-3),
                // Refinement admits 5% slack; compensate so the requested h is a
                // bound on actual maximum edge length, not an approximate target.
                target_edge_length: max_edge / 1.05,
                minimum_angle_degrees: 12.0,
                max_vertices: 50_000,
                max_triangles: 100_000,
                max_refinement_steps: 50_000,
            };
            println!(
                "Starting {count} obstacles, h <= {max_edge:.3}, reference wavelength 0.4 / h = {:.1}",
                0.4 / max_edge
            );
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
                    let mesh = match result {
                        Ok(mesh) => mesh,
                        Err(error) => {
                            failed = true;
                            println!(
                                "FAILED after {:.2} ms: {error}; {:?}",
                                start.elapsed().as_secs_f64() * 1000.0,
                                job.stats()
                            );
                            break;
                        }
                    };
                    println!(
                        "{count} obstacles: {:.2} ms total, {maximum:.3} ms max call, {calls} calls, {} vertices, {} triangles",
                        start.elapsed().as_secs_f64() * 1000.0,
                        mesh.vertices.len(),
                        mesh.triangles.len(),
                    );
                    println!(
                        "  actual max edge {:.6}, min angle {:.2}, wavelength / actual max edge {:.2}",
                        mesh.quality.maximum_edge_length,
                        mesh.quality.minimum_angle_degrees,
                        0.4 / mesh.quality.maximum_edge_length
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
    if failed {
        std::process::ExitCode::FAILURE
    } else {
        std::process::ExitCode::SUCCESS
    }
}
