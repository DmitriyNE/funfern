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
    ClosedCurvePurpose, OpenCurvePurpose, TopologyAcceptance, TopologyAttachment, TopologyDocument,
    TopologyEditor, TopologyProbeTarget,
};
use funfern_app::topology_examples;
use funfern_app::topology_persistence;
use funfern_app::topology_runtime::{
    PreparedTopology, TopologyProbeCompilation, TopologyProbeStencil, TopologyRuntime,
};
use funfern_core::{
    BACKGROUND_REGION, BoundaryEdge, BoundaryLabel, CanonicalPointStencil,
    CanonicalTemporalWaveState, CanonicalWaveOperator, CanonicalWaveState, CurveId, CurveSpanId,
    CurveSpline, CurveTraceSide, DomainRect, ElectromagneticPolarization, FaceAnchor,
    FaceBoundaryCondition, FieldLaw, MaterialFrame, MaterialId, MeshTriangle, MeshingOptions,
    OpenCubicSpline, OuterBoundaryCondition, OuterSide, PeriodicCubicSpline, PhysicsModel, Point2,
    QuadraticPointStencil, RateLaw, RegionId, RestoringLaw, SpanBehavior, SymmetricTensor2,
    TimeSignal, TopologyCurve, TopologyScene, TopologyVertexId, TopologyVertexLocation, TriMesh,
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

/// A smooth pulse of the primary field: a Gaussian `height` high and
/// `width` wide about `centre`.
#[derive(Clone, Copy, Debug)]
struct Pulse {
    centre: Point2,
    width: f64,
    height: f64,
}

impl Pulse {
    /// Centred off the domain's middle, where few scenes put anything,
    /// `strength` times its usual height.
    fn off_centre(domain: DomainRect, strength: f64) -> Self {
        Self {
            centre: Point2::new(
                domain.min_x + 0.37 * domain.width(),
                domain.min_y + 0.41 * domain.height(),
            ),
            width: 0.15 * domain.width().min(domain.height()),
            height: 0.1 * strength,
        }
    }

    fn at(self, point: Point2) -> f64 {
        let offset = point - self.centre;
        self.height * (-offset.dot(offset) / (self.width * self.width)).exp()
    }
}

impl Run {
    /// At rest, then struck by the pulse off the domain's middle,
    /// `strength` times its usual height.
    fn struck(prepared: &PreparedTopology, time_step: f64, strength: f64) -> Result<Self, String> {
        let domain = prepared.bundle.authored.geometry.domain;
        Self::struck_by(prepared, time_step, Pulse::off_centre(domain, strength))
    }

    /// At rest, then struck by `pulse`.
    fn struck_by(
        prepared: &PreparedTopology,
        time_step: f64,
        pulse: Pulse,
    ) -> Result<Self, String> {
        let increment = prepared
            .canonical_operator
            .node_points()
            .iter()
            .map(|point| pulse.at(*point))
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
    prepare_at(document, document.presentation.mesh_edge)
}

/// [`prepare`] at the mesh edge `edge`.
fn prepare_at(
    document: &TopologyDocument,
    edge: f64,
) -> Result<(TopologyEditor, TopologyRuntime, Arc<PreparedTopology>), String> {
    let mut editor = TopologyEditor::from_document(document.clone())
        .map_err(|error| format!("does not load: {error}"))?;
    settle(&mut editor);
    if let TopologyAcceptance::Invalid(issue) = &editor.acceptance {
        return Err(format!("refused at acceptance: {issue}"));
    }
    let domain = editor.document.model.accepted.geometry.domain;
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

/// `work` done on every item on all threads, the results in the items' order.
fn in_parallel<T: Sync, R: Send>(items: &[T], work: impl Fn(&T) -> R + Sync) -> Vec<R> {
    let workers = std::thread::available_parallelism()
        .map(|count| count.get())
        .unwrap_or(4)
        .min(items.len().max(1));
    let work = &work;
    std::thread::scope(|scope| {
        let handles = (0..workers)
            .map(|worker| {
                scope.spawn(move || {
                    items
                        .iter()
                        .enumerate()
                        .skip(worker)
                        .step_by(workers)
                        .map(|(index, item)| (index, work(item)))
                        .collect::<Vec<_>>()
                })
            })
            .collect::<Vec<_>>();
        let mut outcomes = handles
            .into_iter()
            .flat_map(|handle| handle.join().expect("a worker"))
            .collect::<Vec<_>>();
        outcomes.sort_by_key(|(index, _)| *index);
        outcomes.into_iter().map(|(_, outcome)| outcome).collect()
    })
}

/// Runs `check` over every scene on all threads, prints each line, and
/// fails with every scene that failed.
fn over_every_scene(what: &str, check: fn(&Scene) -> Result<(String, Apart), String>) {
    let scenes = scenes();
    let started = Instant::now();
    let outcomes = in_parallel(&scenes, check)
        .into_iter()
        .enumerate()
        .collect::<Vec<_>>();
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
        "{} scenes {what} in {:.1} s, the worst {worst:.1e} apart",
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

/// A rigid motion of the plane.
#[derive(Clone, Copy, Debug)]
enum Motion {
    /// Every point moved by the vector.
    Shift(Point2),
    /// A quarter turn counterclockwise about the origin.
    QuarterTurn,
    /// The mirror image across the vertical axis, `x → -x`. A frame's axes
    /// cannot be mirrored, so its angle goes to `turn - angle`: a turn of 0
    /// reverses the frame's own `x`, of π its own `y`, and a formula even in
    /// that coordinate comes through.
    Mirror { turn: f64 },
    /// The domain's contents turned by `angle` about `centre`, the domain
    /// itself and its sides left where they are: a turn the outer walls do
    /// not share, so a run must end before its waves reach them, and no
    /// curve may meet a side.
    Turn { centre: Point2, angle: f64 },
}

impl Motion {
    fn linear(self, vector: Point2) -> Point2 {
        match self {
            Motion::Shift(_) => vector,
            Motion::QuarterTurn => Point2::new(-vector.y, vector.x),
            Motion::Mirror { .. } => Point2::new(-vector.x, vector.y),
            Motion::Turn { angle, .. } => {
                let (sin, cos) = angle.sin_cos();
                Point2::new(
                    cos * vector.x - sin * vector.y,
                    sin * vector.x + cos * vector.y,
                )
            }
        }
    }

    fn point(self, point: Point2) -> Point2 {
        match self {
            Motion::Shift(shift) => point + shift,
            Motion::Turn { centre, .. } => centre + self.linear(point - centre),
            _ => self.linear(point),
        }
    }

    fn reflects(self) -> bool {
        matches!(self, Motion::Mirror { .. })
    }

    /// The complementary field's image. The field is a gradient turned a
    /// quarter, `R∇ψ`, and a reflection turns it the other way: `-M b`.
    fn complementary(self, vector: Point2) -> Point2 {
        let image = self.linear(vector);
        if self.reflects() { image * -1.0 } else { image }
    }

    /// A tensor's image, `L A Lᵀ`.
    fn tensor(self, tensor: SymmetricTensor2) -> SymmetricTensor2 {
        let SymmetricTensor2 { xx, xy, yy } = tensor;
        match self {
            Motion::Shift(_) => tensor,
            Motion::QuarterTurn => SymmetricTensor2 {
                xx: yy,
                xy: -xy,
                yy: xx,
            },
            Motion::Mirror { .. } => SymmetricTensor2 { xx, xy: -xy, yy },
            Motion::Turn { angle, .. } => {
                let (sin, cos) = angle.sin_cos();
                SymmetricTensor2 {
                    xx: cos * cos * xx - 2.0 * cos * sin * xy + sin * sin * yy,
                    xy: cos * sin * (xx - yy) + (cos * cos - sin * sin) * xy,
                    yy: sin * sin * xx + 2.0 * sin * cos * xy + cos * cos * yy,
                }
            }
        }
    }

    fn domain(self, domain: DomainRect) -> DomainRect {
        if let Motion::Turn { .. } = self {
            return domain;
        }
        let [a, b] = [
            Point2::new(domain.min_x, domain.min_y),
            Point2::new(domain.max_x, domain.max_y),
        ]
        .map(|corner| self.point(corner));
        DomainRect {
            min_x: a.x.min(b.x),
            max_x: a.x.max(b.x),
            min_y: a.y.min(b.y),
            max_y: a.y.max(b.y),
        }
    }

    /// An outer side's image, and a fraction along it, which runs
    /// counterclockwise round the domain.
    fn side(self, side: OuterSide, fraction: f64) -> (OuterSide, f64) {
        match self {
            Motion::Shift(_) | Motion::Turn { .. } => (side, fraction),
            Motion::QuarterTurn => (
                match side {
                    OuterSide::Bottom => OuterSide::Right,
                    OuterSide::Right => OuterSide::Top,
                    OuterSide::Top => OuterSide::Left,
                    OuterSide::Left => OuterSide::Bottom,
                },
                fraction,
            ),
            Motion::Mirror { .. } => (
                match side {
                    OuterSide::Left => OuterSide::Right,
                    OuterSide::Right => OuterSide::Left,
                    other => other,
                },
                1.0 - fraction,
            ),
        }
    }

    fn frame(self, frame: MaterialFrame) -> MaterialFrame {
        MaterialFrame {
            origin: self.point(frame.origin),
            angle_radians: match self {
                Motion::Shift(_) => frame.angle_radians,
                Motion::QuarterTurn => frame.angle_radians + std::f64::consts::FRAC_PI_2,
                Motion::Mirror { turn } => turn - frame.angle_radians,
                Motion::Turn { angle, .. } => frame.angle_radians + angle,
            },
            ..frame
        }
    }

    /// The side of a curve a face lies on: a reflection reverses every
    /// curve, so its left becomes its right.
    fn trace_side(self, side: CurveTraceSide) -> CurveTraceSide {
        if self.reflects() {
            side.opposite()
        } else {
            side
        }
    }

    /// `document` carried through the motion: its domain, curves, junctions,
    /// outer sides, face anchors, frames, source and probes. A reflection
    /// swaps every separated span's two laws, its left now its right.
    fn document(self, document: &TopologyDocument) -> Result<TopologyDocument, String> {
        let mut document = document.clone();
        let model = &mut document.model;
        for scene in [&mut model.draft, &mut model.accepted] {
            let geometry = &mut scene.geometry;
            geometry.domain = self.domain(geometry.domain);
            for curve in &mut geometry.curves {
                let controls = match &curve.spline {
                    CurveSpline::Closed(spline) => spline.controls().to_vec(),
                    CurveSpline::Open(spline) => spline.controls().to_vec(),
                };
                for (index, control) in controls.into_iter().enumerate() {
                    let point = self.point(control);
                    match &mut curve.spline {
                        CurveSpline::Closed(spline) => spline.set_control(index, point),
                        CurveSpline::Open(spline) => spline.set_control(index, point),
                    }
                    .map_err(|error| format!("{error:?}"))?;
                }
                if self.reflects() {
                    for span in &mut curve.spans {
                        if let SpanBehavior::Separated {
                            left,
                            right,
                            coupling,
                        } = span.behavior
                        {
                            span.behavior = SpanBehavior::Separated {
                                left: right,
                                right: left,
                                coupling,
                            };
                        }
                    }
                }
            }
            for vertex in &mut geometry.vertices {
                vertex.location = match vertex.location {
                    TopologyVertexLocation::Free(point) => {
                        TopologyVertexLocation::Free(self.point(point))
                    }
                    TopologyVertexLocation::Interior(point) => {
                        TopologyVertexLocation::Interior(self.point(point))
                    }
                    TopologyVertexLocation::Outer { .. } if matches!(self, Motion::Turn { .. }) => {
                        return Err(NOT_TURNABLE.into());
                    }
                    TopologyVertexLocation::Outer { side, fraction } => {
                        let (side, fraction) = self.side(side, fraction);
                        TopologyVertexLocation::Outer { side, fraction }
                    }
                };
            }
            geometry
                .synchronize_vertices()
                .map_err(|issue| issue.to_string())?;
            let sides = scene.outer_boundaries.sides;
            for side in OuterSide::ALL {
                scene.outer_boundaries.sides[self.side(side, 0.5).0.index()] = sides[side.index()];
            }
            for region in &mut scene.regions {
                region.frame = self.frame(region.frame);
            }
            for assignment in &mut scene.face_assignments {
                match &mut assignment.anchor {
                    FaceAnchor::Outer { side, fraction } => {
                        (*side, *fraction) = self.side(*side, *fraction);
                    }
                    FaceAnchor::Curve { side, .. } => *side = self.trace_side(*side),
                }
            }
        }
        model.source.position = self.point(model.source.position);
        for probe in &mut model.probes {
            match &mut probe.target {
                TopologyProbeTarget::Point(point) => *point = self.point(*point),
                TopologyProbeTarget::Segment { start, end, .. } => {
                    (*start, *end) = (self.point(*start), self.point(*end));
                }
                TopologyProbeTarget::AreaDisk { center, .. } => *center = self.point(*center),
                TopologyProbeTarget::Boundary(target) => target.side = self.trace_side(target.side),
                TopologyProbeTarget::AreaRegion(_) => {}
            }
        }
        Ok(document)
    }
}

/// The stencil of a point probe at `point`, `None` where no element takes
/// it.
fn stencil_at(prepared: &PreparedTopology, point: Point2) -> Option<QuadraticPointStencil> {
    QuadraticPointStencil::build_topology(
        &prepared.mesh,
        &prepared.operator,
        &prepared.bundle.plan,
        prepared.fixed_model(),
        point,
    )
    .ok()
}

/// What a point probe reads at `point`: the field and the complementary
/// field. `None` where no element takes the point.
fn probe_at(prepared: &PreparedTopology, run: &Run, point: Point2) -> Option<(f64, Point2)> {
    let reading = CanonicalPointStencil::from_quadratic(
        stencil_at(prepared, point)?,
        &prepared.canonical_operator,
    )
    .ok()?
    .sample(run.primary(), run.primary(), run.complementary())
    .ok()?;
    Some((reading.primary, reading.complementary))
}

/// What `region`'s material is at `point` as authored, read in the region's
/// frame as the solver reads it: its density, stiffness and axis ratio, its
/// loss rates, its restoring law's scale, and each drive's multiplier at
/// three instants; and its stiffness as a tensor in world axes, the frame's
/// turn on it, built here from the principal values.
fn laws_at(
    scene: &TopologyScene,
    region: RegionId,
    point: Point2,
) -> Result<(Vec<f64>, SymmetricTensor2), String> {
    let region = scene.region(region).ok_or("no such region")?;
    let material = scene.material(region.material).ok_or("no such material")?;
    let coordinates = region.frame.coordinates(point);
    let parameters = &material.parameters;
    let field = |field: &funfern_core::ScalarField| {
        field
            .evaluate(coordinates, parameters)
            .map_err(|error| format!("{error:?}"))
    };
    let mut values = vec![
        field(&material.mass_density)?,
        field(&material.stiffness)?,
        field(&material.axis_ratio)?,
    ];
    let mut drives = vec![&material.mass_law.drive, &material.stiffness_law.drive];
    for loss in [&material.electric_loss, &material.magnetic_loss]
        .into_iter()
        .flatten()
    {
        values.push(field(&loss.base_rate)?);
        drives.push(&loss.law.drive);
    }
    values.push(match &material.restoring {
        RestoringLaw::None => 0.0,
        RestoringLaw::KleinGordon { omega0 } | RestoringLaw::SineGordon { omega0 } => {
            field(omega0)?
        }
        RestoringLaw::Phi4 { lambda, .. } => field(lambda)?,
    });
    for drive in drives {
        let drive = drive
            .evaluate(parameters)
            .map_err(|error| format!("{error:?}"))?;
        for time in [0.0, 0.37, 1.1] {
            values.push(drive.multiplier(time, coordinates));
        }
    }
    // Stiffness times the ratio along the frame's first axis and over it
    // along the second, `R diag Rᵀ`.
    let (stiffness, ratio) = (values[1], values[2]);
    let (x, y) = (stiffness * ratio, stiffness / ratio);
    let (sin, cos) = region.frame.angle_radians.sin_cos();
    let tensor = SymmetricTensor2 {
        xx: x * cos * cos + y * sin * sin,
        xy: (x - y) * sin * cos,
        yy: x * sin * sin + y * cos * cos,
    };
    Ok((values, tensor))
}

/// How [`carried`] says the other scene is not the original carried
/// through as authored: the motion's fault or the rewrite's, or the editor's
/// where it made the other scene, never the solver's.
const NOT_CARRIED: &str = "the other scene's material";

/// How [`Motion::document`] refuses to turn a scene whose curves meet the
/// domain's sides, which stay where they are.
const NOT_TURNABLE: &str = "a curve meets a side the turn leaves";

/// A grid of points over `domain`, those within `margin` of a curve of
/// `mesh` left out: the two meshes approximate a curve each its own way, and
/// a point beside one may lie on its two sides in the two.
fn common_points(domain: DomainRect, mesh: &TriMesh, margin: f64) -> Vec<Point2> {
    let segments = mesh
        .boundary_edges
        .iter()
        .filter(|edge| matches!(edge.label, BoundaryLabel::Curve { .. }))
        .map(|edge| edge.vertices.map(|vertex| mesh.vertices[vertex].point))
        .collect::<Vec<_>>();
    let distance = |point: Point2, [a, b]: [Point2; 2]| {
        let along = b - a;
        let t = ((point - a).dot(along) / along.dot(along)).clamp(0.0, 1.0);
        (a + along * t - point).norm()
    };
    let count = 24;
    (0..count * count)
        .map(|index| {
            Point2::new(
                domain.min_x + domain.width() * ((index % count) as f64 + 0.5) / count as f64,
                domain.min_y + domain.height() * ((index / count) as f64 + 0.5) / count as f64,
            )
        })
        .filter(|point| {
            segments
                .iter()
                .all(|segment| distance(*point, *segment) > margin)
        })
        .collect()
}

/// Whether `other` is `original` carried through `motion` as authored: at
/// every one of `points` both meshes take, the same material read in each
/// region's frame, its stiffness tensor carried through. A scene that is not
/// is the motion's fault or the rewrite's, or the editor's where it made the
/// other scene, never the solver's, and fails apart.
fn carried(
    original: &PreparedTopology,
    other: &PreparedTopology,
    motion: Motion,
    points: &[Point2],
) -> Result<(), String> {
    let close = |a: f64, b: f64| (a - b).abs() <= 1.0e-12 * a.abs().max(b.abs()).max(1.0);
    for point in points {
        let (Some(here), Some(there)) = (
            stencil_at(original, *point),
            stencil_at(other, motion.point(*point)),
        ) else {
            continue;
        };
        let ((laws_here, tensor_here), (laws_there, tensor_there)) = (
            laws_at(&original.bundle.authored, here.region, *point)?,
            laws_at(&other.bundle.authored, there.region, motion.point(*point))?,
        );
        let tensor = motion.tensor(tensor_here);
        if laws_here.len() != laws_there.len()
            || laws_here
                .iter()
                .zip(&laws_there)
                .any(|(a, b)| !close(*a, *b))
            || !close(tensor.xx, tensor_there.xx)
            || !close(tensor.xy, tensor_there.xy)
            || !close(tensor.yy, tensor_there.yy)
        {
            return Err(format!(
                "{NOT_CARRIED} at {point:?} is {:?}, not {:?}",
                (laws_there, tensor_there),
                (laws_here, tensor)
            ));
        }
    }
    Ok(())
}

/// How far `other` reads from `original` carried through `motion`, at the
/// points both take, relative to the original: the field, the complementary
/// field, and how many points.
fn moved_apart(
    (original, original_run): (&PreparedTopology, &Run),
    (other, other_run): (&PreparedTopology, &Run),
    motion: Motion,
    points: &[Point2],
) -> (f64, f64, usize) {
    let (mut field, mut field_norm, mut complementary, mut complementary_norm, mut used) =
        (0.0, 0.0, 0.0, 0.0, 0);
    for point in points {
        let (Some(here), Some(there)) = (
            probe_at(original, original_run, *point),
            probe_at(other, other_run, motion.point(*point)),
        ) else {
            continue;
        };
        field += (there.0 - here.0).powi(2);
        field_norm += here.0 * here.0;
        let expected = motion.complementary(here.1);
        complementary += (there.1 - expected).dot(there.1 - expected);
        complementary_norm += expected.dot(expected);
        used += 1;
    }
    (
        (field / field_norm.max(f64::MIN_POSITIVE)).sqrt(),
        (complementary / complementary_norm.max(f64::MIN_POSITIVE)).sqrt(),
        used,
    )
}

/// `prepared` struck by `pulse` and run to `duration` in whole steps no
/// longer than `time_step`.
fn run_for(
    prepared: &PreparedTopology,
    time_step: f64,
    duration: f64,
    pulse: Pulse,
) -> Result<Run, String> {
    let steps = (duration / time_step).ceil();
    let mut run = Run::struck_by(prepared, duration / steps, pulse)?;
    for step in 0..steps as u64 {
        run.step(prepared)
            .map_err(|error| format!("step {step}: {error}"))?;
    }
    Ok(run)
}

/// Per level, a case's field and complementary field read against its
/// original, the original's own move from the level coarser in each, and
/// the edge.
type Levels = Vec<[f64; 5]>;

/// One judged case: its label, its levels or why it did not run, and how
/// long it took.
type Judged = (String, Result<Levels, String>, f64);

/// The levels IV-04 runs at, as multiples of a scene's own edge.
const LEVELS: [f64; 3] = [2.0, 1.0, 0.5];

/// One scene carried through `motion` at each of [`LEVELS`]: per level the
/// field's and the complementary field's distance from the original carried
/// through, and the original's own distance from the next finer level.
fn moved_levels(scene: &Scene, motion: Motion) -> Result<Levels, String> {
    levels_against(&scene.document, &motion.document(&scene.document)?, motion)
}

/// `document` and `other_document`, the original carried through `motion`,
/// at each of [`LEVELS`]: as [`moved_levels`] reads them.
fn levels_against(
    document: &TopologyDocument,
    other_document: &TopologyDocument,
    motion: Motion,
) -> Result<Levels, String> {
    let edge = document.presentation.mesh_edge;
    let domain = document.model.accepted.geometry.domain;
    let pulse = Pulse::off_centre(domain, 1.0);
    let moved_pulse = Pulse {
        centre: motion.point(pulse.centre),
        ..pulse
    };
    let mut duration = 0.0;
    let (mut points, mut law_points) = (Vec::new(), Vec::new());
    let mut levels = Vec::new();
    let mut previous: Option<(Arc<PreparedTopology>, Run)> = None;
    for factor in LEVELS {
        let (_, _, original) = prepare_at(document, edge * factor)?;
        let (_, _, other) =
            prepare_at(other_document, edge * factor).map_err(|error| format!("moved: {error}"))?;
        if previous.is_none() {
            duration = 64.0 * original.recommended_time_step();
            points = common_points(domain, &original.mesh, edge * LEVELS[0]);
            // Past where two meshes' curves can part: the tolerance a curve
            // is meshed to is at most 5e-4.
            law_points = common_points(domain, &original.mesh, 1.0e-3);
        }
        carried(&original, &other, motion, &law_points)?;
        let time_step = original
            .recommended_time_step()
            .min(other.recommended_time_step());
        let original_run = run_for(&original, time_step, duration, pulse)?;
        let other_run = run_for(&other, time_step, duration, moved_pulse)?;
        let (field, complementary, _) = moved_apart(
            (&original, &original_run),
            (&other, &other_run),
            motion,
            &points,
        );
        let own = match &previous {
            Some((coarser, coarser_run)) => moved_apart(
                (coarser, coarser_run),
                (&original, &original_run),
                Motion::Shift(Point2::default()),
                &points,
            ),
            None => (f64::NAN, f64::NAN, 0),
        };
        levels.push([field, complementary, own.0, own.1, edge * factor]);
        previous = Some((original, original_run));
    }
    Ok(levels)
}

/// How close a moved scene must read to the original carried through, at
/// each of the two finer [`LEVELS`], in its field and its complementary
/// field: within the original's own distance from the level coarser, what
/// one halving of the mesh moves it. A motion the solver does not carry
/// through reads apart by its own error, which no refinement takes away. On
/// 2026-10-31 the study below read 0.56 at worst over the gallery's 39
/// linear scenes shifted, quarter turned and mirrored, and 0.81 for the
/// scenes the turn by 37° applies to; a mirror whose frames reversed the
/// travelling modulations read 3 to 85.
const MOVED: f64 = 1.0;

/// The turn by 37° about the domain's centre, which no frame is tested at.
fn turned(scene: &Scene) -> Motion {
    let domain = scene.document.model.accepted.geometry.domain;
    Motion::Turn {
        centre: Point2::new(
            0.5 * (domain.min_x + domain.max_x),
            0.5 * (domain.min_y + domain.max_y),
        ),
        angle: 37.0_f64.to_radians(),
    }
}

const SHIFT: Motion = Motion::Shift(Point2 {
    x: 0.3137,
    y: -0.2219,
});

const MIRROR: Motion = Motion::Mirror {
    turn: std::f64::consts::PI,
};

/// The gallery scenes whose contents a turn may carry: every curve clear of
/// the sides, and a run that ends before its waves reach them.
const TURNABLE: [&str; 10] = [
    "Anisotropic crystal",
    "Acoustic whispering gallery",
    "Dielectric whispering gallery",
    "Plasma whispering gallery",
    "Luneburg lens",
    "Maxwell's fisheye",
    "Drum modes",
    "Struck drum",
    "Pumped drum",
    "Ellipse flash",
];

fn gallery(name: &str) -> Scene {
    let example = topology_examples::catalog()
        .iter()
        .find(|example| example.name == name)
        .unwrap_or_else(|| panic!("no gallery scene {name}"));
    Scene {
        name: example.name.to_owned(),
        document: example.document.clone(),
    }
}

/// The double slit with its baffle outgoing on its right and reflecting on
/// its left, so that a mirror must swap the two to read alike.
fn one_sided_double_slit() -> Scene {
    let mut editor = TopologyEditor::from_document(gallery("Double slit").document).unwrap();
    settle(&mut editor);
    let spans = editor
        .document
        .model
        .draft
        .geometry
        .curves
        .iter()
        .flat_map(|curve| &curve.spans)
        .filter(|span| matches!(span.behavior, SpanBehavior::Separated { .. }))
        .map(|span| span.id)
        .collect::<std::collections::BTreeSet<_>>();
    assert!(!spans.is_empty(), "the double slit has a baffle");
    editor
        .set_span_face_condition(
            &spans,
            CurveTraceSide::Right,
            FaceBoundaryCondition::SecondOrderOutgoing,
        )
        .unwrap();
    settle(&mut editor);
    assert_eq!(editor.acceptance, TopologyAcceptance::Valid);
    Scene {
        name: "Double slit, its baffle outgoing on one side".into(),
        document: editor.document.clone(),
    }
}

/// IV-04's cases: each scene and the motions it is carried through. The
/// gate's set covers frames, a one-sided baffle, interfaces and curved walls
/// in each skin, a travelling modulation, volume sources, a driven side and
/// a lossy medium; `FUNFERN_INVARIANTS_ALL=1`, nightly, carries every linear
/// gallery scene.
fn moved_cases() -> Vec<(Scene, Motion)> {
    if std::env::var_os("FUNFERN_INVARIANTS_ALL").is_some() {
        return topology_examples::catalog()
            .iter()
            .filter(|example| linear(&example.document.model.accepted))
            .flat_map(|example| {
                let scene = gallery(example.name);
                let mut motions = vec![SHIFT, Motion::QuarterTurn, MIRROR];
                if TURNABLE.contains(&example.name) {
                    motions.push(turned(&scene));
                }
                motions.into_iter().map(move |motion| {
                    (
                        Scene {
                            name: scene.name.clone(),
                            document: scene.document.clone(),
                        },
                        motion,
                    )
                })
            })
            .chain(
                [SHIFT, Motion::QuarterTurn, MIRROR]
                    .map(|motion| (one_sided_double_slit(), motion)),
            )
            .collect();
    }
    let crystal = gallery("Anisotropic crystal");
    let gallery_ring = gallery("Acoustic whispering gallery");
    let mut cases = vec![
        (gallery("Anisotropic crystal"), turned(&crystal)),
        (
            gallery("Acoustic whispering gallery"),
            turned(&gallery_ring),
        ),
    ];
    for (scene, motions) in [
        (crystal, vec![SHIFT, Motion::QuarterTurn, MIRROR]),
        (
            one_sided_double_slit(),
            vec![SHIFT, Motion::QuarterTurn, MIRROR],
        ),
        (gallery("Brewster angle"), vec![Motion::QuarterTurn, MIRROR]),
        (gallery_ring, vec![MIRROR]),
        (gallery("Travelling modulation"), vec![MIRROR]),
        (gallery("Phased array"), vec![Motion::QuarterTurn]),
        (gallery("Echo comb"), vec![MIRROR]),
        (gallery("Skin depth"), vec![SHIFT]),
    ] {
        for motion in motions {
            cases.push((
                Scene {
                    name: scene.name.clone(),
                    document: scene.document.clone(),
                },
                motion,
            ));
        }
    }
    cases
}

/// `scene` carried through `motion` at [`LEVELS`], and the motion it took: a
/// mirror whose turn of the frames the scene's laws do not come through, a
/// travelling modulation along a frame's own `x` reversed by it, takes the
/// other turn.
fn carry(scene: &Scene, motion: Motion) -> Result<(Motion, Levels), String> {
    match (moved_levels(scene, motion), motion) {
        (Err(error), Motion::Mirror { turn }) if error.starts_with(NOT_CARRIED) && turn != 0.0 => {
            let other = Motion::Mirror { turn: 0.0 };
            moved_levels(scene, other).map(|levels| (other, levels))
        }
        (result, _) => result.map(|levels| (motion, levels)),
    }
}

/// Judges cases read at [`LEVELS`], each labelled, against [`MOVED`]:
/// prints each, and fails with every case that stands a halving's move or
/// more from its original, or failed to run.
fn within_a_halving(what: &str, outcomes: Vec<Judged>) {
    let mut failures = Vec::new();
    let mut worst: f64 = 0.0;
    let count = outcomes.len();
    for (label, outcome, seconds) in outcomes {
        match outcome {
            Ok(levels) => {
                let ratios = levels[1..]
                    .iter()
                    .map(|[field, complementary, own, own_complementary, _]| {
                        (field / own).max(complementary / own_complementary)
                    })
                    .collect::<Vec<_>>();
                let largest = ratios.iter().copied().fold(0.0, f64::max);
                worst = worst.max(largest);
                let line = format!(
                    "{label}: {} ({seconds:.1} s)",
                    levels
                        .iter()
                        .map(|[field, complementary, own, own_complementary, edge]| format!(
                            "edge {edge:.3} u {field:.1e} of {own:.1e}, b {complementary:.1e} of {own_complementary:.1e}"
                        ))
                        .collect::<Vec<_>>()
                        .join("; ")
                );
                eprintln!("{line}");
                // `f64::max` drops a NaN, so each ratio is judged itself.
                if ratios.iter().any(|ratio| ratio.is_nan() || *ratio > MOVED) {
                    failures.push(format!("{line}: {largest:.2} of a halving's move"));
                }
            }
            Err(error) => {
                eprintln!("FAILED {label}: {error}");
                failures.push(format!("{label}: {error}"));
            }
        }
    }
    eprintln!("{count} cases {what}, the worst {worst:.2} of a halving's move");
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn a_moved_scene_reads_as_the_original_moved() {
    let cases = moved_cases();
    let outcomes = in_parallel(&cases, |(scene, motion)| {
        let started = Instant::now();
        let outcome = carry(scene, *motion);
        let physics = scene.document.model.accepted.physics;
        let taken = outcome.as_ref().map_or(*motion, |(taken, _)| *taken);
        (
            format!("{} ({physics:?}) {taken:?}", scene.name),
            outcome.map(|(_, levels)| levels),
            started.elapsed().as_secs_f64(),
        )
    });
    within_a_halving("carried", outcomes);
}

/// The study [`MOVED`] was set from: every linear gallery scene through
/// every motion, both mirrors' turns among them, with the ratios printed.
#[test]
#[ignore]
fn study_moved_scenes() {
    let jobs = topology_examples::catalog()
        .iter()
        .filter(|example| linear(&example.document.model.accepted))
        .flat_map(|example| {
            let scene = gallery(example.name);
            let turn = turned(&scene);
            [
                SHIFT,
                Motion::QuarterTurn,
                Motion::Mirror { turn: 0.0 },
                MIRROR,
                turn,
            ]
            .map(|motion| {
                (
                    Scene {
                        name: scene.name.clone(),
                        document: scene.document.clone(),
                    },
                    motion,
                )
            })
        })
        .collect::<Vec<_>>();
    let lines = in_parallel(&jobs, |(scene, motion)| {
        let started = Instant::now();
        let line = match moved_levels(scene, *motion) {
            Ok(levels) => levels
                .iter()
                .map(|[field, complementary, own, own_b, edge]| {
                    format!(
                        "edge {edge:.3}: u {field:.2e}/{own:.2e} = {:.2} b {complementary:.2e}/{own_b:.2e} = {:.2}",
                        field / own,
                        complementary / own_b
                    )
                })
                .collect::<Vec<_>>()
                .join("; "),
            Err(error) => format!("FAILED {error}"),
        };
        format!(
            "{} {motion:?}: {line} ({:.1} s)",
            scene.name,
            started.elapsed().as_secs_f64()
        )
    });
    for line in lines {
        eprintln!("{line}");
    }
}

/// The motion that leaves every point where it is.
const STILL: Motion = Motion::Shift(Point2 { x: 0.0, y: 0.0 });

/// A scene's curves written another way, the geometry the same.
#[derive(Clone, Copy, Debug)]
enum Rewrite {
    /// Every open curve traversed the other way, through the core's own
    /// reversal, which swaps each separated span's laws; the face anchors
    /// and boundary probes on it follow.
    Reversed,
    /// Every closed curve started at its next breakpoint.
    Restarted,
    /// The curves listed backwards, which the mesher reads in order.
    Reordered,
    /// A knot inserted in the middle of every curve's first span through the
    /// editor, which inserts it without moving the curve.
    Refined,
}

/// How [`Rewrite::document`] says a scene has nothing it rewrites.
const NOTHING_TO_REWRITE: &str = "nothing to rewrite";

/// `curve`, closed, started at its next breakpoint: the same points, its
/// parameter shifted by the first interval. Of the alignments of controls to
/// knots the one that traces the same curve is taken, and none is an error.
fn restarted(curve: &TopologyCurve) -> Result<(TopologyCurve, f64), String> {
    let CurveSpline::Closed(spline) = &curve.spline else {
        return Err("not a closed curve".into());
    };
    let shift = spline.intervals()[0];
    let mut intervals = spline.intervals().to_vec();
    intervals.rotate_left(1);
    let mut multiplicities = spline.multiplicities().to_vec();
    multiplicities.rotate_left(1);
    for offset in 0..spline.controls().len() {
        let mut controls = spline.controls().to_vec();
        controls.rotate_left(offset);
        let Ok(candidate) = PeriodicCubicSpline::new_with_multiplicities(
            controls,
            intervals.clone(),
            multiplicities.clone(),
        ) else {
            continue;
        };
        let alike = (0..32).all(|index| {
            let parameter = spline.period() * index as f64 / 32.0;
            (candidate.evaluate(parameter) - spline.evaluate(parameter + shift)).norm() <= 1.0e-12
        });
        if alike {
            let mut restarted = curve.clone();
            restarted.spline = CurveSpline::Closed(candidate);
            restarted.spans.rotate_left(1);
            restarted.nodes.rotate_left(1);
            return Ok((restarted, shift));
        }
    }
    Err(format!("no restart of curve {:?} traces it", curve.id))
}

/// How a rewritten curve's parameter maps: reversed, `p` to `period - p`
/// and its faces' sides swapped; restarted, `p` to `p - shift` within the
/// period.
#[derive(Clone, Copy)]
enum Reparameterized {
    Reversed { period: f64 },
    Restarted { period: f64, shift: f64 },
}

impl Reparameterized {
    fn parameter(self, parameter: f64) -> f64 {
        match self {
            Reparameterized::Reversed { period } => period - parameter,
            Reparameterized::Restarted { period, shift } => (parameter - shift).rem_euclid(period),
        }
    }

    fn side(self, side: CurveTraceSide) -> CurveTraceSide {
        match self {
            Reparameterized::Reversed { .. } => side.opposite(),
            Reparameterized::Restarted { .. } => side,
        }
    }
}

impl Rewrite {
    fn document(self, document: &TopologyDocument) -> Result<TopologyDocument, String> {
        if let Rewrite::Refined = self {
            return refined(document);
        }
        let mut document = document.clone();
        let model = &mut document.model;
        let mut changes = BTreeMap::new();
        let mut reordered = false;
        for scene in [&mut model.draft, &mut model.accepted] {
            if let Rewrite::Reordered = self {
                reordered |= scene.geometry.curves.len() > 1;
                scene.geometry.curves.reverse();
                continue;
            }
            for curve in &mut scene.geometry.curves {
                let change = match (self, &curve.spline) {
                    (Rewrite::Reversed, CurveSpline::Open(spline)) => {
                        let period = spline.period();
                        *curve = curve.reversed().map_err(|issue| issue.to_string())?;
                        Reparameterized::Reversed { period }
                    }
                    (Rewrite::Restarted, CurveSpline::Closed(spline)) => {
                        let period = spline.period();
                        let (restarted, shift) = restarted(curve)?;
                        *curve = restarted;
                        Reparameterized::Restarted { period, shift }
                    }
                    _ => continue,
                };
                changes.insert(curve.id, change);
            }
            for assignment in &mut scene.face_assignments {
                if let FaceAnchor::Curve {
                    curve,
                    side,
                    parameter,
                    ..
                } = &mut assignment.anchor
                    && let Some(change) = changes.get(curve)
                {
                    *parameter = change.parameter(*parameter);
                    *side = change.side(*side);
                }
            }
            scene
                .geometry
                .synchronize_vertices()
                .map_err(|issue| issue.to_string())?;
        }
        for probe in &mut model.probes {
            if let TopologyProbeTarget::Boundary(target) = &mut probe.target
                && let Some(change) = changes.get(&target.curve)
            {
                target.side = change.side(target.side);
            }
        }
        if changes.is_empty() && !reordered {
            return Err(NOTHING_TO_REWRITE.into());
        }
        Ok(document)
    }
}

/// `document` with a knot in the middle of every curve's first span,
/// inserted by the editor as a user inserts a control.
fn refined(document: &TopologyDocument) -> Result<TopologyDocument, String> {
    let mut editor = TopologyEditor::from_document(document.clone())?;
    settle(&mut editor);
    let middles = editor
        .document
        .model
        .draft
        .geometry
        .curves
        .iter()
        .map(|curve| {
            let [start, end] = curve.spline.span_bounds(0).expect("a first span");
            (curve.id, 0.5 * (start + end))
        })
        .collect::<Vec<_>>();
    if middles.is_empty() {
        return Err(NOTHING_TO_REWRITE.into());
    }
    for (curve, parameter) in middles {
        editor.insert_control(curve, parameter)?;
    }
    settle(&mut editor);
    match &editor.acceptance {
        TopologyAcceptance::Valid => Ok(editor.document.clone()),
        other => Err(format!("refined, the scene is {other:?}")),
    }
}

/// IV-09's cases: each scene and the rewrite it is read against. The gate's
/// set reverses the one-sided double slit's baffle and the echo comb's
/// curves, restarts the crystal's and the dielectric whispering gallery's,
/// reorders the obstacle array's and refines the double slit, the crystal
/// and Brewster's angle; `FUNFERN_INVARIANTS_ALL=1`, nightly, every rewrite of every linear
/// gallery scene that has one.
fn rewritten_cases() -> Vec<(Scene, Rewrite)> {
    let rewrites = [
        Rewrite::Reversed,
        Rewrite::Restarted,
        Rewrite::Reordered,
        Rewrite::Refined,
    ];
    let copy = |scene: &Scene| Scene {
        name: scene.name.clone(),
        document: scene.document.clone(),
    };
    if std::env::var_os("FUNFERN_INVARIANTS_ALL").is_some() {
        return topology_examples::catalog()
            .iter()
            .filter(|example| linear(&example.document.model.accepted))
            .map(|example| gallery(example.name))
            .chain([one_sided_double_slit()])
            .flat_map(|scene| {
                rewrites
                    .into_iter()
                    .filter(|rewrite| {
                        !matches!(rewrite.document(&scene.document), Err(error) if error == NOTHING_TO_REWRITE)
                    })
                    .map(|rewrite| (copy(&scene), rewrite))
                    .collect::<Vec<_>>()
            })
            .collect();
    }
    let slit = one_sided_double_slit();
    let crystal = gallery("Anisotropic crystal");
    vec![
        (copy(&slit), Rewrite::Reversed),
        (slit, Rewrite::Refined),
        (copy(&crystal), Rewrite::Restarted),
        (crystal, Rewrite::Refined),
        (gallery("Dielectric whispering gallery"), Rewrite::Restarted),
        (gallery("Obstacle array"), Rewrite::Reordered),
        (gallery("Echo comb"), Rewrite::Reversed),
        (gallery("Brewster angle"), Rewrite::Refined),
    ]
}

#[test]
fn a_rewritten_scene_reads_as_the_original() {
    let cases = rewritten_cases();
    let outcomes = in_parallel(&cases, |(scene, rewrite)| {
        let started = Instant::now();
        let outcome = rewrite
            .document(&scene.document)
            .and_then(|other| levels_against(&scene.document, &other, STILL));
        let physics = scene.document.model.accepted.physics;
        (
            format!("{} ({physics:?}) {rewrite:?}", scene.name),
            outcome,
            started.elapsed().as_secs_f64(),
        )
    });
    within_a_halving("rewritten", outcomes);
}

/// What IV-08 draws into a box of one medium, that medium on both its sides.
#[derive(Clone, Copy, Debug)]
enum Separator {
    /// A closed loop round part of it, made a subdomain of the same material.
    Loop,
    /// A slanting open curve from the bottom side to the top, dividing it in
    /// two, the new face given the material it divides.
    Wall,
    /// An open curve with both ends free, dividing nothing.
    Free,
}

/// The default box in `physics`, before and after `separator` is drawn into
/// it through the editor, as a user draws one.
fn separated(
    physics: PhysicsModel,
    separator: Separator,
) -> Result<(TopologyDocument, TopologyDocument), String> {
    // An anisotropic medium in a turned frame, its density graded along the
    // frame's first axis, which a new face must carry over as it is.
    let mut document = TopologyDocument::default();
    for scene in [&mut document.model.draft, &mut document.model.accepted] {
        for material in &mut scene.materials {
            material.mass_density = funfern_core::ScalarField::formula("1.7 + 0.3 * x").unwrap();
            material.axis_ratio = funfern_core::ScalarField::constant(2.0);
        }
        for region in &mut scene.regions {
            region.frame.angle_radians = 0.4;
        }
    }
    let mut editor = TopologyEditor::from_document(document)?;
    settle(&mut editor);
    editor.set_physics(physics)?;
    settle(&mut editor);
    if editor.acceptance != TopologyAcceptance::Valid {
        return Err(format!("the box is {:?}", editor.acceptance));
    }
    let base = editor.document.clone();
    let scene = &base.model.accepted;
    let material = scene
        .region(BACKGROUND_REGION)
        .ok_or("the box has no background")?
        .material;
    let domain = scene.geometry.domain;
    match separator {
        Separator::Loop => {
            editor.create_closed_curve(
                PeriodicCubicSpline::rounded(Point2::new(0.21, 0.13), 0.33),
                ClosedCurvePurpose::Subdomain { material },
            )?;
        }
        Separator::Wall => {
            let (bottom, top) = (0.137, 0.237);
            let spline = OpenCubicSpline::polyline(vec![
                Point2::new(bottom, domain.min_y),
                Point2::new(top, domain.max_y),
            ])
            .map_err(|error| format!("{error:?}"))?;
            let along = |side, fraction| {
                Some(TopologyAttachment::Boundary(FaceAnchor::Outer {
                    side,
                    fraction,
                }))
            };
            editor.create_open_curve(
                spline,
                OpenCurvePurpose::SubdomainSeparator { material },
                along(OuterSide::Bottom, (bottom - domain.min_x) / domain.width()),
                // The top side runs right to left.
                along(OuterSide::Top, (domain.max_x - top) / domain.width()),
            )?;
        }
        Separator::Free => {
            let spline = OpenCubicSpline::uniform(vec![
                Point2::new(-0.5, -0.4),
                Point2::new(-0.1, -0.3),
                Point2::new(0.2, 0.2),
                Point2::new(0.45, 0.5),
            ])
            .map_err(|error| format!("{error:?}"))?;
            editor.create_open_curve(
                spline,
                OpenCurvePurpose::SubdomainSeparator { material },
                None,
                None,
            )?;
        }
    }
    settle(&mut editor);
    match &editor.acceptance {
        TopologyAcceptance::Valid => Ok((base, editor.document.clone())),
        other => Err(format!("with its separator, the box is {other:?}")),
    }
}

#[test]
fn a_separator_within_one_medium_reads_as_none() {
    let cases = [
        PhysicsModel::Mechanical,
        PhysicsModel::Electromagnetic {
            polarization: ElectromagneticPolarization::Tm,
        },
        PhysicsModel::Electromagnetic {
            polarization: ElectromagneticPolarization::Te,
        },
    ]
    .into_iter()
    .flat_map(|physics| {
        [Separator::Loop, Separator::Wall, Separator::Free].map(|separator| (physics, separator))
    })
    .collect::<Vec<_>>();
    let outcomes = in_parallel(&cases, |(physics, separator)| {
        let started = Instant::now();
        let outcome = separated(*physics, *separator)
            .and_then(|(base, with)| levels_against(&base, &with, STILL));
        (
            format!("the box ({physics:?}) with a {separator:?} separator"),
            outcome,
            started.elapsed().as_secs_f64(),
        )
    });
    within_a_halving("separated", outcomes);
}
