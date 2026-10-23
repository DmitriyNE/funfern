//! The default scene prepared end to end. CI runs this test in the debug
//! profile as well as the release one: an arithmetic overflow there panics
//! where a release build wraps it silently, and the trace eigensolve's seed
//! did, for every scene with a second-order outgoing wall, the default among
//! them (NUM-8 of the 2026-10-20 review).
use funfern_core::*;

#[test]
fn the_default_scene_prepares_through_its_outgoing_walls() {
    let scene = TopologyScene::default();
    let compiled = scene.compile(1).unwrap();
    let options = MeshingOptions {
        target_edge_length: 0.16,
        ..MeshingOptions::default()
    };
    let plan = compiled
        .plan
        .coarsened(&compiled.topology, AtomCoarsening::from_meshing(options))
        .unwrap();
    let mesh = mesh_topology_plan(&plan, 2, options).unwrap();
    let quadratic = QuadraticWaveOperator::assemble_topology(
        &mesh,
        &plan,
        TopologyWaveModel::from_topology_scene(&scene),
    )
    .unwrap();
    let canonical = CanonicalWaveOperator::compile(
        &mesh,
        &quadratic,
        TopologyWaveModel::from_topology_scene(&scene),
        1,
    )
    .unwrap();
    assert!(canonical.outgoing_boundary().is_some());
}
