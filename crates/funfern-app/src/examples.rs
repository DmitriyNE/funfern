use funfern_app::{
    editor::{Document, ProbeDefinition, ProbeId, ProbeTarget, SourceSettings},
    persistence,
};
use funfern_core::*;
use std::sync::OnceLock;

pub struct ExampleScene {
    pub name: &'static str,
    pub description: &'static str,
    pub document: Document,
    pub simulation: ExampleSimulation,
}

#[derive(Clone, Copy, Debug)]
pub struct ExampleSimulation {
    pub source: SourceSettings,
}

pub fn catalog() -> &'static [ExampleScene] {
    static CATALOG: OnceLock<Vec<ExampleScene>> = OnceLock::new();
    CATALOG.get_or_init(|| {
        vec![
            example(
                "Starter obstacle",
                "A point source scatters from a rounded obstacle above a reflecting floor.",
                starter_obstacle(),
                continuous_source(Point2::new(-0.55, 0.05), 2.5, 18.0, 0.06),
            ),
            example(
                "Double slit",
                "A point source illuminates two apertures in a reflecting waveguide.",
                double_slit(),
                continuous_source(Point2::new(-0.62, 0.0), 3.0, 20.0, 0.05),
            ),
            example(
                "Material lens",
                "A point source illuminates a slower circular material region with absorbing edges.",
                material_lens(),
                continuous_source(Point2::new(-0.72, 0.0), 3.5, 16.0, 0.045),
            ),
            example(
                "Obstacle array",
                "A point source drives multiple scattering through eight reflecting obstacles.",
                obstacle_array(),
                continuous_source(Point2::new(-0.92, 0.0), 3.0, 20.0, 0.045),
            ),
        ]
    })
}

fn example(
    name: &'static str,
    description: &'static str,
    mut document: Document,
    simulation: ExampleSimulation,
) -> ExampleScene {
    document.source = simulation.source;
    ExampleScene {
        name,
        description,
        document,
        simulation,
    }
}

fn continuous_source(
    position: Point2,
    frequency_hz: f32,
    amplitude: f32,
    width: f32,
) -> ExampleSimulation {
    ExampleSimulation {
        source: SourceSettings {
            enabled: true,
            position,
            amplitude,
            width,
            frequency_hz,
            region: BACKGROUND_REGION,
        },
    }
}

fn reflecting_channel() -> OuterBoundaryConditions {
    let mut boundaries = OuterBoundaryConditions::default();
    boundaries.sides[OuterSide::Bottom.index()] = OuterBoundaryCondition::Reflecting;
    boundaries.sides[OuterSide::Top.index()] = OuterBoundaryCondition::Reflecting;
    boundaries
}

fn starter_obstacle() -> Document {
    let mut document = Document::default();
    document.draft.outer_boundaries = reflecting_channel();
    document.accepted.outer_boundaries = reflecting_channel();
    document.probes.push(ProbeDefinition {
        id: ProbeId(1),
        name: "Receiver".into(),
        color: [63, 144, 239],
        enabled: true,
        target: ProbeTarget::Point(Point2::new(0.5, 0.15)),
    });
    document
}

fn straight(id: u64, y0: f64, y1: f64) -> InternalBoundary {
    let controls = (0..4)
        .map(|index| {
            let y = y0 + (y1 - y0) * index as f64 / 3.0;
            Point2::new(0.0, y)
        })
        .collect();
    InternalBoundary {
        id: InternalBoundaryId(id),
        spline: OpenCubicSpline::uniform(controls).unwrap(),
        region: BACKGROUND_REGION,
        span_laws: vec![InternalBoundaryLaw::REFLECTING],
    }
}

fn double_slit() -> Document {
    let scene = Scene {
        internal_boundaries: vec![
            straight(1, -0.9, -0.36),
            straight(2, -0.14, 0.14),
            straight(3, 0.36, 0.9),
        ],
        outer_boundaries: reflecting_channel(),
        ..Default::default()
    };
    Document {
        draft: scene.clone(),
        accepted: scene,
        probes: vec![],
        source: SourceSettings::default(),
        far_field: Default::default(),
    }
}

fn material_lens() -> Document {
    let mut scene = Scene {
        outer_boundaries: OuterBoundaryConditions::uniform(
            OuterBoundaryCondition::FirstOrderOutgoing,
        ),
        ..Default::default()
    };
    scene.materials.push(Material {
        id: MaterialId(2),
        name: "Slow lens".into(),
        mass_density: ScalarField::constant(1.0),
        stiffness: ScalarField::constant(0.36),
        damping: ScalarField::constant(0.0),
        parameters: vec![],
        color: [61, 116, 139],
    });
    scene.regions.push(Region {
        id: RegionId(2),
        material: MaterialId(2),
        frame: MaterialFrame {
            attachment: MaterialFrameAttachment::FollowRegion,
            ..MaterialFrame::world()
        },
    });
    scene.obstacles.push(Obstacle::with_role(
        ObstacleId(1),
        PeriodicCubicSpline::rounded(Point2::default(), 0.45),
        LoopRole::MaterialInterface {
            exterior: BACKGROUND_REGION,
            interior: RegionId(2),
        },
    ));
    Document {
        draft: scene.clone(),
        accepted: scene,
        probes: vec![],
        source: SourceSettings::default(),
        far_field: Default::default(),
    }
}

fn obstacle_array() -> Document {
    let mut document =
        persistence::parse_document(include_bytes!("../../../examples/eight-obstacles.json"))
            .expect("bundled obstacle example must remain valid");
    let boundaries = OuterBoundaryConditions::uniform(OuterBoundaryCondition::FirstOrderOutgoing);
    document.draft.outer_boundaries = boundaries;
    document.accepted.outer_boundaries = boundaries;
    document
}

pub fn scene_svg(scene: &Scene) -> String {
    let color = |region: RegionId| {
        scene
            .region_material(region)
            .map(|material| {
                format!(
                    "#{:02x}{:02x}{:02x}",
                    material.color[0], material.color[1], material.color[2]
                )
            })
            .unwrap_or_else(|| "#2f4958".into())
    };
    let mut svg = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 1024 1024\"><rect width=\"1024\" height=\"1024\" fill=\"{}\"/>",
        color(BACKGROUND_REGION)
    );
    for obstacle in &scene.obstacles {
        let points = (0..=128)
            .map(|index| {
                obstacle
                    .spline
                    .evaluate(obstacle.spline.period() * index as f64 / 128.0)
            })
            .map(svg_point)
            .collect::<Vec<_>>()
            .join(" ");
        let fill = match obstacle.role {
            LoopRole::Hole { .. } | LoopRole::Wall { .. } => "#111820".into(),
            LoopRole::MaterialInterface { interior, .. } => color(interior),
        };
        svg.push_str(&format!(
            "<polygon points=\"{points}\" fill=\"{fill}\" stroke=\"#8fd8d0\" stroke-width=\"4\"/>"
        ));
    }
    for boundary in &scene.internal_boundaries {
        let points = (0..=96)
            .map(|index| {
                boundary
                    .spline
                    .evaluate(boundary.spline.period() * index as f64 / 96.0)
            })
            .map(svg_point)
            .collect::<Vec<_>>()
            .join(" ");
        svg.push_str(&format!("<polyline points=\"{points}\" fill=\"none\" stroke=\"#8fd8d0\" stroke-width=\"5\" stroke-linecap=\"round\"/>"));
    }
    svg.push_str("<rect x=\"2\" y=\"2\" width=\"1020\" height=\"1020\" fill=\"none\" stroke=\"#d3e5e3\" stroke-width=\"4\"/></svg>");
    svg
}

fn svg_point(point: Point2) -> String {
    format!(
        "{:.2},{:.2}",
        (point.x + 1.0) * 512.0,
        (1.0 - point.y) * 512.0
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_examples_are_structurally_valid_and_exportable() {
        assert!(catalog().len() >= 4);
        assert_eq!(catalog()[0].document.probes.len(), 1);
        for example in catalog() {
            assert!(
                example.document.accepted.structure_valid(),
                "{}",
                example.name
            );
            let svg = scene_svg(&example.document.accepted);
            assert!(svg.starts_with("<svg"));
            assert!(svg.ends_with("</svg>"));
            let mut candidate = persistence::candidate(example.document.clone());
            let loaded = loop {
                if let Some(result) = candidate.advance(100) {
                    break result.unwrap();
                }
            };
            assert_eq!(loaded, example.document, "{}", example.name);
            assert!(example.simulation.source.enabled, "{}", example.name);
            assert_eq!(example.document.source, example.simulation.source);
            assert_ne!(
                example.document.accepted.outer_boundaries,
                OuterBoundaryConditions::default(),
                "{}",
                example.name
            );
        }
    }

    #[test]
    fn double_slit_has_a_practical_explicit_time_step() {
        let document = double_slit();
        let mesh = mesh_scene(
            &document.accepted,
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
        let minimum_edge = mesh
            .triangles
            .iter()
            .flat_map(|triangle| {
                let points = triangle.vertices.map(|vertex| mesh.vertices[vertex].point);
                [
                    (points[1] - points[0]).norm(),
                    (points[2] - points[1]).norm(),
                    (points[0] - points[2]).norm(),
                ]
            })
            .fold(f64::INFINITY, f64::min);
        let operator = QuadraticWaveOperator::assemble_scene_with_boundaries(
            &mesh,
            &document.accepted,
            document.accepted.outer_boundaries,
        )
        .unwrap();
        assert!(
            minimum_edge > 0.02,
            "double-slit mesh contains an unexpectedly short edge: {minimum_edge:e}"
        );
        assert!(
            mesh.quality.minimum_angle_degrees > 5.0,
            "double-slit mesh contains a CFL-limiting sliver: {:.3}°",
            mesh.quality.minimum_angle_degrees
        );
        assert!(
            operator.recommended_time_step() > 1.0e-3,
            "double-slit timestep is impractical: {:e}",
            operator.recommended_time_step()
        );
    }
}
