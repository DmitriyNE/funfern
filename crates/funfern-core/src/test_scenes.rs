//! Topology scenes the core's tests share, and their preparation as the app's
//! runtime does it: compiled, the plan coarsened to the meshing tolerance,
//! meshed and assembled. They replace the fixtures of the retired `Scene`
//! model, whose holes, baffles and inclusions are curves here. The app's
//! tests reach them through the `test-support` feature.

use crate::*;

/// Walls that reflect on every side, which the `Scene` fixtures forced on
/// the scalar assembly whatever the scene's own walls were.
pub const REFLECTING_WALLS: OuterBoundaryConditions =
    OuterBoundaryConditions::uniform(OuterBoundaryCondition::Reflecting);

/// What `Scene::initial()` was: the default scene and a hole of radius 0.15
/// at the origin, every span of its rim reflecting.
pub fn initial() -> TopologyScene {
    let mut scene = TopologyScene::default();
    add_hole(&mut scene, CurveId(1), Point2::default(), 0.15);
    scene
}

/// The default scene's physics, medium and walls over `regions`: the
/// material library of a plan built by hand.
pub fn library(regions: Vec<Region>) -> OwnedTopologyWaveModel {
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
pub fn add_hole(scene: &mut TopologyScene, id: CurveId, center: Point2, radius: f64) {
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

/// Adds a rounded closed curve whose inside is `region`, its rim
/// transmitting: the legacy material interface's place. The region is the
/// caller's to add.
pub fn add_inclusion(
    scene: &mut TopologyScene,
    id: CurveId,
    center: Point2,
    radius: f64,
    region: RegionId,
) {
    let spline = CurveSpline::Closed(PeriodicCubicSpline::rounded(center, radius));
    let curve = TopologyCurve::new(
        id,
        spline.clone(),
        spans(id, &spline, SpanBehavior::Transmitting),
    )
    .unwrap();
    scene.face_assignments.push(AuthoredFaceAssignment {
        anchor: inside(&curve, CurveTraceSide::Left),
        region: Some(region),
    });
    scene.geometry.curves.push(curve);
}

/// Makes the hole `id` an inclusion of `region`, its rim transmitting, as
/// the legacy scenes turned their hole into a material interface.
pub fn fill_hole(scene: &mut TopologyScene, id: CurveId, region: RegionId) {
    let curve = scene
        .geometry
        .curves
        .iter_mut()
        .find(|curve| curve.id == id)
        .unwrap();
    for span in &mut curve.spans {
        span.behavior = SpanBehavior::Transmitting;
    }
    let anchor = inside(curve, CurveTraceSide::Left);
    scene
        .face_assignments
        .iter_mut()
        .find(|assignment| assignment.anchor == anchor)
        .unwrap()
        .region = Some(region);
}

/// A thin gap of `stiffness_ratio` between two reflecting faces.
pub fn thin_gap(stiffness_ratio: f64) -> SpanBehavior {
    SpanBehavior::Separated {
        left: FaceBoundaryCondition::Reflecting,
        right: FaceBoundaryCondition::Reflecting,
        coupling: InternalBoundaryCoupling::ThinGap { stiffness_ratio },
    }
}

/// Adds a free open curve through `points`, every span with `behavior`: a
/// baffle when separated, the legacy internal boundary's place.
pub fn add_baffle(
    scene: &mut TopologyScene,
    id: CurveId,
    points: Vec<Point2>,
    behavior: SpanBehavior,
) {
    let spline = OpenCubicSpline::uniform(points).unwrap();
    let behaviors = vec![behavior; spline.intervals().len()];
    add_open_curve(scene, id, spline, behaviors);
}

/// Adds a free open curve over `spline`, its spans with `behaviors` in order.
pub fn add_open_curve(
    scene: &mut TopologyScene,
    id: CurveId,
    spline: OpenCubicSpline,
    behaviors: Vec<SpanBehavior>,
) {
    let spans = behaviors
        .into_iter()
        .zip(0..)
        .map(|(behavior, index)| CurveSpan {
            id: CurveSpanId(100 * id.0 + index),
            behavior,
        })
        .collect();
    let curve = TopologyCurve::new(id, CurveSpline::Open(spline), spans).unwrap();
    scene.geometry.curves.push(curve);
}

/// A mesh with the plan it was meshed from, through which the assembly
/// reads the walls and curve laws. It reads as the mesh it holds.
#[derive(Clone)]
pub struct Meshed {
    pub mesh: TriMesh,
    pub plan: TopologyMeshPlan,
    /// The revision and options the plan was compiled and coarsened at,
    /// when it came from a scene: how a law edit's plan is made for the
    /// same mesh. A mesh built by hand has none.
    pub source: Option<(u64, MeshingOptions)>,
}

impl Meshed {
    /// A mesh built by hand, with the plan it stands for.
    pub fn by_hand(mesh: TriMesh, plan: TopologyMeshPlan) -> Self {
        Self {
            mesh,
            plan,
            source: None,
        }
    }

    /// The same mesh under `scene`'s plan, as the runtime keeps a mesh
    /// across a law-only edit; an edit that is not one is refused.
    pub fn under(&self, scene: &TopologyScene) -> Self {
        let (revision, options) = self
            .source
            .expect("a mesh built by hand has no scene to plan again");
        let plan = plan_of(scene, revision, options);
        assert_eq!(
            topology_mesh_update_action(&self.plan, &plan),
            TopologyMeshUpdateAction::Reuse,
            "not a law-only edit"
        );
        Self {
            mesh: self.mesh.clone(),
            plan,
            source: self.source,
        }
    }
}

impl std::ops::Deref for Meshed {
    type Target = TriMesh;

    fn deref(&self) -> &TriMesh {
        &self.mesh
    }
}

/// The plan of `domain` with its one face the background's: what a mesh
/// built by hand over that rectangle stands for.
pub fn box_plan(domain: DomainRect) -> TopologyMeshPlan {
    let geometry = TopologyGeometry {
        domain,
        ..TopologyGeometry::default()
    };
    let topology = compile_topology(&geometry, 1).unwrap();
    TopologyMeshPlan::new(
        &topology,
        &[FaceRegionAssignment {
            face: topology.faces[0].id,
            region: Some(BACKGROUND_REGION),
        }],
    )
    .unwrap()
}

/// `scene` compiled at `revision` and meshed with `options` as the runtime
/// meshes it.
pub fn meshed(
    scene: &TopologyScene,
    revision: u64,
    options: MeshingOptions,
) -> Result<Meshed, MeshError> {
    let plan = plan_of(scene, revision, options);
    let mesh = mesh_topology_plan(&plan, revision, options)?;
    Ok(Meshed {
        mesh,
        plan,
        source: Some((revision, options)),
    })
}

/// `scene`'s plan compiled at `revision` and coarsened for `options`: the
/// plan a mesh made with those options is assembled through, and a law-only
/// edit's new one over the same mesh.
pub fn plan_of(scene: &TopologyScene, revision: u64, options: MeshingOptions) -> TopologyMeshPlan {
    let compiled = scene.compile(revision).unwrap();
    compiled
        .plan
        .coarsened(&compiled.topology, AtomCoarsening::from_meshing(options))
        .unwrap()
}

/// Gives `material` a constant loss of `rate` on `physics`'s primary field,
/// what the legacy `damping` was: the magnetic channel in Mechanical and TE,
/// the electric one in TM.
pub fn set_primary_loss(material: &mut Material, physics: PhysicsModel, rate: ScalarField) {
    let channel = Some(LossChannel {
        base_rate: rate,
        law: DampingLaw::constant(),
    });
    if primary_loss_is_electric(physics) {
        material.electric_loss = channel;
    } else {
        material.magnetic_loss = channel;
    }
}

/// A hole's span with `condition` on its outside, the right side, as the
/// legacy hole's span conditions were.
pub fn rim(condition: FaceBoundaryCondition) -> SpanBehavior {
    SpanBehavior::Separated {
        left: FaceBoundaryCondition::Reflecting,
        right: condition,
        coupling: InternalBoundaryCoupling::Independent,
    }
}

/// Gives every span of the curve `id` `behavior`.
pub fn set_spans(scene: &mut TopologyScene, id: CurveId, behavior: SpanBehavior) {
    let curve = scene
        .geometry
        .curves
        .iter_mut()
        .find(|curve| curve.id == id)
        .unwrap();
    for span in &mut curve.spans {
        span.behavior = behavior;
    }
}

/// Gives every span of the hole `id` `condition` on its outside.
pub fn set_rim(scene: &mut TopologyScene, id: CurveId, condition: FaceBoundaryCondition) {
    set_spans(scene, id, rim(condition));
}

/// `scene`'s scalar operator on `meshed` with `walls`, whatever walls the
/// scene has: what the `Scene` path's assembly took. The curve laws are the
/// plan's, so a scene whose laws are not is refused: a law edit is
/// assembled on [`Meshed::under`] it.
pub fn assembled_with(
    meshed: &Meshed,
    scene: &TopologyScene,
    walls: OuterBoundaryConditions,
) -> Result<QuadraticWaveOperator, WaveError> {
    if let Some((revision, options)) = meshed.source {
        assert!(
            plan_of(scene, revision, options) == meshed.plan,
            "the scene's laws are not the mesh's plan's"
        );
    }
    let model = OwnedTopologyWaveModel {
        outer_boundaries: walls,
        ..TopologyWaveModel::from_topology_scene(scene).to_owned()
    };
    QuadraticWaveOperator::assemble_topology(&meshed.mesh, &meshed.plan, model.as_model())
}

/// [`assembled_with`] one condition on every wall.
pub fn assembled(
    meshed: &Meshed,
    scene: &TopologyScene,
    condition: OuterBoundaryCondition,
) -> Result<QuadraticWaveOperator, WaveError> {
    assembled_with(meshed, scene, OuterBoundaryConditions::uniform(condition))
}

/// `scene`'s scalar operator on a mesh built by hand, which has no plan,
/// with `condition` on every wall: an outer edge takes its triangle's region.
pub fn hand_assembled(
    mesh: &TriMesh,
    scene: &TopologyScene,
    condition: OuterBoundaryCondition,
) -> Result<QuadraticWaveOperator, WaveError> {
    hand_assembled_with(mesh, scene, OuterBoundaryConditions::uniform(condition))
}

/// [`hand_assembled`] with a condition per wall.
pub fn hand_assembled_with(
    mesh: &TriMesh,
    scene: &TopologyScene,
    walls: OuterBoundaryConditions,
) -> Result<QuadraticWaveOperator, WaveError> {
    let model = OwnedTopologyWaveModel {
        outer_boundaries: walls,
        ..TopologyWaveModel::from_topology_scene(scene).to_owned()
    };
    QuadraticWaveOperator::assemble_model(mesh, model.as_model())
}

/// A scene meshed and assembled: the plan the mesh came from, the scalar
/// operator and the canonical one, both of the scene with its temporal laws
/// stripped, as a driven generation's base is.
pub struct Prepared {
    pub scene: TopologyScene,
    pub plan: TopologyMeshPlan,
    pub mesh: TriMesh,
    pub quadratic: QuadraticWaveOperator,
    pub canonical: CanonicalWaveOperator,
}

impl Prepared {
    /// The scene's model, its laws kept.
    pub fn model(&self) -> TopologyWaveModel<'_> {
        TopologyWaveModel::from_topology_scene(&self.scene)
    }
}

/// `scene` meshed with `options` and its plan, as the runtime meshes one.
pub fn mesh(scene: &TopologyScene, options: MeshingOptions) -> (TopologyMeshPlan, TriMesh) {
    let compiled = scene.compile(1).unwrap();
    let plan = compiled
        .plan
        .coarsened(&compiled.topology, AtomCoarsening::from_meshing(options))
        .unwrap();
    let mesh = mesh_topology_plan(&plan, 1, options).unwrap();
    (plan, mesh)
}

/// The meshing options at `edge`, the rest the defaults.
pub fn at_edge(edge: f64) -> MeshingOptions {
    MeshingOptions {
        target_edge_length: edge,
        ..MeshingOptions::default()
    }
}

/// `scene` meshed with `options` and assembled through its own walls.
pub fn prepare_with(scene: TopologyScene, options: MeshingOptions) -> Prepared {
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
pub fn prepare(scene: TopologyScene, edge: f64) -> Prepared {
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
