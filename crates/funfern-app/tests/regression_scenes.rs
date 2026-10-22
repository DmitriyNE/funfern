//! Every scene a user had open when something broke, every frozen version 22
//! scene and every gallery scene, prepared the way the app prepares a
//! document: opened, validated to acceptance, meshed, assembled and compiled
//! through `TopologyRuntime::request` at the document's own mesh edge. Each
//! must prepare, each prepared outgoing basis must read back against its
//! operator, and a regression scene prepared twice from a fresh runtime
//! must give the same mesh and basis bit for bit. `tests/frozen_scenes.rs`
//! checks that a file reads back what it held; this checks what the solver
//! does with it. The fixtures are under `tests/fixtures/scenes-regressions`,
//! one per reported bug that came with a scene, added in the fix's commit.
//!
//! A fixed generation's device plan is packed both ways and the two must
//! agree word for word: production packs through `compile_with_quadratic`,
//! which reads the assembled stiffness rows, adds the thin-gap corrections
//! and drops exact zeros; the packer's own tests pack through `compile`,
//! which rebuilds the rows from the samples. A double slit with a thin gap
//! on its baffle joins the gallery here for that comparison, since no
//! gallery scene has one.

use funfern_app::canonical_gpu::{CanonicalGpuClock, CanonicalGpuPlan};
use funfern_app::topology_editor::{TopologyAcceptance, TopologyDocument, TopologyEditor};
use funfern_app::topology_examples;
use funfern_app::topology_persistence;
use funfern_app::topology_runtime::{PreparedTopology, TopologyRuntime};
use funfern_core::{
    CanonicalOutgoingBoundary, CanonicalWaveState, InternalBoundaryCoupling, MeshingOptions,
    SpanBehavior, TriMesh,
};
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

fn fixtures(folder: &str) -> Vec<(String, Vec<u8>)> {
    let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(folder);
    let mut scenes: Vec<_> = std::fs::read_dir(&directory)
        .unwrap_or_else(|error| panic!("{}: {error}", directory.display()))
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .map(|path| {
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            (name, std::fs::read(path).unwrap())
        })
        .collect();
    scenes.sort();
    scenes
}

struct Scene {
    name: String,
    document: TopologyDocument,
    /// A reported bug's scene, prepared twice.
    regression: bool,
}

fn scenes() -> Vec<Scene> {
    let mut scenes = Vec::new();
    for (folder, regression) in [("scenes-regressions", true), ("scenes-v22", false)] {
        for (name, bytes) in fixtures(folder) {
            let document = topology_persistence::parse_document(&bytes)
                .unwrap_or_else(|error| panic!("{folder}/{name} does not open: {error}"));
            scenes.push(Scene {
                name: format!("{folder}/{name}"),
                document,
                regression,
            });
        }
    }
    for example in topology_examples::catalog() {
        scenes.push(Scene {
            name: format!("gallery/{}", example.name),
            document: example.document.clone(),
            regression: false,
        });
    }
    scenes.push(Scene {
        name: "gallery/Double slit, a thin gap on the baffle".into(),
        document: thin_gap_double_slit(),
        regression: false,
    });
    scenes
}

/// The double slit with its baffle's span given a thin-gap coupling, the one
/// configuration the production packer corrects the stiffness rows for.
fn thin_gap_double_slit() -> TopologyDocument {
    let example = topology_examples::catalog()
        .iter()
        .find(|example| example.name == "Double slit")
        .expect("the double slit");
    let mut editor = TopologyEditor::from_document(example.document.clone()).unwrap();
    settle(&mut editor);
    let baffle = editor
        .document
        .model
        .draft
        .geometry
        .curves
        .last()
        .expect("the baffle")
        .spans
        .iter()
        .find(|span| matches!(span.behavior, SpanBehavior::Separated { .. }))
        .expect("a separated span")
        .id;
    editor
        .set_span_coupling(
            &BTreeSet::from([baffle]),
            InternalBoundaryCoupling::ThinGap {
                stiffness_ratio: 25.0,
            },
        )
        .unwrap();
    settle(&mut editor);
    assert_eq!(editor.acceptance, TopologyAcceptance::Valid);
    editor.document.clone()
}

fn settle(editor: &mut TopologyEditor) {
    for _ in 0..100_000 {
        editor.validate_frame(64);
        if editor.acceptance != TopologyAcceptance::Pending {
            return;
        }
    }
    panic!("validation did not finish");
}

fn mesh_hash(mesh: &TriMesh) -> u64 {
    let words = mesh
        .triangles
        .iter()
        .flat_map(|triangle| {
            triangle
                .vertices
                .iter()
                .map(|vertex| *vertex as u64)
                .chain([triangle.region.0])
        })
        .chain([mesh.vertices.len() as u64])
        .chain(
            mesh.vertices
                .iter()
                .flat_map(|vertex| [vertex.point.x.to_bits(), vertex.point.y.to_bits()]),
        );
    words
        .flat_map(u64::to_le_bytes)
        .fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
            (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3)
        })
}

struct Prepared {
    hash: u64,
    vertices: usize,
    triangles: usize,
    boundary: Option<Arc<CanonicalOutgoingBoundary>>,
    /// Whether the plan was packed both ways, which a driven generation is
    /// not: its plan has one packer.
    packed_both_ways: bool,
}

/// A fixed generation's plan through the production packer and through the
/// packer's own tests' one, word for word the same.
fn pack_both_ways(prepared: &PreparedTopology) -> Result<bool, String> {
    if prepared.canonical_temporal_operator.is_some() {
        return Ok(false);
    }
    let time_step = prepared.recommended_time_step();
    let state = CanonicalWaveState::zero(&prepared.canonical_operator, time_step)
        .map_err(|error| error.to_string())?;
    let clock = CanonicalGpuClock::initial(time_step).map_err(|error| format!("{error:?}"))?;
    let production = CanonicalGpuPlan::compile_with_quadratic(
        &prepared.canonical_operator,
        &prepared.operator,
        &state,
        &prepared.canonical_forcing,
        clock,
    )
    .map_err(|error| format!("the production packer: {error:?}"))?;
    let rebuilt = CanonicalGpuPlan::compile(
        &prepared.canonical_operator,
        &state,
        &prepared.canonical_forcing,
        clock,
    )
    .map_err(|error| format!("the rebuilding packer: {error:?}"))?;
    let differences = production.table_differences(&rebuilt);
    if !differences.is_empty() {
        return Err(format!(
            "the packer's two branches differ in {}",
            differences.join(", ")
        ));
    }
    Ok(true)
}

/// The app's own preparation at the document's own mesh edge.
fn prepare(editor: &TopologyEditor) -> Result<Prepared, String> {
    let domain = editor.document.model.accepted.geometry.domain;
    let edge = editor.document.presentation.mesh_edge;
    let options = MeshingOptions {
        target_edge_length: edge,
        curve_tolerance: (edge * 0.02).min(5.0e-4),
        ..MeshingOptions::default()
    }
    .sized_for_area(domain.width() * domain.height());
    let mut runtime = TopologyRuntime::default();
    runtime.request(
        editor.revision,
        &editor.document,
        editor.compiled_accepted.clone(),
        options,
        true,
    )?;
    for _ in 0..200_000_000 {
        if let Some(outcome) = runtime.advance(64) {
            outcome.map_err(|error| error.to_string())?;
            let prepared = runtime.ready().expect("a prepared topology");
            return Ok(Prepared {
                hash: mesh_hash(&prepared.mesh),
                vertices: prepared.mesh.vertices.len(),
                triangles: prepared.mesh.triangles.len(),
                boundary: prepared.canonical_operator.outgoing_boundary_handle(),
                packed_both_ways: pack_both_ways(prepared)?,
            });
        }
    }
    Err("preparation did not finish".into())
}

/// One scene's outcome: its report line, or what went wrong.
fn check(scene: &Scene) -> Result<String, String> {
    let started = Instant::now();
    let mut editor = TopologyEditor::from_document(scene.document.clone())
        .map_err(|error| format!("does not load: {error}"))?;
    settle(&mut editor);
    if let TopologyAcceptance::Invalid(issue) = &editor.acceptance {
        return Err(format!("refused at acceptance: {issue}"));
    }
    let prepared = prepare(&editor)?;
    let mut walls = String::from("no outgoing walls");
    if let Some(boundary) = &prepared.boundary {
        let trace = boundary.trace_report();
        if trace.modes != trace.trace_nodes
            || trace.smallest_eigenvalue < -2.0e-11 * trace.largest_eigenvalue.max(1.0)
            || trace.residual >= 1.0e-12
            || trace.orthogonality >= 1.0e-11
            || trace.damping_defect >= 1.0e-11
            || trace.auxiliary_count != trace.auxiliaries_expected
        {
            return Err(format!("the outgoing basis does not read back: {trace:?}"));
        }
        walls = format!(
            "{} trace nodes in {} pieces, residual {:.1e}",
            trace.trace_nodes, trace.pieces, trace.residual
        );
    }
    if scene.regression {
        let again = prepare(&editor)?;
        if again.hash != prepared.hash {
            return Err("the mesh differs between two preparations".into());
        }
        match (&prepared.boundary, &again.boundary) {
            (Some(first), Some(second)) if **first != **second => {
                return Err("the outgoing basis differs between two preparations".into());
            }
            (Some(_), None) | (None, Some(_)) => {
                return Err("one preparation has outgoing walls and the other none".into());
            }
            _ => {}
        }
    }
    Ok(format!(
        "{}: edge {}, {} vertices, {} triangles, {walls}{}{}, {:.2} s",
        scene.name,
        scene.document.presentation.mesh_edge,
        prepared.vertices,
        prepared.triangles,
        if prepared.packed_both_ways {
            ", packed both ways alike"
        } else {
            ", driven"
        },
        if scene.regression {
            ", prepared twice"
        } else {
            ""
        },
        started.elapsed().as_secs_f64()
    ))
}

/// PER-2 of the 2026-10-20 review: a version 22 scene written before a rule
/// it breaks opens in the editor with its draft invalid, naming the rule, and
/// its accepted scene as written, which the run refuses as it did before the
/// rule: the scene is there to be mended, not lost.
#[test]
fn a_scene_written_before_a_rule_opens_with_its_draft_invalid() {
    let bytes = std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/scenes-v22-before-rules/driven-span-meeting-a-pinned-one.json"),
    )
    .unwrap();
    let document = topology_persistence::parse_document(&bytes).expect("the scene opens");
    let mut editor = TopologyEditor::from_document(document.clone()).expect("the editor opens it");
    settle(&mut editor);
    match &editor.acceptance {
        TopologyAcceptance::Invalid(issue) => assert!(
            issue.to_string().contains("Dirichlet"),
            "the draft names another issue: {issue}"
        ),
        other => panic!("the draft is not invalid: {other:?}"),
    }
    assert!(
        editor.document.model.accepted == document.model.accepted,
        "the accepted scene is kept as written"
    );
    let refused = prepare(&editor)
        .err()
        .expect("the run refuses the accepted scene");
    assert!(refused.contains("Dirichlet"), "{refused}");
}

#[test]
fn every_regression_frozen_and_gallery_scene_prepares_at_its_own_edge() {
    let scenes = scenes();
    assert!(
        scenes.iter().any(|scene| scene.regression),
        "no regression scene; the first is the autosave of 12 October"
    );
    let workers = std::thread::available_parallelism()
        .map(|count| count.get())
        .unwrap_or(4)
        .min(scenes.len());
    let started = Instant::now();
    let outcomes = std::thread::scope(|scope| {
        let handles = (0..workers)
            .map(|worker| {
                let scenes = &scenes;
                scope.spawn(move || {
                    scenes
                        .iter()
                        .enumerate()
                        .skip(worker)
                        .step_by(workers)
                        .map(|(index, scene)| (index, check(scene)))
                        .collect::<Vec<_>>()
                })
            })
            .collect::<Vec<_>>();
        let mut outcomes = handles
            .into_iter()
            .flat_map(|handle| handle.join().expect("a worker"))
            .collect::<Vec<_>>();
        outcomes.sort_by_key(|(index, _)| *index);
        outcomes
    });
    let mut failures = Vec::new();
    for (index, outcome) in &outcomes {
        match outcome {
            Ok(line) => eprintln!("{line}"),
            Err(error) => {
                let name = &scenes[*index].name;
                eprintln!("FAILED {name}: {error}");
                failures.push(format!("{name}: {error}"));
            }
        }
    }
    eprintln!(
        "{} scenes prepared on {workers} threads in {:.1} s",
        outcomes.len() - failures.len(),
        started.elapsed().as_secs_f64()
    );
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
