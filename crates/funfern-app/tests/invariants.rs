//! Milestone C's invariants (`docs/spikes/funfern-testing-campaign-plan.md`,
//! section 6j): relations two runs of the app must keep between them, with
//! no reference solution.
//!
//! IV-03, mesh renumbering. A mesh with its vertices, its triangles, each
//! triangle's first vertex and its boundary edges in another order is the
//! same discretization and must give the same solution. Every regression,
//! frozen and gallery scene is prepared the way the app prepares a document,
//! at its own edge; its mesh, shuffled, is handed back through
//! `TopologyRuntime::request_adapted`, the path an adaptation's mesh takes,
//! and the same mesh unshuffled goes through that path too, as the control.
//! Each generation is struck by one smooth pulse and run on the CPU with its
//! own forcing; the shuffled run, mapped back element by element, must match
//! the control to roundoff in its fields, its energy and what its point and
//! segment probes read, and the control must match the fresh preparation
//! bit for bit.

use funfern_app::topology_editor::{TopologyAcceptance, TopologyDocument, TopologyEditor};
use funfern_app::topology_examples;
use funfern_app::topology_persistence;
use funfern_app::topology_runtime::{
    PreparedTopology, TopologyProbeCompilation, TopologyProbeStencil, TopologyRuntime,
};
use funfern_core::{
    BoundaryEdge, CanonicalPointStencil, CanonicalTemporalWaveState, CanonicalWaveOperator,
    CanonicalWaveState, MeshTriangle, MeshingOptions, Point2, TriMesh,
};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

/// Steps each run takes after its pulse.
const STEPS: u64 = 64;

/// How far the shuffled run may stand from the control in any quantity,
/// relatively: roundoff, the order of the sums being all that changes. On
/// 2026-10-31 the worst of the 91 scenes read 8.9e-14, the bent fiber's
/// segment probe, and a locator that read points on shared edges from the
/// first triangle in the mesh's order read the fiber amplifier's 1.3e-1.
const ROUNDOFF: f64 = 1.0e-12;

/// The six complementary samples each element owns, in element order.
const SAMPLES: usize = 6;

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
}

fn scenes() -> Vec<Scene> {
    let mut scenes = Vec::new();
    for folder in ["scenes-regressions", "scenes-v22"] {
        for (name, bytes) in fixtures(folder) {
            let document = topology_persistence::parse_document(&bytes)
                .unwrap_or_else(|error| panic!("{folder}/{name} does not open: {error}"));
            scenes.push(Scene {
                name: format!("{folder}/{name}"),
                document,
            });
        }
    }
    for example in topology_examples::catalog() {
        scenes.push(Scene {
            name: format!("gallery/{}", example.name),
            document: example.document.clone(),
        });
    }
    scenes
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

/// Runs the runtime's preparation to its end and publishes it.
fn finish(runtime: &mut TopologyRuntime) -> Result<Arc<PreparedTopology>, String> {
    for _ in 0..200_000_000 {
        if let Some(outcome) = runtime.advance(64) {
            let token = outcome.map_err(|error| error.to_string())?;
            return runtime.commit_ready(token);
        }
    }
    Err("preparation did not finish".into())
}

/// SplitMix64, for shuffles that are the same on every run.
struct Random(u64);

impl Random {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    /// Where each of `count` items goes: a Fisher-Yates shuffle.
    fn permutation(&mut self, count: usize) -> Vec<usize> {
        let mut order = (0..count).collect::<Vec<_>>();
        for index in (1..count).rev() {
            order.swap(index, (self.next() % (index as u64 + 1)) as usize);
        }
        order
    }
}

/// `mesh` with its vertices, triangles and boundary edges in another order
/// and each triangle starting at another of its vertices, under `revision`,
/// and where each triangle went.
fn shuffled(mesh: &TriMesh, revision: u64, seed: u64) -> (TriMesh, Vec<usize>) {
    let mut random = Random(seed);
    let vertices = random.permutation(mesh.vertices.len());
    let triangles = random.permutation(mesh.triangles.len());
    let edges = random.permutation(mesh.boundary_edges.len());
    let mut out = mesh.clone();
    out.mesh_revision = revision;
    for (old, vertex) in mesh.vertices.iter().enumerate() {
        out.vertices[vertices[old]] = *vertex;
    }
    for (old, triangle) in mesh.triangles.iter().enumerate() {
        let turn = (random.next() % 3) as usize;
        let mapped = triangle.vertices.map(|vertex| vertices[vertex]);
        out.triangles[triangles[old]] = MeshTriangle {
            vertices: std::array::from_fn(|corner| mapped[(corner + turn) % 3]),
            region: triangle.region,
        };
    }
    for (old, size) in mesh.requested_sizes.iter().enumerate() {
        out.requested_sizes[triangles[old]] = *size;
    }
    for (old, edge) in mesh.boundary_edges.iter().enumerate() {
        out.boundary_edges[edges[old]] = BoundaryEdge {
            vertices: edge.vertices.map(|vertex| vertices[vertex]),
            ..*edge
        };
    }
    (out, triangles)
}

/// Where each of `from`'s primary nodes and complementary samples lies in
/// `to`, whose element `elements[e]` is `from`'s element `e`: matched within
/// each element by position, which tells apart the two sides of a wall.
fn correspondence(
    from: &CanonicalWaveOperator,
    to: &CanonicalWaveOperator,
    elements: &[usize],
) -> Result<(Vec<usize>, Vec<usize>), String> {
    let (from_elements, to_elements) = (from.element_nodes(), to.element_nodes());
    if from_elements.len() != elements.len()
        || to_elements.len() != elements.len()
        || from.degrees_of_freedom() != to.degrees_of_freedom()
        || from.constitutive_samples().len() != to.constitutive_samples().len()
    {
        return Err("the shuffled generation's sizes differ".into());
    }
    let nearest = |point: Point2, candidates: &mut dyn Iterator<Item = (usize, Point2)>| {
        candidates
            .map(|(index, other)| (index, (other - point).norm()))
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .filter(|(_, distance)| *distance <= 1.0e-12)
            .map(|(index, _)| index)
    };
    let mut nodes = vec![usize::MAX; from.degrees_of_freedom()];
    let mut samples = vec![usize::MAX; from.constitutive_samples().len()];
    for (element, target) in elements.iter().enumerate() {
        for node in from_elements[element] {
            let point = from.node_points()[node as usize];
            let found = nearest(
                point,
                &mut to_elements[*target]
                    .iter()
                    .map(|other| (*other as usize, to.node_points()[*other as usize])),
            )
            .ok_or_else(|| format!("no node of element {target} lies at {point:?}"))?;
            let slot = &mut nodes[node as usize];
            if *slot != usize::MAX && *slot != found {
                return Err(format!("node {node} lies at two shuffled nodes"));
            }
            *slot = found;
        }
        let owned = element * SAMPLES..(element + 1) * SAMPLES;
        for (slot, sample) in samples[owned.clone()].iter_mut().zip(owned) {
            let point = from.constitutive_samples()[sample].point;
            *slot = nearest(
                point,
                &mut (target * SAMPLES..(target + 1) * SAMPLES)
                    .map(|other| (other, to.constitutive_samples()[other].point)),
            )
            .ok_or_else(|| format!("no sample of element {target} lies at {point:?}"))?;
        }
    }
    let mut seen = nodes.clone();
    seen.sort_unstable();
    seen.dedup();
    if seen.len() != nodes.len() || seen.last() == Some(&usize::MAX) {
        return Err("the node correspondence is not one to one".into());
    }
    Ok((nodes, samples))
}

#[derive(Clone)]
enum Run {
    Fixed(CanonicalWaveState),
    Temporal(CanonicalTemporalWaveState),
}

impl Run {
    /// At rest, then struck by a smooth pulse centred off the domain's
    /// middle, where few scenes put anything.
    fn struck(prepared: &PreparedTopology, time_step: f64) -> Result<Self, String> {
        let domain = prepared.bundle.authored.geometry.domain;
        let centre = Point2::new(
            domain.min_x + 0.37 * domain.width(),
            domain.min_y + 0.41 * domain.height(),
        );
        let width = 0.15 * domain.width().min(domain.height());
        let increment = prepared
            .canonical_operator
            .node_points()
            .iter()
            .map(|point| 0.1 * (-(*point - centre).dot(*point - centre) / (width * width)).exp())
            .collect::<Vec<_>>();
        let forcing = &prepared.canonical_forcing;
        match &prepared.canonical_temporal_operator {
            Some(temporal) => {
                let mut state = CanonicalTemporalWaveState::zero(temporal, time_step)
                    .map_err(|error| error.to_string())?;
                state
                    .apply_primary_pulse(temporal, forcing, &increment)
                    .map_err(|error| error.to_string())?;
                Ok(Run::Temporal(state))
            }
            None => {
                let mut state = CanonicalWaveState::zero(&prepared.canonical_operator, time_step)
                    .map_err(|error| error.to_string())?;
                state
                    .apply_primary_pulse(&prepared.canonical_operator, forcing, &increment)
                    .map_err(|error| error.to_string())?;
                Ok(Run::Fixed(state))
            }
        }
    }

    fn step(&mut self, prepared: &PreparedTopology) -> Result<(), String> {
        let forcing = &prepared.canonical_forcing;
        match (self, &prepared.canonical_temporal_operator) {
            (Run::Fixed(state), _) => state
                .step_with_forcing(&prepared.canonical_operator, forcing)
                .map(|_| ()),
            (Run::Temporal(state), Some(temporal)) => {
                state.step_with_forcing(temporal, forcing).map(|_| ())
            }
            (Run::Temporal(_), None) => unreachable!("a temporal state on a fixed generation"),
        }
        .map_err(|error| error.to_string())
    }

    fn primary(&self) -> &[f64] {
        match self {
            Run::Fixed(state) => state.primary_flux(),
            Run::Temporal(state) => state.primary_flux(),
        }
    }

    fn complementary(&self) -> &[Point2] {
        match self {
            Run::Fixed(state) => state.complementary_flux(),
            Run::Temporal(state) => state.complementary_flux(),
        }
    }

    fn integrated(&self) -> &[f64] {
        match self {
            Run::Fixed(_) => &[],
            Run::Temporal(state) => state.integrated_field(),
        }
    }

    fn energy(&self, prepared: &PreparedTopology) -> Result<f64, String> {
        match (self, &prepared.canonical_temporal_operator) {
            (Run::Fixed(state), _) => state.energy(&prepared.canonical_operator),
            (Run::Temporal(state), Some(temporal)) => state.energy(temporal),
            (Run::Temporal(_), None) => unreachable!("a temporal state on a fixed generation"),
        }
        .map_err(|error| error.to_string())
    }

    fn bits(&self) -> Vec<u64> {
        self.primary()
            .iter()
            .chain(self.integrated())
            .map(|value| value.to_bits())
            .chain(
                self.complementary()
                    .iter()
                    .flat_map(|value| [value.x.to_bits(), value.y.to_bits()]),
            )
            .collect()
    }
}

/// `prepared` struck and run `STEPS` steps at `time_step`.
fn run(prepared: &PreparedTopology, time_step: f64) -> Result<Run, String> {
    let mut run = Run::struck(prepared, time_step)?;
    for step in 0..STEPS {
        run.step(prepared)
            .map_err(|error| format!("step {step}: {error}"))?;
    }
    Ok(run)
}

/// `‖a∘map − b‖ / ‖b‖`, against an absolute floor for a field that is zero.
fn apart<T: Copy>(
    read: &[T],
    map: impl Fn(usize) -> usize,
    expected: &[T],
    square: impl Fn(T, T) -> (f64, f64),
) -> f64 {
    let (difference, norm) = (0..expected.len()).fold((0.0, 0.0), |(difference, norm), index| {
        let (d, n) = square(read[map(index)], expected[index]);
        (difference + d, norm + n)
    });
    (difference / norm.max(f64::MIN_POSITIVE)).sqrt()
}

fn scalar(a: f64, b: f64) -> (f64, f64) {
    ((a - b) * (a - b), b * b)
}

fn vector(a: Point2, b: Point2) -> (f64, f64) {
    ((a - b).dot(a - b), b.dot(b))
}

/// What a probe reads: a point's field and complementary field, or a
/// segment's summed over its points.
fn readings(prepared: &PreparedTopology, run: &Run) -> Result<Vec<(f64, Point2)>, String> {
    let operator = &prepared.canonical_operator;
    let read = |stencil: &CanonicalPointStencil| {
        stencil
            .sample(run.primary(), run.primary(), run.complementary())
            .map(|sample| (sample.primary, sample.complementary))
            .map_err(|error| error.to_string())
    };
    let mut readings = Vec::new();
    for probe in prepared.probes.iter() {
        let TopologyProbeCompilation::Ready(stencil) = &probe.result else {
            continue;
        };
        match stencil.as_ref() {
            TopologyProbeStencil::Point(point) => {
                let stencil = CanonicalPointStencil::from_quadratic(*point, operator)
                    .map_err(|error| error.to_string())?;
                readings.push(read(&stencil)?);
            }
            TopologyProbeStencil::Segment(points) => {
                let mut sum = (0.0, Point2::default());
                for point in points {
                    let stencil = CanonicalPointStencil::from_quadratic_own(*point, operator)
                        .map_err(|error| error.to_string())?;
                    let (primary, complementary) = read(&stencil)?;
                    sum = (sum.0 + primary, sum.1 + complementary);
                }
                readings.push(sum);
            }
            TopologyProbeStencil::Boundary(_) | TopologyProbeStencil::Area(_) => {}
        }
    }
    Ok(readings)
}

/// How far the shuffled generation stands from the control, quantity by
/// quantity.
#[derive(Clone, Copy, Debug, Default)]
struct Apart {
    time_step: f64,
    primary: f64,
    complementary: f64,
    integrated: f64,
    energy: f64,
    probe_primary: f64,
    probe_complementary: f64,
}

impl Apart {
    fn worst(self) -> f64 {
        [
            self.time_step,
            self.primary,
            self.complementary,
            self.integrated,
            self.energy,
            self.probe_primary,
            self.probe_complementary,
        ]
        .into_iter()
        .fold(0.0, f64::max)
    }
}

fn renumbered(scene: &Scene) -> Result<(String, Apart), String> {
    let started = Instant::now();
    let mut editor = TopologyEditor::from_document(scene.document.clone())
        .map_err(|error| format!("does not load: {error}"))?;
    settle(&mut editor);
    if let TopologyAcceptance::Invalid(issue) = &editor.acceptance {
        return Err(format!("refused at acceptance: {issue}"));
    }
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
    let fresh = finish(&mut runtime)?;
    let mut same = (*fresh.mesh).clone();
    same.mesh_revision = runtime.reserve_mesh_revision();
    runtime.request_adapted(editor.revision, &editor.document, same)?;
    let control = finish(&mut runtime)?;
    let revision = runtime.reserve_mesh_revision();
    let (mesh, elements) = shuffled(&fresh.mesh, revision, 0x1f03_2026);
    runtime.request_adapted(editor.revision, &editor.document, mesh)?;
    let shuffled = finish(&mut runtime)?;

    let time_step = control.recommended_time_step();
    let fresh_run = run(&fresh, fresh.recommended_time_step())?;
    let control_run = run(&control, time_step)?;
    if fresh_run.bits() != control_run.bits() {
        return Err("the control, the same mesh through adaptation's path, is not the fresh preparation's run bit for bit".into());
    }
    let shuffled_run = run(&shuffled, time_step)?;
    let (nodes, samples) = correspondence(
        &control.canonical_operator,
        &shuffled.canonical_operator,
        &elements,
    )?;
    let (control_readings, shuffled_readings) = (
        readings(&control, &control_run)?,
        readings(&shuffled, &shuffled_run)?,
    );
    if control_readings.len() != shuffled_readings.len() {
        return Err("the shuffled generation compiled other probes".into());
    }
    let apart = Apart {
        time_step: (shuffled.recommended_time_step() / time_step - 1.0).abs(),
        primary: apart(
            shuffled_run.primary(),
            |node| nodes[node],
            control_run.primary(),
            scalar,
        ),
        complementary: apart(
            shuffled_run.complementary(),
            |sample| samples[sample],
            control_run.complementary(),
            vector,
        ),
        integrated: if control_run.integrated().is_empty() {
            0.0
        } else {
            apart(
                shuffled_run.integrated(),
                |node| nodes[node],
                control_run.integrated(),
                scalar,
            )
        },
        energy: (shuffled_run.energy(&shuffled)? / control_run.energy(&control)? - 1.0).abs(),
        probe_primary: apart(
            &shuffled_readings
                .iter()
                .map(|reading| reading.0)
                .collect::<Vec<_>>(),
            |probe| probe,
            &control_readings
                .iter()
                .map(|reading| reading.0)
                .collect::<Vec<_>>(),
            scalar,
        ),
        probe_complementary: apart(
            &shuffled_readings
                .iter()
                .map(|reading| reading.1)
                .collect::<Vec<_>>(),
            |probe| probe,
            &control_readings
                .iter()
                .map(|reading| reading.1)
                .collect::<Vec<_>>(),
            vector,
        ),
    };
    if apart.worst() > ROUNDOFF {
        return Err(format!(
            "the shuffled mesh's run stands {apart:?} from the control's"
        ));
    }
    Ok((
        format!(
            "{}: {} vertices, {} probes read, time step {:.1e}, Q {:.1e}, b {:.1e}, r {:.1e}, energy {:.1e}, probes {:.1e} and {:.1e}, {:.2} s",
            scene.name,
            fresh.mesh.vertices.len(),
            control_readings.len(),
            apart.time_step,
            apart.primary,
            apart.complementary,
            apart.integrated,
            apart.energy,
            apart.probe_primary,
            apart.probe_complementary,
            started.elapsed().as_secs_f64()
        ),
        apart,
    ))
}

#[test]
fn a_shuffled_mesh_gives_every_scene_the_same_solution() {
    let scenes = scenes();
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
                        .map(|(index, scene)| (index, renumbered(scene)))
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
    let mut worst: f64 = 0.0;
    for (index, outcome) in &outcomes {
        let name = &scenes[*index].name;
        match outcome {
            Ok((line, apart)) => {
                eprintln!("{line}");
                worst = worst.max(apart.worst());
            }
            Err(error) => {
                eprintln!("FAILED {name}: {error}");
                failures.push(format!("{name}: {error}"));
            }
        }
    }
    eprintln!(
        "{} scenes renumbered on {workers} threads in {:.1} s, the worst {worst:.1e} apart",
        outcomes.len() - failures.len(),
        started.elapsed().as_secs_f64()
    );
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
