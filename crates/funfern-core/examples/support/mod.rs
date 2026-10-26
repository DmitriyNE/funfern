//! What the core's examples share: topology scenes, meshed and assembled as
//! the app's runtime prepares them. Not every example uses every helper.
#![allow(dead_code)]

use funfern_core::*;

/// A mesh with the plan it was meshed from, through which the assembly reads
/// the walls and curve laws.
pub struct Meshed {
    pub mesh: TriMesh,
    pub plan: TopologyMeshPlan,
}

impl std::ops::Deref for Meshed {
    type Target = TriMesh;

    fn deref(&self) -> &TriMesh {
        &self.mesh
    }
}

/// `scene` compiled at `revision`, its plan coarsened for `options`, and
/// meshed.
pub fn meshed(
    scene: &TopologyScene,
    revision: u64,
    options: MeshingOptions,
) -> Result<Meshed, String> {
    let compiled = scene
        .compile(revision)
        .map_err(|issue| format!("scene: {issue}"))?;
    let plan = compiled
        .plan
        .coarsened(&compiled.topology, AtomCoarsening::from_meshing(options))
        .map_err(|issue| format!("plan: {issue}"))?;
    let mesh =
        mesh_topology_plan(&plan, revision, options).map_err(|error| format!("mesh: {error}"))?;
    Ok(Meshed { mesh, plan })
}

/// `scene`'s scalar operator on `meshed` with `walls`, whatever walls the
/// scene has.
pub fn assembled(
    meshed: &Meshed,
    scene: &TopologyScene,
    walls: OuterBoundaryConditions,
) -> Result<QuadraticWaveOperator, WaveError> {
    let model = OwnedTopologyWaveModel {
        outer_boundaries: walls,
        ..TopologyWaveModel::from_topology_scene(scene).to_owned()
    };
    QuadraticWaveOperator::assemble_topology(&meshed.mesh, &meshed.plan, model.as_model())
}

/// The spans of a curve over `spline`, numbered from `10 * id`, all with
/// `behavior`.
fn curve(id: u64, spline: CurveSpline, behavior: SpanBehavior) -> TopologyCurve {
    let spans = (0..spline.span_count() as u64)
        .map(|index| CurveSpan {
            id: CurveSpanId(10 * id + index),
            behavior,
        })
        .collect();
    TopologyCurve::new(CurveId(id), spline, spans).unwrap()
}

/// Adds a closed curve over `spline` whose inside is a hole, its rim
/// reflecting.
pub fn add_hole(scene: &mut TopologyScene, id: u64, spline: PeriodicCubicSpline) {
    let curve = curve(id, CurveSpline::Closed(spline), SpanBehavior::REFLECTING);
    let [start, end] = curve.spline.span_bounds(0).unwrap();
    scene.face_assignments.push(AuthoredFaceAssignment {
        anchor: FaceAnchor::Curve {
            curve: curve.id,
            span: curve.spans[0].id,
            side: CurveTraceSide::Left,
            parameter: 0.5 * (start + end),
        },
        region: None,
    });
    scene.geometry.curves.push(curve);
}

/// Adds a free curve over `spline`, every span with `behavior`.
pub fn add_baffle(
    scene: &mut TopologyScene,
    id: u64,
    spline: OpenCubicSpline,
    behavior: SpanBehavior,
) {
    scene
        .geometry
        .curves
        .push(curve(id, CurveSpline::Open(spline), behavior));
}

/// Makes the hole `id` an inclusion of `region`, its rim transmitting.
pub fn fill_hole(scene: &mut TopologyScene, id: u64, region: RegionId) {
    let curve = scene
        .geometry
        .curves
        .iter_mut()
        .find(|curve| curve.id == CurveId(id))
        .unwrap();
    for span in &mut curve.spans {
        span.behavior = SpanBehavior::Transmitting;
    }
    let first = curve.spans[0].id;
    scene
        .face_assignments
        .iter_mut()
        .find(|assignment| {
            matches!(assignment.anchor, FaceAnchor::Curve { span, .. } if span == first)
        })
        .unwrap()
        .region = Some(region);
}

/// The default scene and a reflecting hole of radius 0.15 at the origin.
pub fn initial() -> TopologyScene {
    let mut scene = TopologyScene::default();
    add_hole(
        &mut scene,
        1,
        PeriodicCubicSpline::rounded(Point2::default(), 0.15),
    );
    scene
}
