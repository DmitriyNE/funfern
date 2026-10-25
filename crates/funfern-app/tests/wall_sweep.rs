//! Every wall law on every side of every span of every gallery scene, and on
//! every outer side, prepared the way the app prepares a document: the change
//! through the editor's own setters, validation to acceptance, then
//! `TopologyRuntime::request` at the Coarse preset's edge, 0.16, or the one
//! `FUNFERN_WALL_SWEEP_EDGE` names, sized for the domain. Each case
//! is classed before it runs, must succeed, or must be refused by the rule
//! that names it, and the count of successes must equal the count of
//! must-succeed cases, so a solver that politely refuses everything cannot
//! pass. A case that prepares has its outgoing basis read back against the
//! operator it was solved from, and a second preparation from a fresh runtime
//! must give the same mesh and the same basis, bit for bit.
//!
//! The mandatory set per scene: each of nine outer laws on each side alone
//! and on all four; each of nine face laws on each span's left, on its right
//! with the other side reflecting, and on both; each span transmitting and
//! separated; every pair of spans whose ends lie within an edge of each
//! other, both outgoing on the left and on both sides; a driven Dirichlet
//! beside a zero one on adjacent outer sides, which the corner rule refuses;
//! and a thin gap beside an outgoing side, which the editor refuses. Forty
//! mixed cases per scene from a fixed seed supplement it. The report per
//! scene says what was covered; nothing mandatory is omitted. The `Scene`
//! path's counterpart lives in `funfern-core`'s `canonical_wave` tests.
//!
//! Two refusals are predicted from the draft before validation runs, by the
//! rules that make them: a driven Dirichlet beside a differently driven one
//! on adjacent outer sides, which validation refuses at the corner, and two
//! Dirichlet traces meeting at a trace vertex of the mesh plan with
//! different signals, which validation refuses too since 12 October, as the
//! assembly always had at that node. For the second the sweep builds the
//! draft's plan itself, from the public topology compile and face
//! resolution, and walks it the way the assembly does; validation's own
//! check is not consulted.
//!
//! The gallery's three crystals and its two arrays, 5 scenes of 1,051 spans
//! among the gallery's 1,384, are arrays of one obstacle and carry two
//! thirds of the cases; the gate runs the other 40 scenes, about 13,000
//! cases, and the nightly runs all 45 with `FUNFERN_WALL_SWEEP_ALL=1`.
//! `FUNFERN_WALL_SWEEP=<name>` runs one scene, any scene, with its report.
//! `FUNFERN_WALL_SWEEP_EDGE=<edge>` overrides the edge of 0.16, the Coarse
//! preset, and `FUNFERN_WALL_SWEEP_PLAN=1` prints each scene's case counts
//! without preparing any. Every failure to prepare is collected and printed,
//! scene by scene, before the test fails.
//!
//! The sweep's first run, on 12 October, found 36 configurations the editor
//! accepted and the mesher could not build, every one a span changed between
//! transmitting and separated: a transmitting chain the face walked out and
//! back stayed in the face polygon. c3c58dc carried them in a `KNOWN` table,
//! counted apart and required to keep failing until fixed; the fix emptied
//! it the same day, and the table went with it. A configuration the mesher
//! cannot build is a failure here again.

use funfern_app::topology_editor::{TopologyAcceptance, TopologyEditor};
use funfern_app::topology_examples::{self, TopologyExample};
use funfern_app::topology_runtime::TopologyRuntime;
use funfern_core::{
    CanonicalOutgoingBoundary, CurveId, CurveSpanId, CurveTraceSide, FaceBoundaryCondition,
    InternalBoundaryCoupling, MeshingOptions, OuterBoundaryCondition, OuterSide, PhysicsModel,
    PlannedBoundarySource, Point2, SpanBehavior, TimeSignal, TopologyMeshPlan, TraceVertexId,
    TriMesh, compile_topology,
};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Instant;

const DEFAULT_EDGE: f64 = 0.16;

/// Every fourth must-succeed case is prepared twice, from a fresh runtime,
/// and the two must agree bit for bit; the check is about the preparation,
/// not the configuration, so a sample of the cases carries it.
const DETERMINISM_EVERY: usize = 4;

/// Arrays of one obstacle, run by the nightly alone.
const NIGHTLY_ONLY: [&str; 5] = [
    "Obstacle array",
    "Phased array",
    "Photonic crystal",
    "Crystal bend",
    "Disordered crystal",
];

fn edge() -> f64 {
    std::env::var("FUNFERN_WALL_SWEEP_EDGE")
        .ok()
        .and_then(|edge| edge.parse().ok())
        .unwrap_or(DEFAULT_EDGE)
}
const SUPPLEMENTARY: usize = 40;
const DRIVEN: TimeSignal = TimeSignal::Harmonic {
    offset: 0.0,
    amplitude: 1.0,
    frequency_hz: 2.0,
    phase_radians: 0.0,
};

fn outer_laws() -> [OuterBoundaryCondition; 9] {
    use OuterBoundaryCondition as Outer;
    [
        Outer::Reflecting,
        Outer::FirstOrderOutgoing,
        Outer::SecondOrderOutgoing,
        Outer::ElectricWall,
        Outer::MagneticWall,
        Outer::Neumann {
            signal: TimeSignal::ZERO,
        },
        Outer::Neumann { signal: DRIVEN },
        Outer::Dirichlet {
            signal: TimeSignal::ZERO,
        },
        Outer::Dirichlet { signal: DRIVEN },
    ]
}

fn face_laws() -> [FaceBoundaryCondition; 9] {
    use FaceBoundaryCondition as Face;
    [
        Face::Reflecting,
        Face::Impedance { ratio: 0.5 },
        Face::SecondOrderOutgoing,
        Face::ElectricWall,
        Face::MagneticWall,
        Face::Neumann {
            signal: TimeSignal::ZERO,
        },
        Face::Neumann { signal: DRIVEN },
        Face::Dirichlet {
            signal: TimeSignal::ZERO,
        },
        Face::Dirichlet { signal: DRIVEN },
    ]
}

#[derive(Clone, Debug)]
enum Change {
    Outer(BTreeSet<OuterSide>, OuterBoundaryCondition),
    /// A face law on one side, or on both when the side is absent.
    Face(CurveSpanId, Option<CurveTraceSide>, FaceBoundaryCondition),
    Behavior(CurveSpanId, SpanBehavior),
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Class {
    MustSucceed,
    /// The editor's setter refuses with this message.
    EditorRefuses(&'static str),
}

const CORNER: &str = "the outer boundary conditions disagree at a corner";
const TRACES: &str = "two Dirichlet conditions with different signals meet on one node";

struct Case {
    label: String,
    changes: Vec<Change>,
    class: Class,
    mandatory: bool,
}

#[derive(Default)]
struct Report {
    scene: String,
    spans: usize,
    mandatory: usize,
    supplementary: usize,
    must_succeed: usize,
    succeeded: usize,
    must_refuse: usize,
    refused_as_named: usize,
    /// Of the refusals, those the editor's setter made, those validation made
    /// at a corner, and those the assembly made at a shared node.
    by_editor: usize,
    by_corner: usize,
    by_shared_node: usize,
    /// Must-succeed cases prepared twice and compared.
    determinism_checks: usize,
    with_walls: usize,
    pieces: usize,
    residual: f64,
    orthogonality: f64,
    seconds: f64,
    /// Must-succeed cases that did not prepare, with the reason.
    failures: Vec<String>,
}

/// Every span's two end points, by curve, for the adjacency test.
fn span_ends(editor: &TopologyEditor) -> Vec<(CurveId, CurveSpanId, [Point2; 2])> {
    let mut ends = Vec::new();
    for curve in &editor.document.model.draft.geometry.curves {
        let nodes = curve.spline.node_count();
        for (index, span) in curve.spans.iter().enumerate() {
            let next = if curve.spline.is_open() {
                index + 1
            } else {
                (index + 1) % nodes
            };
            if let (Some(a), Some(b)) = (
                curve.spline.node_point(index),
                curve.spline.node_point(next),
            ) {
                ends.push((curve.id, span.id, [a, b]));
            }
        }
    }
    ends
}

/// For every pair of curves with span ends within an edge of each other,
/// the closest such pair of spans: the configuration of 12 October, two
/// walls an element apart, once per pair of curves rather than once per
/// pair of spans, which on an array of obstacles would be thousands.
fn adjacent_pairs(editor: &TopologyEditor) -> Vec<(CurveSpanId, CurveSpanId)> {
    let ends = span_ends(editor);
    let mut closest: BTreeMap<(CurveId, CurveId), (f64, CurveSpanId, CurveSpanId)> =
        BTreeMap::new();
    for (i, (curve_a, a, ends_a)) in ends.iter().enumerate() {
        for (curve_b, b, ends_b) in ends.iter().skip(i + 1) {
            if curve_a == curve_b {
                continue;
            }
            let distance = ends_a
                .iter()
                .flat_map(|p| {
                    ends_b
                        .iter()
                        .map(move |q| ((p.x - q.x).powi(2) + (p.y - q.y).powi(2)).sqrt())
                })
                .fold(f64::INFINITY, f64::min);
            if distance > edge() {
                continue;
            }
            let entry = closest
                .entry((*curve_a, *curve_b))
                .or_insert((f64::INFINITY, *a, *b));
            if distance < entry.0 {
                *entry = (distance, *a, *b);
            }
        }
    }
    closest.into_values().map(|(_, a, b)| (a, b)).collect()
}

/// Whether two adjacent outer sides resolve to Dirichlet with different
/// signals, which validation refuses at the corner.
fn corner_conflict(sides: [OuterBoundaryCondition; 4], physics: PhysicsModel) -> bool {
    [[0, 1], [1, 2], [2, 3], [3, 0]].into_iter().any(|[a, b]| {
        matches!(
            (sides[a].resolved(physics), sides[b].resolved(physics)),
            (
                OuterBoundaryCondition::Dirichlet { signal: x },
                OuterBoundaryCondition::Dirichlet { signal: y },
            ) if x != y
        )
    })
}

/// Whether two Dirichlet traces meet at a trace vertex of the draft's plan
/// with different signals, which validation refuses: the plan built here as
/// validation builds it and walked as the assembly walks it, over its
/// boundary edges and their trace vertices, with the laws the plan gives
/// each source resolved per physics. A draft whose plan cannot be built
/// predicts nothing; validation then refuses it for its own reason.
fn shared_node_conflict(editor: &TopologyEditor) -> bool {
    let scene = &editor.document.model.draft;
    let physics = scene.physics;
    let Ok(topology) = compile_topology(&scene.geometry, editor.revision) else {
        return false;
    };
    let Ok(assignments) = scene.resolve_face_assignments(&topology) else {
        return false;
    };
    let Ok(plan) = TopologyMeshPlan::new(&topology, &assignments) else {
        return false;
    };
    let mut signal_at: BTreeMap<TraceVertexId, TimeSignal> = BTreeMap::new();
    for boundary in &plan.boundaries {
        let signal = match boundary.source {
            PlannedBoundarySource::Outer(side) => {
                match scene.outer_boundaries.sides[side.index()].resolved(physics) {
                    OuterBoundaryCondition::Dirichlet { signal } => Some(signal),
                    _ => None,
                }
            }
            PlannedBoundarySource::Curve { side, .. } => match boundary.behavior {
                Some(SpanBehavior::Separated { left, right, .. }) => {
                    let face = match side {
                        CurveTraceSide::Left => left,
                        CurveTraceSide::Right => right,
                    };
                    match face.resolved(physics) {
                        FaceBoundaryCondition::Dirichlet { signal } => Some(signal),
                        _ => None,
                    }
                }
                _ => None,
            },
        };
        let Some(signal) = signal else {
            continue;
        };
        for trace in boundary.traces {
            match signal_at.get(&trace) {
                Some(previous) if *previous != signal => return true,
                Some(_) => {}
                None => {
                    signal_at.insert(trace, signal);
                }
            }
        }
    }
    false
}

fn cases(editor: &TopologyEditor, seed: u64) -> Vec<Case> {
    let mut cases = Vec::new();
    for law in outer_laws() {
        for side in OuterSide::ALL {
            cases.push(Case {
                label: format!("outer {side:?} {law:?}"),
                changes: vec![Change::Outer(BTreeSet::from([side]), law)],
                class: Class::MustSucceed,
                mandatory: true,
            });
        }
        cases.push(Case {
            label: format!("outer all {law:?}"),
            changes: vec![Change::Outer(BTreeSet::from(OuterSide::ALL), law)],
            class: Class::MustSucceed,
            mandatory: true,
        });
    }
    for [driven, zero] in [[0, 1], [1, 2], [2, 3], [3, 0]] {
        let (driven, zero) = (OuterSide::ALL[driven], OuterSide::ALL[zero]);
        cases.push(Case {
            label: format!("corner: driven Dirichlet on {driven:?}, zero on {zero:?}"),
            changes: vec![
                Change::Outer(
                    BTreeSet::from([driven]),
                    OuterBoundaryCondition::Dirichlet { signal: DRIVEN },
                ),
                Change::Outer(
                    BTreeSet::from([zero]),
                    OuterBoundaryCondition::Dirichlet {
                        signal: TimeSignal::ZERO,
                    },
                ),
            ],
            class: Class::MustSucceed,
            mandatory: true,
        });
    }
    let spans = editor
        .document
        .model
        .draft
        .geometry
        .curves
        .iter()
        .flat_map(|curve| curve.spans.iter().map(|span| span.id))
        .collect::<Vec<_>>();
    for &span in &spans {
        for law in face_laws() {
            for (which, side) in [
                ("left", Some(CurveTraceSide::Left)),
                ("right", Some(CurveTraceSide::Right)),
                ("both", None),
            ] {
                cases.push(Case {
                    label: format!("span {} {which} {law:?}", span.0),
                    changes: vec![Change::Face(span, side, law)],
                    class: Class::MustSucceed,
                    mandatory: true,
                });
            }
        }
        for (which, behavior) in [
            ("transmitting", SpanBehavior::Transmitting),
            ("separated", SpanBehavior::REFLECTING),
        ] {
            cases.push(Case {
                label: format!("span {} {which}", span.0),
                changes: vec![Change::Behavior(span, behavior)],
                class: Class::MustSucceed,
                mandatory: true,
            });
        }
        cases.push(Case {
            label: format!("span {} thin gap beside an outgoing side", span.0),
            changes: vec![Change::Behavior(
                span,
                SpanBehavior::Separated {
                    left: FaceBoundaryCondition::SecondOrderOutgoing,
                    right: FaceBoundaryCondition::Reflecting,
                    coupling: InternalBoundaryCoupling::ThinGap {
                        stiffness_ratio: 25.0,
                    },
                },
            )],
            class: Class::EditorRefuses("Boundary behavior is invalid"),
            mandatory: true,
        });
    }
    for (a, b) in adjacent_pairs(editor) {
        for (which, side) in [("left", Some(CurveTraceSide::Left)), ("both", None)] {
            cases.push(Case {
                label: format!("adjacent spans {} and {} outgoing {which}", a.0, b.0),
                changes: vec![
                    Change::Face(a, side, FaceBoundaryCondition::SecondOrderOutgoing),
                    Change::Face(b, side, FaceBoundaryCondition::SecondOrderOutgoing),
                ],
                class: Class::MustSucceed,
                mandatory: true,
            });
        }
    }
    // The supplementary sample: two face changes and an outer change at once.
    let mut state = 0x9E37_79B9_7F4A_7C15_u64 ^ (seed + 1).wrapping_mul(0xD1B5_4A32_D192_ED03);
    let mut next = move |bound: usize| {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        (state % bound as u64) as usize
    };
    let sides = [
        Some(CurveTraceSide::Left),
        Some(CurveTraceSide::Right),
        None,
    ];
    for sample in 0..SUPPLEMENTARY {
        let mut changes = Vec::new();
        if !spans.is_empty() {
            for _ in 0..2 {
                changes.push(Change::Face(
                    spans[next(spans.len())],
                    sides[next(3)],
                    face_laws()[next(9)],
                ));
            }
        }
        changes.push(Change::Outer(
            BTreeSet::from([OuterSide::ALL[next(4)]]),
            outer_laws()[next(9)],
        ));
        let described = changes
            .iter()
            .map(|change| match change {
                Change::Face(span, Some(side), law) => format!("span {} {side:?} {law:?}", span.0),
                Change::Face(span, None, law) => format!("span {} both {law:?}", span.0),
                Change::Behavior(span, behavior) => format!("span {} {behavior:?}", span.0),
                Change::Outer(sides, law) => format!("outer {sides:?} {law:?}"),
            })
            .collect::<Vec<_>>()
            .join(", ");
        cases.push(Case {
            label: format!("mixed {sample}: {described}"),
            changes,
            class: Class::MustSucceed,
            mandatory: false,
        });
    }
    cases
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

/// Applies a change through the editor; a setter that finds nothing to
/// change, because the law is already there, leaves the scene as it is.
fn apply(editor: &mut TopologyEditor, change: &Change) -> Result<(), String> {
    let result = match change {
        Change::Outer(sides, law) => editor.set_outer_condition(sides, *law),
        Change::Face(span, Some(side), law) => {
            editor.set_span_face_condition(&BTreeSet::from([*span]), *side, *law)
        }
        Change::Face(span, None, law) => {
            let spans = BTreeSet::from([*span]);
            editor
                .set_span_face_condition(&spans, CurveTraceSide::Left, *law)
                .or_else(already)?;
            editor.set_span_face_condition(&spans, CurveTraceSide::Right, *law)
        }
        Change::Behavior(span, behavior) => {
            editor.set_span_behavior(&BTreeSet::from([*span]), *behavior)
        }
    };
    result.or_else(already)
}

fn already(error: String) -> Result<(), String> {
    if error == "Select existing curve spans" {
        Ok(())
    } else {
        Err(error)
    }
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

fn prepare(
    editor: &TopologyEditor,
) -> Result<(u64, Option<Arc<CanonicalOutgoingBoundary>>), String> {
    let domain = editor.document.model.accepted.geometry.domain;
    let edge = edge();
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
    for _ in 0..20_000_000 {
        if let Some(outcome) = runtime.advance(64) {
            outcome.map_err(|error| error.to_string())?;
            let prepared = runtime.ready().expect("a prepared topology");
            return Ok((
                mesh_hash(&prepared.mesh),
                prepared.canonical_operator.outgoing_boundary_handle(),
            ));
        }
    }
    Err("preparation did not finish".into())
}

fn sweep(example: &TopologyExample, seed: u64) -> Report {
    let started = Instant::now();
    let base = TopologyEditor::from_document(example.document.clone())
        .unwrap_or_else(|error| panic!("{}: {error}", example.name));
    let mut base = base;
    settle(&mut base);
    assert_eq!(
        base.acceptance,
        TopologyAcceptance::Valid,
        "{} is not valid as authored",
        example.name
    );
    let cases = cases(&base, seed);
    let plan_only = std::env::var_os("FUNFERN_WALL_SWEEP_PLAN").is_some();
    let mut report = Report {
        scene: example.name.to_owned(),
        spans: span_ends(&base).len(),
        ..Report::default()
    };
    for case in &cases {
        if case.mandatory {
            report.mandatory += 1;
        } else {
            report.supplementary += 1;
        }
        if plan_only {
            continue;
        }
        let name = example.name;
        let label = &case.label;
        let mut editor = TopologyEditor::from_document(example.document.clone()).unwrap();
        settle(&mut editor);
        let mut refused = false;
        for change in &case.changes {
            match (apply(&mut editor, change), case.class) {
                (Ok(()), _) => {}
                (Err(error), Class::EditorRefuses(expected)) if error == expected => {
                    refused = true;
                    break;
                }
                (Err(error), _) => panic!("{name}: {label}: the editor refused: {error}"),
            }
        }
        if let Class::EditorRefuses(expected) = case.class {
            report.must_refuse += 1;
            assert!(
                refused,
                "{name}: {label}: the editor did not refuse with '{expected}'"
            );
            report.refused_as_named += 1;
            report.by_editor += 1;
            continue;
        }
        // The two rules validation applies, predicted from the draft: the
        // corner rule from its outer sides, the trace rule from its own plan.
        // The corner rule runs first in validation, so it is the one a draft
        // that breaks both is refused by.
        let corner = corner_conflict(
            editor.document.model.draft.outer_boundaries.sides,
            editor.document.model.draft.physics,
        );
        let shared = !corner && shared_node_conflict(&editor);
        let expected = if corner {
            Some((CORNER, "its outer sides disagree at a corner"))
        } else if shared {
            Some((
                TRACES,
                "two Dirichlet signals meet on one of its trace vertices",
            ))
        } else {
            None
        };
        settle(&mut editor);
        match (&editor.acceptance, expected) {
            (TopologyAcceptance::Invalid(issue), Some((message, _)))
                if issue.to_string() == message =>
            {
                report.must_refuse += 1;
                report.refused_as_named += 1;
                if corner {
                    report.by_corner += 1;
                } else {
                    report.by_shared_node += 1;
                }
                continue;
            }
            (TopologyAcceptance::Invalid(issue), _) => {
                report.must_succeed += 1;
                report
                    .failures
                    .push(format!("{label}: refused at acceptance: {issue}"));
                continue;
            }
            (_, Some((_, why))) => {
                report.must_refuse += 1;
                report
                    .failures
                    .push(format!("{label}: accepted although {why}"));
                continue;
            }
            _ => {}
        }
        let prepared = prepare(&editor);
        report.must_succeed += 1;
        let (hash, boundary) = match prepared {
            Ok(prepared) => prepared,
            Err(error) => {
                report.failures.push(format!("{label}: {error}"));
                continue;
            }
        };
        if report.must_succeed.is_multiple_of(DETERMINISM_EVERY) {
            report.determinism_checks += 1;
            let (again, boundary_again) =
                prepare(&editor).unwrap_or_else(|error| panic!("{name}: {label}: {error}"));
            assert_eq!(
                hash, again,
                "{name}: {label}: the mesh differs between two preparations"
            );
            match (&boundary, &boundary_again) {
                (Some(first), Some(second)) => assert!(
                    **first == **second,
                    "{name}: {label}: the outgoing basis differs between two preparations"
                ),
                (None, None) => {}
                _ => {
                    panic!("{name}: {label}: one preparation has outgoing walls and the other none")
                }
            }
        }
        if let Some(boundary) = boundary {
            report.with_walls += 1;
            let trace = boundary.trace_report();
            assert_eq!(trace.modes, trace.trace_nodes, "{name}: {label}: {trace:?}");
            assert!(
                trace.smallest_eigenvalue >= -2.0e-11 * trace.largest_eigenvalue.max(1.0),
                "{name}: {label}: {trace:?}"
            );
            assert!(trace.residual < 1.0e-12, "{name}: {label}: {trace:?}");
            assert!(trace.orthogonality < 1.0e-11, "{name}: {label}: {trace:?}");
            assert!(trace.damping_defect < 1.0e-11, "{name}: {label}: {trace:?}");
            assert_eq!(
                trace.auxiliary_count, trace.auxiliaries_expected,
                "{name}: {label}: {trace:?}"
            );
            report.pieces = report.pieces.max(trace.pieces);
            report.residual = report.residual.max(trace.residual);
            report.orthogonality = report.orthogonality.max(trace.orthogonality);
        }
        report.succeeded += 1;
    }
    report.seconds = started.elapsed().as_secs_f64();
    report
}

fn print(report: &Report) {
    eprintln!(
        "{}: {} spans, {} mandatory + {} supplementary; {} of {} prepared ({} twice), {} of {} refused as named ({} by the editor, {} at a corner, {} at a shared node), {} with outgoing walls, pieces up to {}, residual {:.1e}, orthogonality {:.1e}, {:.1} s",
        report.scene,
        report.spans,
        report.mandatory,
        report.supplementary,
        report.succeeded,
        report.must_succeed,
        report.determinism_checks,
        report.refused_as_named,
        report.must_refuse,
        report.by_editor,
        report.by_corner,
        report.by_shared_node,
        report.with_walls,
        report.pieces,
        report.residual,
        report.orthogonality,
        report.seconds
    );
    for failure in &report.failures {
        eprintln!("  FAILED {failure}");
    }
}

#[test]
fn every_wall_law_on_every_side_of_every_gallery_scene_prepares() {
    let catalog = topology_examples::catalog();
    let only = std::env::var("FUNFERN_WALL_SWEEP").ok();
    let all = std::env::var_os("FUNFERN_WALL_SWEEP_ALL").is_some();
    let selected = catalog
        .iter()
        .enumerate()
        .filter(|(_, example)| match &only {
            Some(name) => example.name == name,
            None => all || !NIGHTLY_ONLY.contains(&example.name),
        })
        .collect::<Vec<_>>();
    assert!(!selected.is_empty(), "no scene named {only:?}");
    if only.is_none() && !all {
        eprintln!("omitted until the nightly: {}", NIGHTLY_ONLY.join(", "));
    }
    let workers = std::thread::available_parallelism()
        .map(|count| count.get())
        .unwrap_or(4)
        .min(selected.len());
    let started = Instant::now();
    let reports = std::thread::scope(|scope| {
        let handles = (0..workers)
            .map(|worker| {
                let selected = &selected;
                scope.spawn(move || {
                    selected
                        .iter()
                        .skip(worker)
                        .step_by(workers)
                        .map(|(index, example)| (*index, sweep(example, *index as u64)))
                        .collect::<Vec<_>>()
                })
            })
            .collect::<Vec<_>>();
        let mut reports = handles
            .into_iter()
            .flat_map(|handle| handle.join().expect("a sweep worker"))
            .collect::<Vec<_>>();
        reports.sort_by_key(|(index, _)| *index);
        reports
    });
    let mut totals = Report::default();
    for (_, report) in &reports {
        print(report);
    }
    let failures = reports
        .iter()
        .map(|(_, report)| report.failures.len())
        .sum::<usize>();
    assert_eq!(
        failures, 0,
        "{failures} cases did not prepare; see the report above"
    );
    for (_, report) in &reports {
        assert_eq!(report.succeeded, report.must_succeed, "{}", report.scene);
        assert_eq!(
            report.refused_as_named, report.must_refuse,
            "{}",
            report.scene
        );
        totals.mandatory += report.mandatory;
        totals.supplementary += report.supplementary;
        totals.must_succeed += report.must_succeed;
        totals.succeeded += report.succeeded;
        totals.must_refuse += report.must_refuse;
        totals.refused_as_named += report.refused_as_named;
        totals.by_editor += report.by_editor;
        totals.by_corner += report.by_corner;
        totals.by_shared_node += report.by_shared_node;
        totals.determinism_checks += report.determinism_checks;
        totals.spans += report.spans;
        totals.with_walls += report.with_walls;
        totals.pieces = totals.pieces.max(report.pieces);
        totals.residual = totals.residual.max(report.residual);
        totals.orthogonality = totals.orthogonality.max(report.orthogonality);
    }
    eprintln!(
        "gallery at edge {}: {} scenes, {} spans, {} mandatory + {} supplementary cases, {} prepared ({} twice), {} refused as named ({} by the editor, {} at a corner, {} at a shared node), {} with outgoing walls, pieces up to {}, residual {:.1e}, orthogonality {:.1e}, {:.1} s on {workers} threads",
        edge(),
        reports.len(),
        totals.spans,
        totals.mandatory,
        totals.supplementary,
        totals.succeeded,
        totals.determinism_checks,
        totals.refused_as_named,
        totals.by_editor,
        totals.by_corner,
        totals.by_shared_node,
        totals.with_walls,
        totals.pieces,
        totals.residual,
        totals.orthogonality,
        started.elapsed().as_secs_f64()
    );
    if std::env::var_os("FUNFERN_WALL_SWEEP_PLAN").is_some() {
        return;
    }
    assert_eq!(totals.succeeded, totals.must_succeed);
    assert_eq!(totals.refused_as_named, totals.must_refuse);
    if only.is_none() {
        assert!(
            totals.mandatory > 2_000,
            "{} mandatory cases",
            totals.mandatory
        );
    }
}
