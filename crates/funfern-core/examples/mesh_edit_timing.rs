//! Native meshing harness. --paced runs 2 ms slices on a 60 Hz schedule.
//! It excludes rendering/GPU work; the app reports its own observed latency.
use funfern_core::*;
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

fn main() {
    let paced = std::env::args().any(|a| a == "--paced");
    for h in [0.04_f64, 0.02] {
        let options = MeshingOptions {
            curve_tolerance: (h * 0.02).min(0.0015),
            target_edge_length: h / 1.05,
            minimum_angle_degrees: 12.0,
            max_vertices: 50_000,
            max_triangles: 100_000,
            max_refinement_steps: 50_000,
        };
        let mut scene = Scene {
            obstacles: (0..8)
                .map(|i| {
                    Obstacle::hole(
                        ObstacleId(i + 1),
                        PeriodicCubicSpline::rounded(
                            Point2::new(-0.66 + (i % 4) as f64 * 0.44, -0.4 + (i / 4) as f64 * 0.8),
                            0.12,
                        ),
                    )
                })
                .collect(),
            materials: vec![
                Material::default_medium(),
                Material {
                    id: MaterialId(2),
                    name: "Inclusion".into(),
                    mass_density: ScalarField::constant(1.0),
                    stiffness: ScalarField::constant(1.8),
                    damping: ScalarField::constant(0.0),
                    parameters: vec![],
                    color: [190, 110, 80],
                },
            ],
            regions: vec![
                Region {
                    id: BACKGROUND_REGION,
                    material: DEFAULT_MATERIAL,
                    frame: MaterialFrame::world(),
                },
                Region {
                    id: RegionId(2),
                    material: MaterialId(2),
                    frame: MaterialFrame::world(),
                },
            ],
            ..Scene::default()
        };
        scene.obstacles[1].role = LoopRole::MaterialInterface {
            exterior: BACKGROUND_REGION,
            interior: RegionId(2),
        };
        scene.internal_boundaries.push(InternalBoundary {
            id: InternalBoundaryId(1),
            spline: OpenCubicSpline::uniform(vec![
                Point2::new(-0.38, 0.0),
                Point2::new(-0.13, 0.015),
                Point2::new(0.13, -0.015),
                Point2::new(0.38, 0.0),
            ])
            .unwrap(),
            region: BACKGROUND_REGION,
            span_laws: vec![InternalBoundaryLaw::REFLECTING],
        });
        let start = Instant::now();
        let mut mesh = Arc::new(mesh_scene(&scene, 0, options).unwrap());
        println!(
            "h={h}: initial continuous full build {:.1} ms, {} triangles",
            start.elapsed().as_secs_f64() * 1000.0,
            mesh.triangles.len()
        );
        for (revision, (loop_index, delta)) in [
            (Some(0), Point2::new(0.005, 0.0)),
            (Some(1), Point2::new(0.0, 0.005)),
            (None, Point2::new(-0.005, -0.005)),
        ]
        .into_iter()
        .enumerate()
        {
            let mut next = scene.clone();
            let target_name = if let Some(loop_index) = loop_index {
                let p = next.obstacles[loop_index].spline.controls()[0];
                next.obstacles[loop_index]
                    .spline
                    .set_control(0, p + delta)
                    .unwrap();
                format!("loop {loop_index}")
            } else {
                let p = next.internal_boundaries[0].spline.controls()[1];
                next.internal_boundaries[0]
                    .spline
                    .set_control(1, p + delta)
                    .unwrap();
                "baffle 0".into()
            };
            let start = Instant::now();
            let mut job = MeshUpdateJob::new(
                Some((mesh.clone(), scene.clone())),
                next.clone(),
                revision as u64 + 1,
                options,
            );
            let mut work = Duration::ZERO;
            let mut max = Duration::ZERO;
            let mut slices = 0;
            loop {
                let slice = Instant::now();
                let mut result = None;
                for _ in 0..100_000 {
                    result = job.advance(1);
                    if result.is_some() || slice.elapsed() >= Duration::from_millis(2) {
                        break;
                    }
                }
                work += slice.elapsed();
                max = max.max(slice.elapsed());
                slices += 1;
                if let Some(result) = result {
                    let result = result.unwrap();
                    println!(
                        "  edit {} {target_name}: ready {:.1} ms, active {:.1} ms, gaps {:.1} ms, max {:.3} ms, {slices} slices; attempts {}, retry causes {:?}, fallback {:?}, patch {}v/{}t, baffles {}/{}, preserved {:.1}%",
                        revision + 1,
                        start.elapsed().as_secs_f64() * 1000.0,
                        work.as_secs_f64() * 1000.0,
                        start.elapsed().saturating_sub(work).as_secs_f64() * 1000.0,
                        max.as_secs_f64() * 1000.0,
                        result.report.repair_attempts,
                        result
                            .report
                            .retry_failures
                            .iter()
                            .map(|failure| failure.kind)
                            .collect::<Vec<_>>(),
                        result
                            .report
                            .fallback_failure
                            .as_ref()
                            .map(|failure| failure.kind),
                        result.report.repair_vertices,
                        result.report.repair_triangles,
                        result.report.repaired_baffles,
                        result.report.paired_trace_segments,
                        100.0 * result.report.preserved_triangles as f64
                            / result.report.original_triangles.max(1) as f64,
                    );
                    mesh = Arc::new(result.mesh);
                    break;
                }
                if paced {
                    std::thread::sleep(
                        Duration::from_secs_f64(1.0 / 60.0).saturating_sub(slice.elapsed()),
                    );
                }
            }
            scene = next;
        }
    }
}
