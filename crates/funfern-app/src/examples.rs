use funfern_app::{editor::Document, persistence};
use funfern_core::*;
use std::sync::OnceLock;

pub struct ExampleScene {
    pub name: &'static str,
    pub description: &'static str,
    pub document: Document,
}

pub fn catalog() -> &'static [ExampleScene] {
    static CATALOG: OnceLock<Vec<ExampleScene>> = OnceLock::new();
    CATALOG.get_or_init(|| {
        vec![
            ExampleScene {
                name: "Starter obstacle",
                description: "A rounded reflecting obstacle in a second-order absorbing box.",
                document: Document::default(),
            },
            ExampleScene {
                name: "Double slit",
                description: "Three straight baffles form a simple two-aperture screen.",
                document: double_slit(),
            },
            ExampleScene {
                name: "Material lens",
                description: "A slower circular material region bends waves toward its axis.",
                document: material_lens(),
            },
            ExampleScene {
                name: "Obstacle array",
                description: "Eight reflecting obstacles exercise scattering and adaptive meshing.",
                document: persistence::parse_document(include_bytes!(
                    "../../../examples/eight-obstacles.json"
                ))
                .expect("bundled obstacle example must remain valid"),
            },
        ]
    })
}

fn straight(id: u64, y0: f64, y1: f64) -> InternalBoundary {
    let controls = (0..4)
        .map(|index| Point2::new(0.0, y0 + (y1 - y0) * index as f64 / 3.0))
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
        ..Default::default()
    };
    Document {
        draft: scene.clone(),
        accepted: scene,
    }
}

fn material_lens() -> Document {
    let mut scene = Scene::default();
    scene.materials.push(Material {
        id: MaterialId(2),
        name: "Slow lens".into(),
        mass_density: 1.0,
        stiffness: 0.36,
        damping: 0.0,
        color: [61, 116, 139],
    });
    scene.regions.push(Region {
        id: RegionId(2),
        material: MaterialId(2),
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
    }
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
        }
    }
}
