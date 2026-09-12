use crate::material_overlay::{MaterialOverlay, MaterialProperty};
use funfern_app::{
    editor::{
        Document, ProbeDefinition, ProbeId, ProbeSamplingPreset, ProbeTarget, SourceSettings,
    },
    persistence,
};
use funfern_core::*;
use std::sync::OnceLock;

pub struct ExampleScene {
    pub name: &'static str,
    pub description: &'static str,
    pub document: Document,
    pub simulation: ExampleSimulation,
    pub property_preview: Option<ExamplePropertyPreview>,
}

#[derive(Clone, Debug)]
pub struct ExamplePropertyPreview {
    pub triangles: Vec<ExamplePropertyTriangle>,
    pub minimum: f64,
    pub maximum: f64,
}

#[derive(Clone, Copy, Debug)]
pub struct ExamplePropertyTriangle {
    pub points: [Point2; 3],
    pub values: [f64; 3],
}

#[derive(Clone, Copy, Debug)]
pub struct ExampleSimulation {
    pub source: SourceSettings,
    pub material_overlay: MaterialOverlay,
    pub material_overlay_opacity: f32,
    pub show_amr_target: bool,
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
                "GRIN rod",
                "An off-axis source is guided by a smooth transverse index profile.",
                grin_rod(),
                grin_rod_simulation(),
            ),
            example(
                "Luneburg lens",
                "A plane-like boundary wave focuses at the far rim of a radial index lens.",
                luneburg_lens(),
                luneburg_simulation(),
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
    let property_preview = match simulation.material_overlay {
        MaterialOverlay::Property(property) => property_preview(&document.accepted, property),
        MaterialOverlay::Off | MaterialOverlay::Regions => None,
    };
    ExampleScene {
        name,
        description,
        document,
        simulation,
        property_preview,
    }
}

fn property_preview(scene: &Scene, property: MaterialProperty) -> Option<ExamplePropertyPreview> {
    let mut minimum = f64::INFINITY;
    let mut maximum = f64::NEG_INFINITY;
    let mut triangles = Vec::new();
    for obstacle in &scene.obstacles {
        let LoopRole::MaterialInterface { interior, .. } = obstacle.role else {
            continue;
        };
        let material = scene.region_material(interior)?;
        if !material.varying() {
            continue;
        }
        let polygon = (0..128)
            .map(|index| {
                obstacle
                    .spline
                    .evaluate(obstacle.spline.period() * index as f64 / 128.0)
            })
            .collect::<Vec<_>>();
        let (mut low, mut high) = (
            Point2::new(f64::INFINITY, f64::INFINITY),
            Point2::new(f64::NEG_INFINITY, f64::NEG_INFINITY),
        );
        for point in &polygon {
            low.x = low.x.min(point.x);
            low.y = low.y.min(point.y);
            high.x = high.x.max(point.x);
            high.y = high.y.max(point.y);
        }
        const CELLS: usize = 32;
        for y in 0..CELLS {
            for x in 0..CELLS {
                let corners = [
                    Point2::new(
                        low.x + (high.x - low.x) * x as f64 / CELLS as f64,
                        low.y + (high.y - low.y) * y as f64 / CELLS as f64,
                    ),
                    Point2::new(
                        low.x + (high.x - low.x) * (x + 1) as f64 / CELLS as f64,
                        low.y + (high.y - low.y) * y as f64 / CELLS as f64,
                    ),
                    Point2::new(
                        low.x + (high.x - low.x) * (x + 1) as f64 / CELLS as f64,
                        low.y + (high.y - low.y) * (y + 1) as f64 / CELLS as f64,
                    ),
                    Point2::new(
                        low.x + (high.x - low.x) * x as f64 / CELLS as f64,
                        low.y + (high.y - low.y) * (y + 1) as f64 / CELLS as f64,
                    ),
                ];
                for points in [
                    [corners[0], corners[1], corners[2]],
                    [corners[0], corners[2], corners[3]],
                ] {
                    let center = (points[0] + points[1] + points[2]) / 3.0;
                    if !point_in_polygon(center, &polygon) {
                        continue;
                    }
                    let mut values = [0.0; 3];
                    for (value, point) in values.iter_mut().zip(points) {
                        let coefficients = scene.material_at(interior, point).ok()?;
                        *value = match property {
                            MaterialProperty::Density => coefficients.mass_density,
                            MaterialProperty::Stiffness => coefficients.stiffness,
                            MaterialProperty::Damping => coefficients.damping,
                            MaterialProperty::WaveSpeed => {
                                (coefficients.stiffness / coefficients.mass_density).sqrt()
                            }
                            MaterialProperty::Impedance => {
                                (coefficients.stiffness * coefficients.mass_density).sqrt()
                            }
                        };
                        minimum = minimum.min(*value);
                        maximum = maximum.max(*value);
                    }
                    triangles.push(ExamplePropertyTriangle { points, values });
                }
            }
        }
    }
    (!triangles.is_empty() && minimum.is_finite() && maximum.is_finite()).then_some(
        ExamplePropertyPreview {
            triangles,
            minimum,
            maximum,
        },
    )
}

fn point_in_polygon(point: Point2, polygon: &[Point2]) -> bool {
    polygon
        .iter()
        .zip(polygon.iter().cycle().skip(1))
        .fold(false, |inside, (a, b)| {
            let crosses = (a.y > point.y) != (b.y > point.y)
                && point.x < (b.x - a.x) * (point.y - a.y) / (b.y - a.y) + a.x;
            inside ^ crosses
        })
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
        material_overlay: MaterialOverlay::Off,
        material_overlay_opacity: 0.55,
        show_amr_target: false,
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

fn grin_rod_simulation() -> ExampleSimulation {
    let mut simulation = continuous_source(Point2::new(-0.56, 0.11), 4.0, 16.0, 0.04);
    simulation.source.region = RegionId(2);
    simulation.material_overlay = MaterialOverlay::Property(MaterialProperty::WaveSpeed);
    simulation
}

fn grin_rod() -> Document {
    let region = RegionId(2);
    let material = MaterialId(2);
    let mut scene = Scene::default();
    scene.materials.push(Material {
        id: material,
        name: "GRIN profile".into(),
        mass_density: ScalarField::formula("1 + dn * smoothstep(0, 1, 1 - (y / H)^2)").unwrap(),
        stiffness: ScalarField::formula("1 / (1 + dn * smoothstep(0, 1, 1 - (y / H)^2))").unwrap(),
        damping: ScalarField::constant(0.0),
        parameters: vec![
            MaterialParameter {
                name: "H".into(),
                value: 0.30,
            },
            MaterialParameter {
                name: "dn".into(),
                value: 0.60,
            },
        ],
        color: [46, 120, 139],
    });
    scene.regions.push(Region {
        id: region,
        material,
        frame: MaterialFrame {
            attachment: MaterialFrameAttachment::FollowRegion,
            ..MaterialFrame::world()
        },
    });
    scene.obstacles.push(Obstacle::with_role(
        ObstacleId(1),
        PeriodicCubicSpline::uniform(vec![
            Point2::new(-0.70, -0.24),
            Point2::new(-0.42, -0.32),
            Point2::new(0.42, -0.32),
            Point2::new(0.70, -0.24),
            Point2::new(0.74, -0.08),
            Point2::new(0.74, 0.08),
            Point2::new(0.70, 0.24),
            Point2::new(0.42, 0.32),
            Point2::new(-0.42, 0.32),
            Point2::new(-0.70, 0.24),
            Point2::new(-0.74, 0.08),
            Point2::new(-0.74, -0.08),
        ])
        .unwrap(),
        LoopRole::MaterialInterface {
            exterior: BACKGROUND_REGION,
            interior: region,
        },
    ));
    Document {
        draft: scene.clone(),
        accepted: scene,
        probes: vec![ProbeDefinition {
            id: ProbeId(1),
            name: "Rod output profile".into(),
            color: [94, 220, 195],
            enabled: true,
            target: ProbeTarget::Segment {
                start: Point2::new(0.50, -0.38),
                end: Point2::new(0.50, 0.38),
                preset: ProbeSamplingPreset::High,
            },
        }],
        source: SourceSettings::default(),
        far_field: Default::default(),
    }
}

fn luneburg_simulation() -> ExampleSimulation {
    ExampleSimulation {
        source: SourceSettings::default(),
        material_overlay: MaterialOverlay::Property(MaterialProperty::WaveSpeed),
        material_overlay_opacity: 0.55,
        show_amr_target: false,
    }
}

fn luneburg_lens() -> Document {
    const RADIUS: f64 = 0.43;
    const CONTROL_COUNT: usize = 16;
    let region = RegionId(2);
    let material = MaterialId(2);
    let center = Point2::new(0.08, 0.0);
    let mut scene = Scene::default();
    scene.outer_boundaries.sides[OuterSide::Left.index()] = OuterBoundaryCondition::Dirichlet {
        signal: BoundarySignal {
            offset: 0.0,
            amplitude: 0.7,
            frequency_hz: 3.5,
            phase_radians: 0.0,
        },
    };
    scene.materials.push(Material {
        id: material,
        name: "Luneburg profile".into(),
        mass_density: ScalarField::formula("sqrt(max(2 - (r / R)^2, 1))").unwrap(),
        stiffness: ScalarField::formula("1 / sqrt(max(2 - (r / R)^2, 1))").unwrap(),
        damping: ScalarField::constant(0.0),
        parameters: vec![MaterialParameter {
            name: "R".into(),
            value: RADIUS,
        }],
        color: [66, 105, 151],
    });
    scene.regions.push(Region {
        id: region,
        material,
        frame: MaterialFrame {
            origin: center,
            attachment: MaterialFrameAttachment::FollowRegion,
            ..MaterialFrame::world()
        },
    });
    let step = std::f64::consts::TAU / CONTROL_COUNT as f64;
    let control_radius = RADIUS * 3.0 / (2.0 + step.cos());
    let controls = (0..CONTROL_COUNT)
        .map(|index| {
            let angle = step * index as f64;
            center + Point2::new(angle.cos(), angle.sin()) * control_radius
        })
        .collect();
    scene.obstacles.push(Obstacle::with_role(
        ObstacleId(1),
        PeriodicCubicSpline::uniform(controls).unwrap(),
        LoopRole::MaterialInterface {
            exterior: BACKGROUND_REGION,
            interior: region,
        },
    ));
    Document {
        draft: scene.clone(),
        accepted: scene,
        probes: vec![
            ProbeDefinition {
                id: ProbeId(1),
                name: "Focal profile".into(),
                color: [94, 220, 195],
                enabled: true,
                target: ProbeTarget::Segment {
                    start: Point2::new(center.x + RADIUS - 0.025, -0.28),
                    end: Point2::new(center.x + RADIUS - 0.025, 0.28),
                    preset: ProbeSamplingPreset::High,
                },
            },
            ProbeDefinition {
                id: ProbeId(2),
                name: "Focus energy".into(),
                color: [248, 196, 112],
                enabled: true,
                target: ProbeTarget::AreaDisk {
                    center: Point2::new(center.x + RADIUS - 0.025, 0.0),
                    radius: 0.065,
                },
            },
        ],
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
    use std::sync::Arc;

    #[test]
    fn bundled_examples_are_structurally_valid_and_exportable() {
        assert_eq!(catalog().len(), 6);
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
            assert_eq!(example.document.source, example.simulation.source);
            let driven_boundary = OuterSide::ALL.into_iter().any(|side| {
                example
                    .document
                    .accepted
                    .outer_boundaries
                    .get(side)
                    .signal()
                    .is_some_and(|signal| signal.amplitude != 0.0)
            });
            assert!(
                example.simulation.source.enabled || driven_boundary,
                "{} has no active driver",
                example.name
            );
            if matches!(
                example.simulation.material_overlay,
                MaterialOverlay::Property(_)
            ) {
                let preview = example
                    .property_preview
                    .as_ref()
                    .expect("property example needs a thumbnail preview");
                assert!(!preview.triangles.is_empty());
                assert!(preview.minimum.is_finite());
                assert!(preview.maximum > preview.minimum);
            }
        }
    }

    #[test]
    fn spatial_examples_are_impedance_matched_and_amr_resolves_the_driver() {
        for (name, center, edge, expected_center_speed, frequency) in [
            (
                "GRIN rod",
                Point2::new(0.0, 0.0),
                Point2::new(0.0, 0.30),
                0.625,
                4.0,
            ),
            (
                "Luneburg lens",
                Point2::new(0.08, 0.0),
                Point2::new(0.51, 0.0),
                1.0 / std::f64::consts::SQRT_2,
                3.5,
            ),
        ] {
            let example = catalog()
                .iter()
                .find(|example| example.name == name)
                .unwrap();
            let scene = &example.document.accepted;
            assert!(scene.has_varying_materials());
            assert!(matches!(
                example.simulation.material_overlay,
                MaterialOverlay::Property(MaterialProperty::WaveSpeed)
            ));
            assert!(example.document.probes.iter().all(ProbeDefinition::valid));
            let region = RegionId(2);
            for point in [center, edge] {
                let coefficients = scene.material_at(region, point).unwrap();
                assert!((coefficients.mass_density * coefficients.stiffness - 1.0).abs() < 1e-12);
            }
            let center_coefficients = scene.material_at(region, center).unwrap();
            let center_speed =
                (center_coefficients.stiffness / center_coefficients.mass_density).sqrt();
            assert!((center_speed - expected_center_speed).abs() < 1e-12);
            let edge_coefficients = scene.material_at(region, edge).unwrap();
            let edge_speed = (edge_coefficients.stiffness / edge_coefficients.mass_density).sqrt();
            assert!((edge_speed - 1.0).abs() < 1e-12);

            let mesh = Arc::new(
                mesh_scene(
                    scene,
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
                .unwrap(),
            );
            let operator = Arc::new(
                QuadraticWaveOperator::assemble_scene_with_boundaries(
                    &mesh,
                    scene,
                    scene.outer_boundaries,
                )
                .unwrap(),
            );
            assert!(
                operator.recommended_time_step() > 5.0e-4,
                "{name} timestep is impractical: {:e}",
                operator.recommended_time_step()
            );
            let time_step = operator.recommended_time_step();
            let count = operator.degrees_of_freedom();
            let snapshot = QuadraticSolutionSnapshot {
                mesh_revision: mesh.mesh_revision,
                displacement: vec![0.0; count],
                velocity: vec![0.0; count],
                acceleration: vec![0.0; count],
                auxiliary: vec![0.0; count],
                volume_acceleration: vec![0.0; count],
                time: 0.0,
                time_step,
            };
            let mut job = SolutionIndicatorJob::new(
                mesh,
                operator,
                scene.clone(),
                snapshot,
                SolutionIndicatorOptions {
                    forcing_frequency_hz: frequency,
                    ..Default::default()
                },
            );
            let result = loop {
                if let Some(result) = job.advance(100_000) {
                    break result.unwrap();
                }
            };
            assert!(result.report.minimum_target >= 0.02);
            assert!(result.report.maximum_target <= 0.16);
            assert!(
                result.report.minimum_target < 0.05,
                "{name} driver wavelength did not tighten the AMR target"
            );
            eprintln!(
                "{name}: triangles={}, dt={:.4e}, AMR target={:.4}..{:.4}",
                result.element_targets.len(),
                time_step,
                result.report.minimum_target,
                result.report.maximum_target
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
