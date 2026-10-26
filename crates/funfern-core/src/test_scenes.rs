//! Topology scenes the core's tests share, and their preparation as the app's
//! runtime does it: compiled, the plan coarsened to the meshing tolerance,
//! meshed and assembled. They replace the fixtures of the retired `Scene`
//! model, whose holes, baffles and inclusions are curves here.

use crate::*;

/// Walls that reflect on every side, which the `Scene` fixtures forced on
/// the scalar assembly whatever the scene's own walls were.
pub(crate) const REFLECTING_WALLS: OuterBoundaryConditions =
    OuterBoundaryConditions::uniform(OuterBoundaryCondition::Reflecting);

/// What `Scene::initial()` was: the default scene and a hole of radius 0.15
/// at the origin, every span of its rim reflecting.
pub(crate) fn initial() -> TopologyScene {
    let mut scene = TopologyScene::default();
    add_hole(&mut scene, CurveId(1), Point2::default(), 0.15);
    scene
}

/// The default scene's physics, medium and walls over `regions`: the
/// material library of a plan built by hand.
pub(crate) fn library(regions: Vec<Region>) -> OwnedTopologyWaveModel {
    OwnedTopologyWaveModel {
        regions,
        ..TopologyWaveModel::from_topology_scene(&TopologyScene::default()).to_owned()
    }
}

/// The spans of a curve over `spline`, numbered from `100 * curve`, all with
/// one behaviour.
fn spans(curve: CurveId, spline: &CurveSpline, behavior: SpanBehavior) -> Vec<CurveSpan> {
    (0..spline.span_count() as u64)
        .map(|index| CurveSpan {
            id: CurveSpanId(100 * curve.0 + index),
            behavior,
        })
        .collect()
}

/// The anchor of the face on `side` of a closed curve's first span.
fn inside(curve: &TopologyCurve, side: CurveTraceSide) -> FaceAnchor {
    let [start, end] = curve.spline.span_bounds(0).unwrap();
    FaceAnchor::Curve {
        curve: curve.id,
        span: curve.spans[0].id,
        side,
        parameter: 0.5 * (start + end),
    }
}

/// Adds a rounded closed curve whose inside is a hole, its rim reflecting.
pub(crate) fn add_hole(scene: &mut TopologyScene, id: CurveId, center: Point2, radius: f64) {
    let spline = CurveSpline::Closed(PeriodicCubicSpline::rounded(center, radius));
    let curve = TopologyCurve::new(
        id,
        spline.clone(),
        spans(id, &spline, SpanBehavior::REFLECTING),
    )
    .unwrap();
    scene.face_assignments.push(AuthoredFaceAssignment {
        anchor: inside(&curve, CurveTraceSide::Left),
        region: None,
    });
    scene.geometry.curves.push(curve);
}

/// Adds a free open curve through `points`, every span with `behavior`: a
/// baffle when separated, the legacy internal boundary's place.
pub(crate) fn add_baffle(
    scene: &mut TopologyScene,
    id: CurveId,
    points: Vec<Point2>,
    behavior: SpanBehavior,
) {
    let spline = CurveSpline::Open(OpenCubicSpline::uniform(points).unwrap());
    let curve = TopologyCurve::new(id, spline.clone(), spans(id, &spline, behavior)).unwrap();
    scene.geometry.curves.push(curve);
}

/// A scene meshed and assembled: the plan the mesh came from, the scalar
/// operator and the canonical one, both of the scene with its temporal laws
/// stripped, as a driven generation's base is.
pub(crate) struct Prepared {
    pub(crate) scene: TopologyScene,
    pub(crate) plan: TopologyMeshPlan,
    pub(crate) mesh: TriMesh,
    pub(crate) quadratic: QuadraticWaveOperator,
    pub(crate) canonical: CanonicalWaveOperator,
}

impl Prepared {
    /// The scene's model, its laws kept.
    pub(crate) fn model(&self) -> TopologyWaveModel<'_> {
        TopologyWaveModel::from_topology_scene(&self.scene)
    }
}

/// `scene` meshed with `options` and its plan, as the runtime meshes one.
pub(crate) fn mesh(scene: &TopologyScene, options: MeshingOptions) -> (TopologyMeshPlan, TriMesh) {
    let compiled = scene.compile(1).unwrap();
    let plan = compiled
        .plan
        .coarsened(&compiled.topology, AtomCoarsening::from_meshing(options))
        .unwrap();
    let mesh = mesh_topology_plan(&plan, 1, options).unwrap();
    (plan, mesh)
}

/// The meshing options at `edge`, the rest the defaults.
pub(crate) fn at_edge(edge: f64) -> MeshingOptions {
    MeshingOptions {
        target_edge_length: edge,
        ..MeshingOptions::default()
    }
}

/// `scene` meshed with `options` and assembled through its own walls.
pub(crate) fn prepare_with(scene: TopologyScene, options: MeshingOptions) -> Prepared {
    let (plan, mesh) = self::mesh(&scene, options);
    let base = TopologyWaveModel::from_topology_scene(&scene)
        .to_owned()
        .without_temporal_laws();
    let quadratic =
        QuadraticWaveOperator::assemble_topology(&mesh, &plan, base.as_model()).unwrap();
    let canonical = CanonicalWaveOperator::compile(&mesh, &quadratic, base.as_model(), 1).unwrap();
    Prepared {
        scene,
        plan,
        mesh,
        quadratic,
        canonical,
    }
}

/// `scene` meshed at `edge` and assembled through its own walls.
pub(crate) fn prepare(scene: TopologyScene, edge: f64) -> Prepared {
    prepare_with(scene, at_edge(edge))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The initial scene's hole is its rim's inside: no triangle there, and
    /// every other one the background's.
    #[test]
    fn the_initial_scenes_hole_is_inside_its_rim() {
        let (_, mesh) = mesh(&initial(), at_edge(0.3));
        let area = mesh
            .triangles
            .iter()
            .map(|triangle| {
                let [a, b, c] = triangle.vertices.map(|vertex| mesh.vertices[vertex].point);
                0.5 * (b - a).cross(c - a).abs()
            })
            .sum::<f64>();
        assert!(
            mesh.triangles
                .iter()
                .all(|triangle| triangle.region == BACKGROUND_REGION)
        );
        // The rounded rim encloses less than its circle.
        assert!(
            area > 4.0 - std::f64::consts::PI * 0.15 * 0.15 && area < 4.0,
            "{area}"
        );
    }
}
