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
//!
//! IV-02, identifiers. The same scenes with every curve, span, junction,
//! material, region and probe given another identifier, wide apart and in
//! no order of its list, and the materials, regions, face assignments, volume
//! sources and probes listed backwards, are the same scenes. The background
//! keeps `BACKGROUND_REGION`, which validation requires, and the curves keep
//! their order, which the mesher reads (section 6j measured a scene meshing
//! differently with its curves listed in another order). Each must mesh
//! alike, point for point, and run as the original does to roundoff, its
//! probes read by their new identifiers.
//!
//! IV-01, superposition. A linear scene responds linearly to what drives
//! it. Struck by the pulse with its own sources running, it reads the sum
//! of the pulse with every source's signal at zero and of its sources
//! alone; with the pulse and every signal times -2.5 it reads -2.5 times as
//! much. A scene is linear when every material is: its field laws linear,
//! its loss rates constant, its restoring law none or Klein-Gordon. Drives,
//! gates and switches in time keep a medium linear, and walls with memory,
//! thin gaps, losses and dispersion are linear too. The sources are the
//! point source, the volume sources and every Neumann or Dirichlet side and
//! face, each signal scaled through its offset and amplitude; a scene's
//! three versions must mesh alike.

use funfern_app::document::ProbeId;
use funfern_app::topology_editor::{
    TopologyAcceptance, TopologyDocument, TopologyEditor, TopologyProbeTarget,
};
use funfern_app::topology_examples;
use funfern_app::topology_persistence;
use funfern_app::topology_runtime::{
    PreparedTopology, TopologyProbeCompilation, TopologyProbeStencil, TopologyRuntime,
};
use funfern_core::{
    BACKGROUND_REGION, BoundaryEdge, BoundaryLabel, CanonicalPointStencil,
    CanonicalTemporalWaveState, CanonicalWaveOperator, CanonicalWaveState, CurveId, CurveSpanId,
    FaceAnchor, FaceBoundaryCondition, FieldLaw, MaterialId, MeshTriangle, MeshingOptions,
    OuterBoundaryCondition, Point2, RateLaw, RegionId, RestoringLaw, SpanBehavior, TimeSignal,
    TopologyScene, TopologyVertexId, TriMesh,
};
use std::collections::BTreeMap;
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
    /// middle, where few scenes put anything, `strength` times its height.
    fn struck(prepared: &PreparedTopology, time_step: f64, strength: f64) -> Result<Self, String> {
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
            .map(|point| {
                0.1 * strength * (-(*point - centre).dot(*point - centre) / (width * width)).exp()
            })
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
    run_struck(prepared, time_step, 1.0)
}

/// `prepared` struck at `strength` and run `STEPS` steps at `time_step`.
fn run_struck(prepared: &PreparedTopology, time_step: f64, strength: f64) -> Result<Run, String> {
    let mut run = Run::struck(prepared, time_step, strength)?;
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
fn readings(prepared: &PreparedTopology, run: &Run) -> Result<Vec<(ProbeId, f64, Point2)>, String> {
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
                let (primary, complementary) = read(&stencil)?;
                readings.push((probe.id, primary, complementary));
            }
            TopologyProbeStencil::Segment(points) => {
                let mut sum = (0.0, Point2::default());
                for point in points {
                    let stencil = CanonicalPointStencil::from_quadratic_own(*point, operator)
                        .map_err(|error| error.to_string())?;
                    let (primary, complementary) = read(&stencil)?;
                    sum = (sum.0 + primary, sum.1 + complementary);
                }
                readings.push((probe.id, sum.0, sum.1));
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

/// `document` opened, validated to acceptance and prepared the way the app
/// prepares it, at its own edge, and published: the editor, the runtime that
/// holds it and the generation.
fn prepare(
    document: &TopologyDocument,
) -> Result<(TopologyEditor, TopologyRuntime, Arc<PreparedTopology>), String> {
    let mut editor = TopologyEditor::from_document(document.clone())
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
    let prepared = finish(&mut runtime)?;
    Ok((editor, runtime, prepared))
}

/// How far `other`'s run stands from `control`'s, its primary nodes and
/// complementary samples and probes found through the maps given, and how
/// many probes were read.
fn compare(
    (control, control_run): (&PreparedTopology, &Run),
    (other, other_run): (&PreparedTopology, &Run),
    node: impl Fn(usize) -> usize,
    sample: impl Fn(usize) -> usize,
    probe: impl Fn(ProbeId) -> ProbeId,
) -> Result<(Apart, usize), String> {
    let expected = readings(control, control_run)?;
    let read = readings(other, other_run)?;
    if read.len() != expected.len() {
        return Err(format!(
            "{} probes read against {}",
            read.len(),
            expected.len()
        ));
    }
    let matched = expected
        .iter()
        .map(|(id, ..)| {
            read.iter()
                .find(|(other, ..)| *other == probe(*id))
                .map(|(_, primary, complementary)| (*primary, *complementary))
                .ok_or_else(|| format!("no probe {:?} for {id:?}", probe(*id)))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let apart = Apart {
        time_step: (other.recommended_time_step() / control.recommended_time_step() - 1.0).abs(),
        primary: apart(other_run.primary(), &node, control_run.primary(), scalar),
        complementary: apart(
            other_run.complementary(),
            &sample,
            control_run.complementary(),
            vector,
        ),
        integrated: if control_run.integrated().is_empty() {
            0.0
        } else {
            apart(
                other_run.integrated(),
                &node,
                control_run.integrated(),
                scalar,
            )
        },
        energy: (other_run.energy(other)? / control_run.energy(control)? - 1.0).abs(),
        probe_primary: apart(
            &matched.iter().map(|reading| reading.0).collect::<Vec<_>>(),
            |index| index,
            &expected.iter().map(|reading| reading.1).collect::<Vec<_>>(),
            scalar,
        ),
        probe_complementary: apart(
            &matched.iter().map(|reading| reading.1).collect::<Vec<_>>(),
            |index| index,
            &expected.iter().map(|reading| reading.2).collect::<Vec<_>>(),
            vector,
        ),
    };
    Ok((apart, expected.len()))
}

fn renumbered(scene: &Scene) -> Result<(String, Apart), String> {
    let started = Instant::now();
    let (editor, mut runtime, fresh) = prepare(&scene.document)?;
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
    let (apart, probes) = compare(
        (&control, &control_run),
        (&shuffled, &shuffled_run),
        |node| nodes[node],
        |sample| samples[sample],
        |probe| probe,
    )?;
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
            probes,
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
    over_every_scene("renumbered", renumbered);
}

/// New identifiers for `ids`, given in the order their list holds them,
/// those in `kept` aside: wide apart, none of them small, and zigzagging
/// along the list as it will stand, reversed where `reversed`, so that it
/// is never sorted by them, nor sorted backwards from three on.
fn renamed(ids: impl IntoIterator<Item = u64>, kept: &[u64], reversed: bool) -> BTreeMap<u64, u64> {
    let mut order = Vec::new();
    for id in ids {
        if !kept.contains(&id) && !order.contains(&id) {
            order.push(id);
        }
    }
    if reversed {
        order.reverse();
    }
    let count = order.len();
    order
        .into_iter()
        .enumerate()
        .map(|(position, id)| {
            let rank = if position % 2 == 0 {
                count - 1 - position / 2
            } else {
                position / 2
            };
            (id, 104_729 + 7_919 * rank as u64)
        })
        .collect()
}

/// The identifiers a document's scenes and probes carry, old to new.
struct Renaming {
    curves: BTreeMap<u64, u64>,
    spans: BTreeMap<u64, u64>,
    vertices: BTreeMap<u64, u64>,
    materials: BTreeMap<u64, u64>,
    regions: BTreeMap<u64, u64>,
    probes: BTreeMap<u64, u64>,
}

impl Renaming {
    /// The accepted scene's lists set the order, the draft's adding what
    /// only it holds.
    fn of(document: &TopologyDocument) -> Self {
        let scenes = [&document.model.accepted, &document.model.draft];
        let curves = || scenes.iter().flat_map(|scene| &scene.geometry.curves);
        Self {
            curves: renamed(curves().map(|curve| curve.id.0), &[], false),
            spans: renamed(
                curves().flat_map(|curve| curve.spans.iter().map(|span| span.id.0)),
                &[],
                false,
            ),
            vertices: renamed(
                scenes
                    .iter()
                    .flat_map(|scene| &scene.geometry.vertices)
                    .map(|vertex| vertex.id.0),
                &[],
                false,
            ),
            materials: renamed(
                scenes
                    .iter()
                    .flat_map(|scene| &scene.materials)
                    .map(|material| material.id.0),
                &[],
                true,
            ),
            regions: renamed(
                scenes
                    .iter()
                    .flat_map(|scene| &scene.regions)
                    .map(|region| region.id.0),
                &[BACKGROUND_REGION.0],
                true,
            ),
            probes: renamed(
                document.model.probes.iter().map(|probe| probe.id.0),
                &[],
                true,
            ),
        }
    }

    fn region(&self, region: RegionId) -> RegionId {
        RegionId(self.regions.get(&region.0).copied().unwrap_or(region.0))
    }

    fn probe(&self, probe: ProbeId) -> ProbeId {
        ProbeId(self.probes[&probe.0])
    }

    fn scene(&self, scene: &mut TopologyScene) {
        for curve in &mut scene.geometry.curves {
            curve.id = CurveId(self.curves[&curve.id.0]);
            for span in &mut curve.spans {
                span.id = CurveSpanId(self.spans[&span.id.0]);
            }
            for vertex in curve
                .nodes
                .iter_mut()
                .filter_map(|node| node.vertex.as_mut())
            {
                *vertex = TopologyVertexId(self.vertices[&vertex.0]);
            }
        }
        for vertex in &mut scene.geometry.vertices {
            vertex.id = TopologyVertexId(self.vertices[&vertex.id.0]);
        }
        for material in &mut scene.materials {
            material.id = MaterialId(self.materials[&material.id.0]);
        }
        for region in &mut scene.regions {
            region.id = self.region(region.id);
            region.material = MaterialId(self.materials[&region.material.0]);
        }
        for assignment in &mut scene.face_assignments {
            if let FaceAnchor::Curve { curve, span, .. } = &mut assignment.anchor {
                *curve = CurveId(self.curves[&curve.0]);
                *span = CurveSpanId(self.spans[&span.0]);
            }
            assignment.region = assignment.region.map(|region| self.region(region));
        }
        for source in &mut scene.volume_sources {
            source.region = self.region(source.region);
        }
        scene.materials.reverse();
        scene.regions.reverse();
        scene.face_assignments.reverse();
        scene.volume_sources.reverse();
    }

    fn document(&self, document: &TopologyDocument) -> TopologyDocument {
        let mut document = document.clone();
        let model = &mut document.model;
        self.scene(&mut model.draft);
        self.scene(&mut model.accepted);
        for probe in &mut model.probes {
            probe.id = self.probe(probe.id);
            match &mut probe.target {
                TopologyProbeTarget::Boundary(target) => {
                    target.curve = CurveId(self.curves[&target.curve.0]);
                    for span in &mut target.spans {
                        *span = CurveSpanId(self.spans[&span.0]);
                    }
                }
                TopologyProbeTarget::AreaRegion(region) => *region = self.region(*region),
                TopologyProbeTarget::Point(_)
                | TopologyProbeTarget::Segment { .. }
                | TopologyProbeTarget::AreaDisk { .. } => {}
            }
        }
        model.probes.reverse();
        model.source.region = self.region(model.source.region);
        model.region_names = std::mem::take(&mut model.region_names)
            .into_iter()
            .map(|(region, name)| (self.region(region), name))
            .collect();
        document.readouts.probes = std::mem::take(&mut document.readouts.probes)
            .into_iter()
            .map(|(probe, readout)| (self.probe(probe), readout))
            .collect();
        document
    }
}

/// Where each of `original`'s triangles lies in `other`, when the two are
/// the same mesh, perhaps numbered otherwise: the same triangles point for
/// point, each of the region `renaming` makes of its own, and the same
/// boundary edges, each labelled with the curve and span `renaming` makes
/// of its own. `None` where they differ.
fn same_mesh(original: &TriMesh, other: &TriMesh, renaming: &Renaming) -> Option<Vec<usize>> {
    let point = |mesh: &TriMesh, vertex: usize| {
        let point = mesh.vertices[vertex].point;
        (point.x.to_bits(), point.y.to_bits())
    };
    let corners = |mesh: &TriMesh, vertices: &[usize]| {
        let mut corners = vertices
            .iter()
            .map(|vertex| point(mesh, *vertex))
            .collect::<Vec<_>>();
        corners.sort_unstable();
        corners
    };
    let triangles = other
        .triangles
        .iter()
        .enumerate()
        .map(|(index, triangle)| ((corners(other, &triangle.vertices), triangle.region), index))
        .collect::<BTreeMap<_, _>>();
    let elements = original
        .triangles
        .iter()
        .map(|triangle| {
            triangles
                .get(&(
                    corners(original, &triangle.vertices),
                    renaming.region(triangle.region),
                ))
                .copied()
        })
        .collect::<Option<Vec<_>>>()?;
    let label = |label: BoundaryLabel| match label {
        BoundaryLabel::Curve {
            curve,
            span,
            side,
            separated,
        } => BoundaryLabel::Curve {
            curve: CurveId(renaming.curves[&curve.0]),
            span: CurveSpanId(renaming.spans[&span.0]),
            side,
            separated,
        },
        outer => outer,
    };
    let edges = |mesh: &TriMesh, relabel: &dyn Fn(BoundaryLabel) -> BoundaryLabel| {
        let mut edges = mesh
            .boundary_edges
            .iter()
            .map(|edge| {
                (
                    edge.vertices.map(|vertex| point(mesh, vertex)),
                    edge.parameters.map(f64::to_bits),
                    format!("{:?}", relabel(edge.label)),
                )
            })
            .collect::<Vec<_>>();
        edges.sort_unstable();
        edges
    };
    (original.vertices.len() == other.vertices.len()
        && original.triangles.len() == other.triangles.len()
        && edges(original, &label) == edges(other, &|label| label))
    .then_some(elements)
}

fn relabelled(scene: &Scene) -> Result<(String, Apart), String> {
    let started = Instant::now();
    let renaming = Renaming::of(&scene.document);
    let (_, _, original) = prepare(&scene.document)?;
    let (_, _, other) = prepare(&renaming.document(&scene.document))
        .map_err(|error| format!("relabelled: {error}"))?;
    let elements = same_mesh(&original.mesh, &other.mesh, &renaming)
        .ok_or("the relabelled scene meshes differently")?;
    let renumbered = elements.iter().enumerate().any(|(from, to)| from != *to);
    let (nodes, samples) = correspondence(
        &original.canonical_operator,
        &other.canonical_operator,
        &elements,
    )?;
    let time_step = original.recommended_time_step();
    let (original_run, other_run) = (run(&original, time_step)?, run(&other, time_step)?);
    let (apart, probes) = compare(
        (&original, &original_run),
        (&other, &other_run),
        |node| nodes[node],
        |sample| samples[sample],
        |probe| renaming.probe(probe),
    )?;
    if apart.worst() > ROUNDOFF {
        return Err(format!(
            "the relabelled scene's run stands {apart:?} from the original's"
        ));
    }
    Ok((
        format!(
            "{}: {} vertices alike{}, {} probes read, {}, {:.2} s",
            scene.name,
            original.mesh.vertices.len(),
            if renumbered {
                ", numbered otherwise"
            } else {
                ""
            },
            probes,
            if !renumbered && original_run.bits() == other_run.bits() {
                "the same bits".to_owned()
            } else {
                format!("{:.1e} apart", apart.worst())
            },
            started.elapsed().as_secs_f64()
        ),
        apart,
    ))
}

/// Runs `check` over every scene on all threads, prints each line, and
/// fails with every scene that failed.
fn over_every_scene(what: &str, check: fn(&Scene) -> Result<(String, Apart), String>) {
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
        "{} scenes {what} on {workers} threads in {:.1} s, the worst {worst:.1e} apart",
        outcomes.len() - failures.len(),
        started.elapsed().as_secs_f64()
    );
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn relabelled_identifiers_give_every_scene_the_same_solution() {
    over_every_scene("relabelled", relabelled);
}

/// The factor IV-01's second run takes the pulse and every signal by.
const SCALE: f64 = -2.5;

/// Whether every material of `scene` responds linearly to its field.
fn linear(scene: &TopologyScene) -> bool {
    scene.materials.iter().all(|material| {
        [&material.mass_law, &material.stiffness_law]
            .iter()
            .all(|law| matches!(law.field, FieldLaw::Linear))
            && [&material.electric_loss, &material.magnetic_loss]
                .into_iter()
                .flatten()
                .all(|loss| matches!(loss.law.rate, RateLaw::Constant))
            && matches!(
                material.restoring,
                RestoringLaw::None | RestoringLaw::KleinGordon { .. }
            )
    })
}

/// `signal` times `factor`, through its offset and amplitude.
fn scaled_signal(signal: TimeSignal, factor: f64) -> TimeSignal {
    match signal {
        TimeSignal::Harmonic {
            offset,
            amplitude,
            frequency_hz,
            phase_radians,
        } => TimeSignal::Harmonic {
            offset: factor * offset,
            amplitude: factor * amplitude,
            frequency_hz,
            phase_radians,
        },
        TimeSignal::Pulsed {
            offset,
            amplitude,
            frequency_hz,
            phase_radians,
            envelope,
            start,
            repeat,
        } => TimeSignal::Pulsed {
            offset: factor * offset,
            amplitude: factor * amplitude,
            frequency_hz,
            phase_radians,
            envelope,
            start,
            repeat,
        },
    }
}

/// `document` with every source's signal times `factor`: the point source,
/// the volume sources and every driven side and face, in both scenes.
fn sources_scaled(document: &TopologyDocument, factor: f64) -> TopologyDocument {
    let face = |condition: &mut FaceBoundaryCondition| match condition {
        FaceBoundaryCondition::Neumann { signal } | FaceBoundaryCondition::Dirichlet { signal } => {
            *signal = scaled_signal(*signal, factor)
        }
        _ => {}
    };
    let mut document = document.clone();
    let model = &mut document.model;
    for scene in [&mut model.draft, &mut model.accepted] {
        for source in &mut scene.volume_sources {
            source.signal = scaled_signal(source.signal, factor);
        }
        for side in &mut scene.outer_boundaries.sides {
            match side {
                OuterBoundaryCondition::Neumann { signal }
                | OuterBoundaryCondition::Dirichlet { signal } => {
                    *signal = scaled_signal(*signal, factor)
                }
                _ => {}
            }
        }
        for span in scene
            .geometry
            .curves
            .iter_mut()
            .flat_map(|curve| &mut curve.spans)
        {
            if let SpanBehavior::Separated { left, right, .. } = &mut span.behavior {
                face(left);
                face(right);
            }
        }
    }
    model.source.signal = scaled_signal(model.source.signal, factor);
    document
}

/// `‖Σ cᵢ xᵢ‖ / maxᵢ ‖cᵢ xᵢ‖` for each quantity the runs carry: zero where
/// the linear relation the terms state holds.
fn residual(terms: &[(f64, &PreparedTopology, &Run)]) -> Result<Apart, String> {
    fn combine<T>(terms: &[(f64, &[T])], square: impl Fn(T) -> f64) -> f64
    where
        T: Copy + std::ops::Add<Output = T> + std::ops::Mul<f64, Output = T>,
    {
        let largest = terms
            .iter()
            .map(|(factor, values)| values.iter().map(|value| square(*value * *factor)).sum())
            .fold(0.0, f64::max);
        let sum = (0..terms[0].1.len())
            .map(|index| {
                square(
                    terms
                        .iter()
                        .map(|(factor, values)| values[index] * *factor)
                        .reduce(|a, b| a + b)
                        .unwrap(),
                )
            })
            .sum::<f64>();
        (sum / largest.max(f64::MIN_POSITIVE)).sqrt()
    }
    let scalar = |values: Vec<(f64, &[f64])>| combine(&values, |value| value * value);
    let vector = |values: Vec<(f64, &[Point2])>| combine(&values, |value| value.dot(value));
    let readings = terms
        .iter()
        .map(|(_, prepared, run)| readings(prepared, run))
        .collect::<Result<Vec<_>, _>>()?;
    let primaries = readings
        .iter()
        .map(|read| read.iter().map(|reading| reading.1).collect::<Vec<_>>())
        .collect::<Vec<_>>();
    let complementaries = readings
        .iter()
        .map(|read| read.iter().map(|reading| reading.2).collect::<Vec<_>>())
        .collect::<Vec<_>>();
    let factors = || terms.iter().map(|(factor, ..)| *factor);
    Ok(Apart {
        primary: scalar(
            factors()
                .zip(terms.iter().map(|(.., run)| run.primary()))
                .collect(),
        ),
        complementary: vector(
            factors()
                .zip(terms.iter().map(|(.., run)| run.complementary()))
                .collect(),
        ),
        integrated: scalar(
            factors()
                .zip(terms.iter().map(|(.., run)| run.integrated()))
                .collect(),
        ),
        probe_primary: scalar(factors().zip(primaries.iter().map(Vec::as_slice)).collect()),
        probe_complementary: vector(
            factors()
                .zip(complementaries.iter().map(Vec::as_slice))
                .collect(),
        ),
        ..Apart::default()
    })
}

fn superposed(scene: &Scene) -> Result<(String, Apart), String> {
    let started = Instant::now();
    if !linear(&scene.document.model.accepted) {
        return Ok((
            format!("{}: not linear, left out", scene.name),
            Apart::default(),
        ));
    }
    let (_, _, whole) = prepare(&scene.document)?;
    let (_, _, quiet) = prepare(&sources_scaled(&scene.document, 0.0))
        .map_err(|error| format!("its sources at zero: {error}"))?;
    let (_, _, negated) = prepare(&sources_scaled(&scene.document, SCALE))
        .map_err(|error| format!("its sources scaled: {error}"))?;
    let alike = |other: &PreparedTopology| {
        other.mesh.vertices == whole.mesh.vertices
            && other.mesh.triangles == whole.mesh.triangles
            && other.mesh.boundary_edges == whole.mesh.boundary_edges
    };
    if !alike(&quiet) || !alike(&negated) {
        return Err("its sources' strength changed its mesh".into());
    }
    let time_step = whole.recommended_time_step();
    let both = run_struck(&whole, time_step, 1.0)?;
    let pulse = run_struck(&quiet, time_step, 1.0)?;
    let sources = run_struck(&whole, time_step, 0.0)?;
    let scaled = run_struck(&negated, time_step, SCALE)?;
    let sum = residual(&[
        (1.0, &whole, &both),
        (-1.0, &quiet, &pulse),
        (-1.0, &whole, &sources),
    ])?;
    let scale = residual(&[(1.0, &negated, &scaled), (-SCALE, &whole, &both)])?;
    if sum.worst().max(scale.worst()) > ROUNDOFF {
        return Err(format!(
            "the pulse and the sources do not add: {sum:?}; scaled: {scale:?}"
        ));
    }
    let driven = sources.primary().iter().any(|value| *value != 0.0);
    Ok((
        format!(
            "{}: {}, superposed {:.1e}, scaled {:.1e}, {:.2} s",
            scene.name,
            if driven { "driven" } else { "no sources" },
            sum.worst(),
            scale.worst(),
            started.elapsed().as_secs_f64()
        ),
        Apart {
            primary: sum.worst().max(scale.worst()),
            ..Apart::default()
        },
    ))
}

#[test]
fn every_linear_scene_adds_and_scales_its_responses() {
    // 79 of the 91 on 2026-10-31: a test that found every scene nonlinear
    // would pass on nothing.
    let linear = scenes()
        .iter()
        .filter(|scene| linear(&scene.document.model.accepted))
        .count();
    assert!(linear >= 60, "only {linear} scenes are linear");
    over_every_scene("superposed", superposed);
}
