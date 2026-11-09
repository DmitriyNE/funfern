//! Milestone C's adaptation (`docs/spikes/funfern-testing-campaign-plan.md`,
//! section 6l): the app's own adaptation, `funfern_app::adaptation`, driven
//! on runs stepped on the CPU.
//!
//! AMR-01, the closed loop. A localized wave crosses a box whose walls let
//! it out: a narrow pulse struck at rest, which spreads as a ring and
//! leaves through every side, and a packet launched along `x`, which
//! crosses and leaves through one. The run adapts as the app adapts it. At
//! the app's cadence its state and the state a step earlier are estimated,
//! the estimate decides, and the transaction it asks for runs; the mesh it
//! makes is prepared by the runtime and published, and the field carried
//! over as the app carries it. Each job runs to its end at once, so a mesh
//! changes at the instant of its estimate, where the app's changes a few
//! frames later. The same scene runs on a fixed coarse mesh, the app's own
//! before it adapts, and on a fixed fine one at the smallest element
//! adaptation allows, the reference, all three at one step the fine mesh
//! allows, so that the time error is common and what differs is space.

mod support;
use support::*;

use funfern_app::adaptation::{
    AMR_ERROR_FLOOR_RATIO, AMR_ESTIMATE_SECONDS, AMR_ESTIMATE_STEPS, AMR_FIELD_PRESENCE_RATIO,
    AdaptationController, AdaptationStep, AmrDecision, IndicatorSnapshots, adaptation_decision,
    size_rule_refines, target_accuracy,
};
use funfern_app::document::PresentationSettings;
use funfern_app::topology_editor::{TopologyDocument, TopologyEditor};
use funfern_app::topology_runtime::{
    PreparedCanonicalTransfer, PreparedSolverUpdate, PreparedTopology,
};
use funfern_core::{
    CanonicalPointStencil, CanonicalTemporalWaveState, CanonicalWaveState, DomainRect,
    ElectromagneticPolarization, GRID_SCALE_FILTER_CADENCE, OuterBoundaryCondition,
    OuterBoundaryConditions, PhysicsModel, Point2, SolutionIndicatorResult, TriMesh,
};
use std::sync::Arc;

/// Where a run starts.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Start {
    /// A narrow pulse struck at rest off the box's middle: a ring that
    /// spreads and leaves through every side.
    Ring,
    /// A packet launched along `x` from the box's left half, which crosses
    /// and leaves through one side, spreading as it goes.
    Packet,
    /// A broad pulse struck where the ring is: smooth content, which the
    /// authored mesh carries and the estimator was calibrated on.
    Broad,
    /// At rest, for a scene whose sources and drives put the field in.
    Rest,
}

/// The broad pulse, four authored elements across.
const BROAD: Pulse = Pulse {
    centre: Point2 { x: -0.2, y: -0.1 },
    width: 0.3,
    height: 1.0,
};

/// The ring's pulse, narrow against the largest element adaptation allows,
/// so that only a refined mesh carries it.
const RING: Pulse = Pulse {
    centre: Point2 { x: -0.2, y: -0.1 },
    width: 0.06,
    height: 1.0,
};

/// The packet's carrier wavelength, its envelope's width and its start.
const PACKET_WAVELENGTH: f64 = 0.25;
const PACKET_WIDTH: f64 = 0.15;
const PACKET_START: Point2 = Point2 { x: -0.5, y: 0.0 };

/// The packet's primary field and potential at `point`: the potential a
/// carrier along `x` under a Gaussian envelope, the field its derivative
/// along `x` negated, which sends it along `x`.
fn packet(point: Point2) -> (f64, f64) {
    let k = std::f64::consts::TAU / PACKET_WAVELENGTH;
    let offset = point - PACKET_START;
    let envelope = (-offset.dot(offset) / (PACKET_WIDTH * PACKET_WIDTH)).exp();
    let (sine, cosine) = (k * offset.x).sin_cos();
    let potential = envelope * sine / k;
    let field = -envelope * (cosine - 2.0 * offset.x / (k * PACKET_WIDTH * PACKET_WIDTH) * sine);
    (field, potential)
}

/// `prepared` at rest and struck as `start` says, to be stepped at
/// `time_step`.
fn started(prepared: &PreparedTopology, time_step: f64, start: Start) -> Result<Run, String> {
    match start {
        Start::Ring => Run::struck_by(prepared, time_step, RING),
        Start::Broad => Run::struck_by(prepared, time_step, BROAD),
        Start::Rest => match &prepared.canonical_temporal_operator {
            Some(temporal) => CanonicalTemporalWaveState::zero(temporal, time_step)
                .map(Run::Temporal)
                .map_err(|error| error.to_string()),
            None => CanonicalWaveState::zero(&prepared.canonical_operator, time_step)
                .map(Run::Fixed)
                .map_err(|error| error.to_string()),
        },
        Start::Packet => {
            let operator = &prepared.canonical_operator;
            let (field, potential): (Vec<f64>, Vec<f64>) = operator
                .node_points()
                .iter()
                .map(|point| packet(*point))
                .unzip();
            CanonicalWaveState::from_primary_and_potential(operator, time_step, &field, &potential)
                .map(Run::Fixed)
                .map_err(|error| error.to_string())
        }
    }
}

/// The default box in `physics`: one medium, its sides second-order
/// outgoing walls, and adaptation as a new document has it.
fn box_document(physics: PhysicsModel) -> Result<TopologyDocument, String> {
    let mut editor = TopologyEditor::default();
    editor.set_physics(physics)?;
    settle(&mut editor);
    Ok(editor.document)
}

/// Points on a lattice of `count` by `count` cells over `domain`, at the
/// cells' middles, row by row.
fn lattice(domain: DomainRect, count: usize) -> Vec<Point2> {
    (0..count * count)
        .map(|index| {
            Point2::new(
                domain.min_x + domain.width() * ((index % count) as f64 + 0.5) / count as f64,
                domain.min_y + domain.height() * ((index / count) as f64 + 0.5) / count as f64,
            )
        })
        .collect()
}

/// Cells across the lattice the runs are read at: about two to the
/// smallest element adaptation allows, so that a narrow ring falls on it.
const LATTICE: usize = 64;

/// The point probes of `prepared` at `points`.
fn stencils(
    prepared: &PreparedTopology,
    points: &[Point2],
) -> Result<Vec<CanonicalPointStencil>, String> {
    points
        .iter()
        .map(|point| {
            let stencil = stencil_at(prepared, *point)
                .ok_or_else(|| format!("no element takes the lattice point {point:?}"))?;
            CanonicalPointStencil::from_quadratic(stencil, &prepared.canonical_operator)
                .map_err(|error| format!("{error:?}"))
        })
        .collect()
}

/// What `run` reads at each of `stencils`: the field and the complementary
/// field.
fn read(stencils: &[CanonicalPointStencil], run: &Run) -> Result<Vec<(f64, Point2)>, String> {
    stencils
        .iter()
        .map(|stencil| {
            stencil
                .sample(run.primary(), run.primary(), run.complementary())
                .map(|reading| (reading.primary, reading.complementary))
                .map_err(|error| format!("{error:?}"))
        })
        .collect()
}

/// The energy density at each lattice point, as a unit medium stores it.
fn densities(readings: &[(f64, Point2)]) -> Vec<f64> {
    readings
        .iter()
        .map(|(field, complementary)| field * field + complementary.dot(*complementary))
        .collect()
}

/// `‖read − reference‖ / ‖reference‖` over the lattice, the field and the
/// complementary field together, as a unit medium weighs them in its stored
/// energy.
fn relative_error(read: &[(f64, Point2)], reference: &[(f64, Point2)]) -> f64 {
    let difference = read
        .iter()
        .zip(reference)
        .map(
            |((field, complementary), (expected, expected_complementary))| {
                let along = *complementary - *expected_complementary;
                (field - expected).powi(2) + along.dot(along)
            },
        )
        .sum::<f64>();
    (difference / densities(reference).iter().sum::<f64>()).sqrt()
}

/// The longest edge of each triangle of `mesh`.
fn longest_edges(mesh: &TriMesh) -> Vec<f64> {
    mesh.triangles
        .iter()
        .map(|triangle| {
            let [a, b, c] = triangle.vertices.map(|vertex| mesh.vertices[vertex].point);
            (b - a).norm().max((c - b).norm()).max((a - c).norm())
        })
        .collect()
}

/// Of the triangles of `mesh` refined below `refined` along their longest
/// edge, the share of their area lying where the reference's energy density
/// on the lattice is at least `BAND` of its largest, the field's band.
fn refined_in_band(mesh: &TriMesh, refined: f64, domain: DomainRect, density: &[f64]) -> f64 {
    let most = density.iter().copied().fold(0.0, f64::max);
    let cell = |point: Point2| {
        let column = (((point.x - domain.min_x) / domain.width()) * LATTICE as f64)
            .clamp(0.0, LATTICE as f64 - 1.0) as usize;
        let row = (((point.y - domain.min_y) / domain.height()) * LATTICE as f64)
            .clamp(0.0, LATTICE as f64 - 1.0) as usize;
        row * LATTICE + column
    };
    let (mut inside, mut all) = (0.0, 0.0);
    for (triangle, edge) in mesh.triangles.iter().zip(longest_edges(mesh)) {
        if edge >= refined {
            continue;
        }
        let [a, b, c] = triangle.vertices.map(|vertex| mesh.vertices[vertex].point);
        let area = 0.5 * ((b - a).x * (c - a).y - (b - a).y * (c - a).x).abs();
        all += area;
        if density[cell((a + b + c) / 3.0)] >= BAND * most {
            inside += area;
        }
    }
    if all > 0.0 { inside / all } else { 1.0 }
}

/// A map of `mesh` on the lattice, a character per two cells: `#` refined
/// in the field's band, `+` refined outside it, `.` the band unrefined.
fn refined_map(mesh: &TriMesh, refined: f64, domain: DomainRect, density: &[f64]) -> String {
    let most = density.iter().copied().fold(0.0, f64::max);
    let half = LATTICE / 2;
    let mut marked = vec![false; half * half];
    for (triangle, edge) in mesh.triangles.iter().zip(longest_edges(mesh)) {
        if edge >= refined {
            continue;
        }
        let [a, b, c] = triangle.vertices.map(|vertex| mesh.vertices[vertex].point);
        let centre = (a + b + c) / 3.0;
        let column = (((centre.x - domain.min_x) / domain.width()) * half as f64)
            .clamp(0.0, half as f64 - 1.0) as usize;
        let row = (((centre.y - domain.min_y) / domain.height()) * half as f64)
            .clamp(0.0, half as f64 - 1.0) as usize;
        marked[row * half + column] = true;
    }
    let mut map = String::new();
    for row in (0..half).rev() {
        for column in 0..half {
            let band = (0..2).any(|dy| {
                (0..2).any(|dx| density[(2 * row + dy) * LATTICE + 2 * column + dx] >= BAND * most)
            });
            map.push(match (marked[row * half + column], band) {
                (true, true) => '#',
                (true, false) => '+',
                (false, true) => '.',
                (false, false) => ' ',
            });
        }
        map.push('\n');
    }
    map
}

/// A study's map at each estimate, a character per two lattice cells: the
/// adaptive mesh's longest edge there, `O` past `coarser`, `#` under
/// `refined`, `-` between; and beside it the adaptive run's energy density,
/// `@` at four times the reference's and at least `1e-4` of its largest,
/// `.` the reference's band.
fn excess_map(
    mesh: &TriMesh,
    refined: f64,
    coarser: f64,
    domain: DomainRect,
    density: &[f64],
    expected: &[f64],
) -> String {
    let most = expected.iter().copied().fold(0.0, f64::max);
    let half = LATTICE / 2;
    let mut longest = vec![0.0f64; half * half];
    let mut shortest = vec![f64::INFINITY; half * half];
    for (triangle, edge) in mesh.triangles.iter().zip(longest_edges(mesh)) {
        let [a, b, c] = triangle.vertices.map(|vertex| mesh.vertices[vertex].point);
        let centre = (a + b + c) / 3.0;
        let column = (((centre.x - domain.min_x) / domain.width()) * half as f64)
            .clamp(0.0, half as f64 - 1.0) as usize;
        let row = (((centre.y - domain.min_y) / domain.height()) * half as f64)
            .clamp(0.0, half as f64 - 1.0) as usize;
        longest[row * half + column] = longest[row * half + column].max(edge);
        shortest[row * half + column] = shortest[row * half + column].min(edge);
    }
    let mut map = String::new();
    for row in (0..half).rev() {
        for column in 0..half {
            let cell = row * half + column;
            map.push(if longest[cell] > coarser {
                'O'
            } else if shortest[cell] < refined {
                '#'
            } else if longest[cell] > 0.0 {
                '-'
            } else {
                ' '
            });
        }
        map.push_str("  ");
        for column in 0..half {
            let cells = (0..2)
                .flat_map(|dy| (0..2).map(move |dx| (2 * row + dy) * LATTICE + 2 * column + dx));
            let (mut excess, mut band) = (false, false);
            for index in cells {
                excess |= density[index] > 4.0 * expected[index] && density[index] > 1.0e-4 * most;
                band |= expected[index] >= BAND * most;
            }
            map.push(if excess {
                '@'
            } else if band {
                '.'
            } else {
                ' '
            });
        }
        map.push('\n');
    }
    map
}

/// The distance from `point` to the triangle `[a, b, c]`, zero inside it.
fn point_triangle_distance(point: Point2, [a, b, c]: [Point2; 3]) -> f64 {
    let side = |from: Point2, to: Point2| {
        let edge = to - from;
        let offset = point - from;
        edge.x * offset.y - edge.y * offset.x
    };
    let signs = [side(a, b), side(b, c), side(c, a)];
    if signs.iter().all(|sign| *sign >= 0.0) || signs.iter().all(|sign| *sign <= 0.0) {
        return 0.0;
    }
    let segment = |from: Point2, to: Point2| {
        let edge = to - from;
        let along = ((point - from).dot(edge) / edge.dot(edge)).clamp(0.0, 1.0);
        (point - from - edge * along).norm()
    };
    segment(a, b).min(segment(b, c)).min(segment(c, a))
}

/// The triangles of `mesh` longer than `coarser` along their longest edge
/// that lie within `reach` of a lattice point where `density` exceeds
/// `floor`: where the field can get before the next transaction, on
/// elements past the authored size.
fn exposed(
    mesh: &TriMesh,
    coarser: f64,
    reach: f64,
    points: &[Point2],
    density: &[f64],
    floor: f64,
) -> usize {
    let held = points
        .iter()
        .zip(density)
        .filter(|(_, density)| **density > floor)
        .map(|(point, _)| *point)
        .collect::<Vec<_>>();
    mesh.triangles
        .iter()
        .zip(longest_edges(mesh))
        .filter(|(triangle, edge)| {
            *edge > coarser && {
                let corners = triangle.vertices.map(|vertex| mesh.vertices[vertex].point);
                held.iter()
                    .any(|point| point_triangle_distance(*point, corners) <= reach)
            }
        })
        .count()
}

/// Where the field is, for [`refined_in_band`]: an energy density at least
/// this share of its largest.
const BAND: f64 = 1.0e-3;

/// What one estimate found and did.
#[derive(Clone, Debug)]
struct Reading {
    time: f64,
    triangles: usize,
    /// The whole field's error as the estimate puts it.
    estimate: f64,
    dormant: bool,
    /// The adaptive and the coarse run's error against the reference.
    adaptive: f64,
    coarse: f64,
    /// The reference's norm on the lattice over its largest yet, and the
    /// energy the adaptive run holds over its peak.
    norm: f64,
    energy: f64,
    /// What it decided: hold, confirm, refine or coarsen.
    decided: &'static str,
    /// Of a refinement, whether the size rule alone asked for it, the error
    /// within its target: the reach restoring an element, say.
    limits_only: bool,
    /// What the transaction changed, and the triangles after it.
    changes: usize,
    after: usize,
    /// The energy the handoff changed, relatively.
    handoff: f64,
    /// Of the area refined after the transaction, the share in the field's
    /// band.
    in_band: f64,
    /// How far the estimate put the field's reach, and the elements of the
    /// mesh after the transaction coarser than the authored one that the
    /// reference's field can get to before the next transaction.
    reach: f64,
    exposed: usize,
    /// The elements of that mesh coarser than the authored one anywhere.
    coarser: usize,
    /// Each run's primary total, which nothing in a lossless closed box
    /// changes, and the coarse run's energy over its peak.
    total: f64,
    coarse_total: f64,
    coarse_energy: f64,
    /// The wall-clock time the transaction took, in milliseconds.
    job_ms: f64,
    /// The share of the energy the cleanup took after the handoff, the
    /// stationary flux the carry left.
    cleaned: f64,
    /// With `AMR_DIAGNOSE`: the fine and the coarse run's own estimates, the
    /// fine run's energy over its peak, and the adaptive estimate's terms as
    /// shares of its residual: recovery, cell residual, interior jump and
    /// boundary.
    fine_estimate: f64,
    coarse_estimate: f64,
    fine_energy: f64,
    shares: [f64; 4],
    /// The field's share of the energy density on the lattice, the rest the
    /// complementary field's: the adaptive, the coarse and the fine run.
    primary: [f64; 3],
    /// The share of the adaptive and the coarse run's energy held by the
    /// stationary part of their complementary flux, which neither moves nor
    /// leaves.
    stationary: [f64; 2],
    /// Outside the field's band: the share of the adaptive and the coarse
    /// run's error, then of the adaptive, the coarse and the fine run's
    /// energy density.
    outside: [f64; 5],
}

/// Of `read` against `reference` on the lattice, the share of the error and
/// of `read`'s energy density at points outside the reference's band.
fn outside_band(read: &[(f64, Point2)], reference: &[(f64, Point2)]) -> (f64, f64) {
    let expected = densities(reference);
    let most = expected.iter().copied().fold(0.0, f64::max);
    let held = densities(read);
    let (mut error, mut error_outside, mut energy, mut energy_outside) = (0.0, 0.0, 0.0, 0.0);
    for (index, ((field, complementary), (field_expected, complementary_expected))) in
        read.iter().zip(reference).enumerate()
    {
        let along = *complementary - *complementary_expected;
        let difference = (field - field_expected).powi(2) + along.dot(along);
        error += difference;
        energy += held[index];
        if expected[index] < BAND * most {
            error_outside += difference;
            energy_outside += held[index];
        }
    }
    (error_outside / error, energy_outside / energy)
}

/// The share of `run`'s energy on `prepared` that the stationary part of its
/// complementary flux holds.
fn stationary_share(prepared: &PreparedTopology, run: &Run) -> Result<f64, String> {
    let operator = &prepared.canonical_operator;
    let stationary = operator
        .stationary_complementary_component(run.complementary())
        .map_err(|error| error.to_string())?;
    let energy = operator
        .constitutive_samples()
        .iter()
        .zip(&stationary)
        .map(|(sample, flux)| {
            0.5 * sample.integration_weight * flux.dot(sample.complementary_inverse.apply(*flux))
        })
        .sum::<f64>();
    Ok(energy / run.energy(prepared)?)
}

/// The field's share of the energy density over `readings`.
fn primary_share(readings: &[(f64, Point2)]) -> f64 {
    let field = readings.iter().map(|(field, _)| field * field).sum::<f64>();
    field / densities(readings).iter().sum::<f64>()
}

/// `run` on `prepared` estimated by `controller` against `previous`, the
/// state a step earlier.
fn estimated(
    controller: &mut AdaptationController,
    prepared: &PreparedTopology,
    run: &Run,
    previous: &Run,
    presentation: &PresentationSettings,
) -> Result<SolutionIndicatorResult, String> {
    let snapshots = match (run, previous) {
        (Run::Fixed(state), Run::Fixed(before)) => {
            IndicatorSnapshots::of_fixed_run(prepared, state, before)
        }
        (Run::Temporal(state), Run::Temporal(before)) => {
            let operator = prepared
                .canonical_temporal_operator
                .as_ref()
                .ok_or("a driven run on a fixed generation")?;
            IndicatorSnapshots::of_temporal_run(prepared, operator, state, before)
        }
        _ => return Err("the run and the state a step before disagree".into()),
    }
    .map_err(|error| error.to_string())?;
    controller.observe_energy(run.energy(prepared)?);
    let mut estimate = controller.estimate(prepared, snapshots, presentation);
    loop {
        if let Some(result) = estimate.advance(1 << 16) {
            break result.map_err(|error| error.to_string());
        }
    }
}

/// One AMR-01 case's runs, read at each estimate.
struct Looped {
    readings: Vec<Reading>,
    fine_triangles: usize,
    coarse_triangles: usize,
}

/// An element past this of the authored edge is coarser than the authored
/// mesh, whose longest edges reach 1.05 of it.
const EXPOSED_EDGE: f64 = 1.25;

/// How much of the fine mesh's own step the three runs take: the adaptive
/// run's generations must each allow it. A refinement's bisections leave
/// thinner elements than the mesher's at the same edge: at 0.7 the ring at a
/// tenth of the speed met a generation that allowed 0.69 of it.
const STEP_SHARE: f64 = 0.5;

/// What happens to a long run at a time, a new experiment either way: the
/// field struck again, or the point source's frequency scaled, an edit the
/// app commits in place on the mesh it runs.
#[derive(Clone, Copy, Debug)]
enum Event {
    Strike(Pulse),
    Frequency(f64),
}

/// `start` in the box in `physics`, run `duration` and adapted at
/// `speed`'s cadence, beside the coarse and the fine run until
/// `reference_until`. Past it the field has left, its error against the
/// reference reads residue over residue, and the adapted run goes on alone:
/// its readings have no errors and where the field is is its own.
fn looped(
    physics: PhysicsModel,
    start: Start,
    speed: f64,
    duration: f64,
    reference_until: f64,
) -> Result<Looped, String> {
    looped_in(
        box_document(physics)?,
        start,
        &[],
        Some(speed),
        duration,
        reference_until,
        reference_until,
    )
}

/// [`looped`] in `document` at `speed`, or its own speed ceiling, the
/// coarse run stepping beside it until `coarse_until`, which for a run
/// whose energy and totals are compared with it can outlast the fine one.
fn looped_in(
    mut document: TopologyDocument,
    start: Start,
    events: &[(f64, Event)],
    speed: Option<f64>,
    duration: f64,
    reference_until: f64,
    coarse_until: f64,
) -> Result<Looped, String> {
    // The speed ceiling the app runs this run's speed under, which bounds
    // how far the field gets between estimates.
    let speed = speed.unwrap_or(document.presentation.simulation_speed);
    document.presentation.simulation_speed = speed;
    let presentation = &document.presentation;
    let settings = presentation.adaptation;
    let domain = document.model.accepted.geometry.domain;
    let (_, _, mut fine) = prepare_at(&document, settings.minimum_edge)?;
    let (mut editor, mut runtime, mut coarse) = prepare(&document)?;
    let time_step = STEP_SHARE * fine.recommended_time_step();
    let points = lattice(domain, LATTICE);
    let fine_stencils = stencils(&fine, &points)?;
    let coarse_stencils = stencils(&coarse, &points)?;
    let mut fine_run = started(&fine, time_step, start)?;
    let mut coarse_run = started(&coarse, time_step, start)?;

    let mut controller = AdaptationController::default();
    controller.published(&coarse);
    let diagnose = std::env::var("AMR_DIAGNOSE").is_ok();
    // A study's question: what the adapted field does once the mesh stops
    // changing.
    let freeze = std::env::var("AMR_FREEZE")
        .ok()
        .and_then(|value| value.parse::<f64>().ok())
        .unwrap_or(f64::INFINITY);
    let mut fine_controller = AdaptationController::default();
    fine_controller.published(&fine);
    let mut coarse_controller = AdaptationController::default();
    coarse_controller.published(&coarse);
    let mut prepared = coarse.clone();
    let mut adaptive_stencils = stencils(&prepared, &points)?;
    let mut run = started(&prepared, time_step, start)?;
    let refined = 0.6 * document.presentation.mesh_edge;

    // The app starts an estimate once the generation has stepped, and the
    // next no sooner than its gap in wall-clock time, which a simulation
    // running at `speed` spends as that many simulated seconds.
    let gap = ((AMR_ESTIMATE_SECONDS * speed / time_step).ceil() as u64).max(AMR_ESTIMATE_STEPS);
    let total = (duration / time_step).ceil() as u64;
    let mut last = None;
    let mut largest_norm: f64 = 0.0;
    // The lattice's mean density at its largest this experiment, its peak
    // energy spread evenly over the box in the lattice's units: the
    // reference's while it steps, the adapted run's own past it or without
    // one, as the app holds the field against its experiment's peak.
    let mut largest_mean: f64 = 0.0;
    let mut readings = Vec::new();
    // The app filters the grid scale every few steps, as a new document has
    // it, and estimates no pair of states a filter stands between.
    let filtered = presentation.grid_scale_filter && std::env::var("AMR_UNFILTERED").is_err();
    let mut coarse_peak: f64 = 0.0;
    // A study's watch on every step's energy from a time on.
    let watch_from = std::env::var("AMR_WATCH_FROM")
        .ok()
        .and_then(|value| value.parse::<f64>().ok())
        .unwrap_or(f64::INFINITY);
    let watching = |step: u64| step as f64 * time_step >= watch_from;
    for step in 1..=total {
        let referenced = step as f64 * time_step <= reference_until;
        let coarsened = step as f64 * time_step <= coarse_until;
        let boundary = filtered && step.is_multiple_of(GRID_SCALE_FILTER_CADENCE);
        // An event lands after this step, a new experiment the app begins
        // when the device takes it; no estimate reads a pair across it.
        let event = events.iter().find(|(time, _)| {
            (step as f64 - 0.5) * time_step < *time && *time <= (step as f64 + 0.5) * time_step
        });
        let due = !boundary && event.is_none() && last.is_none_or(|last| step >= last + gap);
        let fine_previous = (due && diagnose && referenced).then(|| fine_run.clone());
        let coarse_previous = (due && diagnose && referenced).then(|| coarse_run.clone());
        if referenced {
            fine_run.step(&fine)?;
        }
        if coarsened {
            coarse_run.step(&coarse)?;
        }
        // The state a step earlier on this generation: a handoff falls
        // between steps, so the generation estimated has stepped since its own.
        let previous = run.clone();
        run.step(&prepared).map_err(|error| {
            format!(
                "the adapted run's step {step} at {:.4} s, {} triangles: {error}; the energy a step before {:.3e}",
                step as f64 * time_step,
                prepared.mesh.triangles.len(),
                previous.energy(&prepared).unwrap_or(f64::NAN)
            )
        })?;
        if boundary {
            if referenced {
                fine_run.filter(&fine)?;
            }
            if coarsened {
                coarse_run.filter(&coarse)?;
            }
            if watching(step) {
                eprintln!(
                    "  step {step} {:.4} s before the filter: energy {:.6e}",
                    step as f64 * time_step,
                    run.energy(&prepared).unwrap_or(f64::NAN)
                );
            }
            let unfiltered = run.clone();
            run.filter(&prepared).map_err(|error| {
                format!(
                    "the adapted run's filter at step {step}, {:.4} s, {} triangles: {error}; the energy before it {:.3e}",
                    step as f64 * time_step,
                    prepared.mesh.triangles.len(),
                    unfiltered.energy(&prepared).unwrap_or(f64::NAN)
                )
            })?;
        }
        if let Some((_, Event::Strike(pulse))) = event {
            if referenced {
                fine_run.strike(&fine, *pulse)?;
                fine_controller.mark_experiment();
            }
            if coarsened {
                coarse_run.strike(&coarse, *pulse)?;
                coarse_controller.mark_experiment();
            }
            run.strike(&prepared, *pulse)?;
            controller.mark_experiment();
            largest_mean = 0.0;
        }
        if watching(step) {
            eprintln!(
                "  step {step} {:.4} s: energy {:.6e}{}",
                step as f64 * time_step,
                run.energy(&prepared).unwrap_or(f64::NAN),
                if boundary { " filtered" } else { "" }
            );
        }
        if let Some((_, Event::Frequency(factor))) = event {
            let mut source = editor.document.model.source;
            *source.signal.carrier_mut().2 *= factor;
            editor.set_point_source(source)?;
            settle(&mut editor);
            // The fixed runs, edited on meshes the mesher builds again as
            // they were.
            let again = |prepared: &PreparedTopology, edited: Arc<PreparedTopology>| {
                if edited.mesh.triangles != prepared.mesh.triangles {
                    return Err("the edit's mesh is not the run's".to_string());
                }
                Ok(edited)
            };
            if referenced {
                fine = again(
                    &fine,
                    prepare_at(&editor.document, settings.minimum_edge)?.2,
                )?;
                fine_controller.mark_experiment();
            }
            if coarsened {
                coarse = again(&coarse, prepare(&editor.document)?.2)?;
                coarse_controller.mark_experiment();
            }
            runtime.request(
                editor.revision,
                &editor.document,
                editor.compiled_accepted.clone(),
                meshing(domain, document.presentation.mesh_edge),
                false,
            )?;
            let target = finish(&mut runtime)?;
            // The app patches the drive in place on the mesh it runs, and
            // hands off only what needs it.
            if target.solver_update == PreparedSolverUpdate::FullHandoff {
                controller.published(&target);
            }
            prepared = target;
            adaptive_stencils = stencils(&prepared, &points)?;
            controller.mark_experiment();
            largest_mean = 0.0;
        }
        if !due {
            continue;
        }
        last = Some(step);
        let adaptive_readings = read(&adaptive_stencils, &run)?;
        let (reference, coarse_readings) = if referenced {
            (
                read(&fine_stencils, &fine_run)?,
                read(&coarse_stencils, &coarse_run)?,
            )
        } else {
            (adaptive_readings.clone(), adaptive_readings.clone())
        };
        let norm = densities(&reference).iter().sum::<f64>().sqrt();
        if referenced {
            largest_norm = largest_norm.max(norm);
        }
        largest_mean = largest_mean.max(norm * norm / points.len() as f64);
        let (adaptive, coarse_error) = if referenced {
            (
                relative_error(&adaptive_readings, &reference),
                relative_error(&coarse_readings, &reference),
            )
        } else {
            (f64::NAN, f64::NAN)
        };
        let primary = [
            primary_share(&adaptive_readings),
            primary_share(&coarse_readings),
            primary_share(&reference),
        ];

        if std::env::var("AMR_EXCESS").is_ok() {
            eprintln!(
                "at {:.3} s, the mesh and the excess energy:",
                step as f64 * time_step
            );
            eprint!(
                "{}",
                excess_map(
                    &prepared.mesh,
                    refined,
                    1.25 * document.presentation.mesh_edge,
                    domain,
                    &densities(&adaptive_readings),
                    &densities(&reference),
                )
            );
        }
        let result = estimated(&mut controller, &prepared, &run, &previous, presentation)
            .map_err(|error| format!("the estimate at step {step}: {error}"))?;
        let (fine_estimate, coarse_estimate, fine_energy) = match (&fine_previous, &coarse_previous)
        {
            (Some(fine_previous), Some(coarse_previous)) => {
                let fine_result = estimated(
                    &mut fine_controller,
                    &fine,
                    &fine_run,
                    fine_previous,
                    presentation,
                )?;
                let coarse_result = estimated(
                    &mut coarse_controller,
                    &coarse,
                    &coarse_run,
                    coarse_previous,
                    presentation,
                )?;
                (
                    fine_result.report.global_indicator,
                    coarse_result.report.global_indicator,
                    fine_run.energy(&fine)? / fine_controller.energy_peak,
                )
            }
            _ => (f64::NAN, f64::NAN, f64::NAN),
        };
        let stationary = if diagnose {
            [
                stationary_share(&prepared, &run)?,
                stationary_share(&coarse, &coarse_run)?,
            ]
        } else {
            [f64::NAN; 2]
        };
        let report = &result.report;
        let shares = [
            report.recovery_contribution,
            report.cell_residual_contribution,
            report.interior_jump_contribution,
            report.boundary_residual_contribution,
        ]
        .map(|term| term / report.total_residual);
        let mut reading = Reading {
            time: step as f64 * time_step,
            triangles: prepared.mesh.triangles.len(),
            estimate: result.report.global_indicator,
            dormant: result.report.dormant,
            adaptive,
            coarse: coarse_error,
            norm: if referenced {
                norm / largest_norm
            } else {
                f64::NAN
            },
            energy: run.energy(&prepared)? / controller.energy_peak,
            decided: "hold",
            limits_only: false,
            changes: 0,
            after: prepared.mesh.triangles.len(),
            handoff: 0.0,
            in_band: f64::NAN,
            reach: result.report.field_reach,
            exposed: 0,
            coarser: 0,
            total: run.primary().iter().sum(),
            coarse_total: if coarsened {
                coarse_run.primary().iter().sum()
            } else {
                f64::NAN
            },
            coarse_energy: if coarsened {
                let energy = coarse_run.energy(&coarse)?;
                coarse_peak = coarse_peak.max(energy);
                energy / coarse_peak
            } else {
                f64::NAN
            },
            job_ms: 0.0,
            cleaned: 0.0,
            fine_estimate,
            coarse_estimate,
            fine_energy,
            shares,
            primary,
            stationary,
            outside: {
                let (adaptive_error, adaptive_energy) =
                    outside_band(&adaptive_readings, &reference);
                let (coarse_error, coarse_energy) = outside_band(&coarse_readings, &reference);
                let (_, fine_energy) = outside_band(&reference, &reference);
                [
                    adaptive_error,
                    coarse_error,
                    adaptive_energy,
                    coarse_energy,
                    fine_energy,
                ]
            },
        };
        if std::env::var("AMR_TRACE").is_ok()
            && adaptation_decision(&result.report, target_accuracy(settings))
                == AmrDecision::Coarsen
            && controller.direction(
                &result.report,
                &result.coarsen_elements,
                prepared.mesh.mesh_revision,
                target_accuracy(settings),
            ) == AmrDecision::Hold
        {
            eprintln!(
                "  {:.3} s: a coarsening of {} held by the refusals on mesh {}",
                reading.time,
                result.coarsen_elements.len(),
                prepared.mesh.mesh_revision
            );
        }
        let decided = if reading.time < freeze {
            controller.decide(&result, &prepared, settings, &mut runtime)
        } else {
            AdaptationStep::Hold
        };
        let mut job = match decided {
            AdaptationStep::Hold => None,
            AdaptationStep::Confirming => {
                reading.decided = "confirm";
                None
            }
            AdaptationStep::Initialized => {
                reading.decided = "initialize";
                None
            }
            AdaptationStep::Adapt(job) => {
                reading.limits_only = size_rule_refines(&result.report, target_accuracy(settings));
                Some(job)
            }
        };
        if let Some(job) = &mut job {
            let started_job = std::time::Instant::now();
            let result = loop {
                if let Some(result) = job.advance(1 << 16) {
                    break result.map_err(|error| error.to_string())?;
                }
            };
            reading.job_ms = started_job.elapsed().as_secs_f64() * 1000.0;
            reading.decided = if result.report.inserted_vertices > 0 {
                "refine"
            } else if result.report.collapsed_vertices > 0 {
                "coarsen"
            } else {
                "unchanged"
            };
            let (report, mesh) = controller.adapted(result, Some(&prepared.mesh));
            reading.changes = report.topology_changes;
            if let Some(mesh) = mesh {
                runtime.request_adapted_carrying(mesh, !controller.dormant_transaction)?;
                let (target, maps) = finish_with_maps(&mut runtime)
                    .map_err(|error| format!("the preparation at step {step}: {error}"))?;
                controller.published(&target);
                if target.recommended_time_step() < time_step {
                    return Err(format!(
                        "at {:.2} s an adapted mesh allows {:.3e}, under the step {time_step:.3e}",
                        reading.time,
                        target.recommended_time_step()
                    ));
                }
                let before = run.energy(&prepared)?;
                run = handed((&prepared, &run), &target, &maps, time_step).map_err(|error| {
                    format!(
                        "the carry at step {step} from {} to {} triangles: {error}; {}",
                        prepared.mesh.triangles.len(),
                        target.mesh.triangles.len(),
                        diagnose_carry((&prepared, &run), &target, &maps)
                    )
                })?;
                if std::env::var("AMR_TRACE").is_ok() {
                    eprintln!(
                        "  handoff at {:.3} s: {} to {} triangles, the carry changed the energy by {:.3e}, the shortest edge {:.4}, the recommended step {:.3e} against {:.3e}",
                        reading.time,
                        prepared.mesh.triangles.len(),
                        target.mesh.triangles.len(),
                        run.energy(&target)? / before - 1.0,
                        target
                            .mesh
                            .triangles
                            .iter()
                            .flat_map(|triangle| {
                                let [a, b, c] = triangle
                                    .vertices
                                    .map(|vertex| target.mesh.vertices[vertex].point);
                                [(b - a).norm(), (c - b).norm(), (a - c).norm()]
                            })
                            .fold(f64::INFINITY, f64::min),
                        target.recommended_time_step(),
                        time_step
                    );
                }
                // The app takes away the stationary flux the carry left once
                // it has solved for it, which on a lossless generation comes
                // to taking it away here.
                if std::env::var("AMR_UNCLEANED").is_err() {
                    reading.cleaned = cleaned(&mut run, &target)
                        .map_err(|error| format!("the cleanup at step {step}: {error}"))?;
                }
                reading.handoff = run.energy(&target)? / before - 1.0;
                prepared = target;
                adaptive_stencils = stencils(&prepared, &points)?;
                reading.after = prepared.mesh.triangles.len();
                reading.in_band =
                    refined_in_band(&prepared.mesh, refined, domain, &densities(&reference));
                if std::env::var("AMR_MAP").is_ok() {
                    eprintln!("at {:.3} s:", reading.time);
                    eprint!(
                        "{}",
                        refined_map(&prepared.mesh, refined, domain, &densities(&reference))
                    );
                }
            }
        }
        // The mesh the run steps on until the next estimate's transaction,
        // `gap` steps on, whether or not this one changed it; the box's medium
        // runs its waves at one.
        reading.coarser = longest_edges(&prepared.mesh)
            .iter()
            .filter(|edge| **edge > EXPOSED_EDGE * presentation.mesh_edge)
            .count();
        reading.exposed = exposed(
            &prepared.mesh,
            EXPOSED_EDGE * presentation.mesh_edge,
            gap as f64 * time_step,
            &points,
            &densities(&reference),
            AMR_FIELD_PRESENCE_RATIO * largest_mean,
        );
        if std::env::var("AMR_TRACE").is_ok() {
            eprintln!(
                "{:.3} s: {} triangles, estimate {:.4}{}, energy {:.2e}, {} {} changes, handoff {:.2e}, cleaned {:.2e}, {} coarsen candidates, {} refused on mesh {:?}",
                reading.time,
                reading.triangles,
                reading.estimate,
                if reading.dormant { " dormant" } else { "" },
                reading.energy,
                reading.decided,
                reading.changes,
                reading.handoff,
                reading.cleaned,
                result.report.coarsen_candidates,
                controller
                    .refused_coarsening
                    .as_ref()
                    .map_or(0, |refused| refused.elements.len()),
                controller
                    .refused_coarsening
                    .as_ref()
                    .map(|refused| refused.mesh_revision),
            );
        }
        readings.push(reading);
    }
    Ok(Looped {
        readings,
        fine_triangles: fine.mesh.triangles.len(),
        coarse_triangles: coarse.mesh.triangles.len(),
    })
}

/// The study AMR-01's bounds come from: each start in each skin, adapted at
/// the app's cadence at speed one and at slower speeds, every estimate's
/// reading printed.
#[test]
#[ignore = "a study: cargo test --release --test adaptation study -- --ignored --nocapture"]
fn study_the_adaptive_loop() {
    let physics = match std::env::var("AMR_PHYSICS").as_deref() {
        Ok("TM") => PhysicsModel::Electromagnetic {
            polarization: ElectromagneticPolarization::Tm,
        },
        Ok("TE") => PhysicsModel::Electromagnetic {
            polarization: ElectromagneticPolarization::Te,
        },
        _ => PhysicsModel::Mechanical,
    };
    let duration = std::env::var("AMR_DURATION")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(4.0);
    let starts = match std::env::var("AMR_START").as_deref() {
        Ok("ring") => vec![Start::Ring],
        Ok("packet") => vec![Start::Packet],
        Ok("broad") => vec![Start::Broad],
        _ => vec![Start::Ring, Start::Packet],
    };
    let speeds = std::env::var("AMR_SPEED")
        .ok()
        .and_then(|value| value.parse().ok())
        .map_or(vec![1.0, 0.25, 0.1], |speed| vec![speed]);
    for start in starts {
        for speed in speeds.iter().copied() {
            let started = std::time::Instant::now();
            let looped = match looped(physics, start, speed, duration, f64::INFINITY) {
                Ok(looped) => looped,
                Err(error) => {
                    eprintln!("{start:?} at speed {speed}: FAILED {error}");
                    continue;
                }
            };
            eprintln!(
                "{start:?} in {physics:?} at speed {speed}: fine {} triangles, coarse {}, in {:.1} s",
                looped.fine_triangles,
                looped.coarse_triangles,
                started.elapsed().as_secs_f64()
            );
            eprintln!(
                "{:>6} {:>6} {:>8} {:>8} {:>8} {:>6} {:>8} {:>10} {:>6} {:>6} {:>9} {:>6} {:>6} {:>7} {:>7}",
                "t",
                "tris",
                "estimate",
                "adaptive",
                "coarse",
                "norm",
                "energy",
                "decided",
                "changes",
                "after",
                "handoff",
                "band",
                "reach",
                "exposed",
                "coarser"
            );
            for reading in &looped.readings {
                eprintln!(
                    "{:>6.3} {:>6} {:>8.4}{} {:>8.4} {:>8.4} {:>6.3} {:>8.1e} {:>10} {:>6} {:>6} {:>9.2e} {:>6.2} {:>6.3} {:>7} {:>7}",
                    reading.time,
                    reading.triangles,
                    reading.estimate,
                    if reading.dormant { "d" } else { " " },
                    reading.adaptive,
                    reading.coarse,
                    reading.norm,
                    reading.energy,
                    reading.decided,
                    reading.changes,
                    reading.after,
                    reading.handoff,
                    reading.in_band,
                    reading.reach,
                    reading.exposed,
                    reading.coarser
                );
                if std::env::var("AMR_DIAGNOSE").is_ok() {
                    eprintln!(
                        "        fine estimate {:.4} energy {:.1e}, coarse estimate {:.4}; shares {:.2} {:.2} {:.2} {:.2}; field share {:.2} {:.2} {:.2}; stationary {:.1e} {:.1e}, cleaned {:.1e}; outside the band error {:.2} {:.2}, energy {:.1e} {:.1e} {:.1e}",
                        reading.fine_estimate,
                        reading.fine_energy,
                        reading.coarse_estimate,
                        reading.shares[0],
                        reading.shares[1],
                        reading.shares[2],
                        reading.shares[3],
                        reading.primary[0],
                        reading.primary[1],
                        reading.primary[2],
                        reading.stationary[0],
                        reading.stationary[1],
                        reading.cleaned,
                        reading.outside[0],
                        reading.outside[1],
                        reading.outside[2],
                        reading.outside[3],
                        reading.outside[4]
                    );
                }
            }
        }
    }
}

/// A mesh, the one adapted from it and the maps the handoff between them
/// carries the field through.
type Refinement = (
    Arc<PreparedTopology>,
    Arc<PreparedTopology>,
    Arc<PreparedCanonicalTransfer>,
);

/// The packet's first adaptive refinement, as the loop makes it from the
/// packet as it starts.
fn first_refinement(document: &TopologyDocument, time_step: f64) -> Result<Refinement, String> {
    let settings = document.presentation.adaptation;
    let (_, mut runtime, coarse) = prepare(document)?;
    let mut run = started(&coarse, time_step, Start::Packet)?;
    let previous = run.clone();
    run.step(&coarse)?;
    let mut controller = AdaptationController::default();
    controller.published(&coarse);
    let result = estimated(
        &mut controller,
        &coarse,
        &run,
        &previous,
        &document.presentation,
    )?;
    let AdaptationStep::Adapt(mut job) =
        controller.decide(&result, &coarse, settings, &mut runtime)
    else {
        return Err("the packet's first estimate did not adapt".into());
    };
    let adapted = loop {
        if let Some(result) = job.advance(1 << 16) {
            break result.map_err(|error| error.to_string())?;
        }
    };
    let (_, mesh) = controller.adapted(adapted, Some(&coarse.mesh));
    runtime.request_adapted(mesh.ok_or("the refinement changed nothing")?)?;
    let (refined, maps) = finish_with_maps(&mut runtime)?;
    Ok((coarse, refined, maps))
}

/// How long AMR-01's cases run: at full speed past two coarsenings after
/// the field has gone, at the slower ones until quiet regions have grown
/// past the authored mesh.
const LOOP_DURATION: f64 = 8.0;
/// Until when the fine and the coarse run step beside the adapted one: by
/// then the ring holds 4% of its norm and the packet 2%, and their errors
/// read residue over residue.
const LOOP_REFERENCE: f64 = 2.5;
/// The reference's norm over its largest above which the field is inside.
const INSIDE: f64 = 0.9;

/// AMR-01, the closed loop: `start` adapted at `speed`'s cadence as the app
/// adapts it, beside the coarse and the fine run. From the study of
/// 2026-11-05 (the box in the Mechanical skin, its three skins bit-identical
/// there):
///
/// - while the field is inside, the adapted run is never less accurate than
///   the coarse one. The coarse mesh holds both starts inside the 0.12
///   target but for the packet's first estimate, which reads 0.158 and
///   refines for its error: the next estimate must read at least 5% under
///   the coarse run (14% to 23% measured). The ring is not refined while
///   inside, its estimate 0.09 (AMR-02);
/// - no element past the authored size lies where the field gets before the
///   next transaction;
/// - the mesh never holds a quarter of the fine mesh's triangles (9,344 of
///   45,203 the most, the packet at a tenth);
/// - the field goes dormant, and nothing refines for its error after; the
///   reach may still restore an element where a leftover gathers;
/// - it coarsens back, to at most four fifths of its peak (0.72 at full
///   speed, 0.28 and less slower), and below full speed quiet regions grow
///   past the authored mesh;
/// - a handoff while the field holds at least the error floor's 1% of its
///   peak takes at most 1% of its energy (0.74% the most) and adds at most
///   1e-6 (3e-13). Below it, a dormant coarsening can add to what is left
///   (plan.md, Worth checking sometime).
fn closes_the_loop(start: Start, speed: f64) {
    let looped = looped(
        PhysicsModel::Mechanical,
        start,
        speed,
        LOOP_DURATION,
        LOOP_REFERENCE,
    )
    .unwrap();
    let readings = &looped.readings;
    let case = format!("{start:?} at speed {speed}");
    for reading in readings {
        assert_eq!(
            reading.exposed, 0,
            "{case} at {:.3} s: {reading:?}",
            reading.time
        );
        if reading.norm > INSIDE {
            assert!(
                reading.adaptive <= reading.coarse,
                "{case} at {:.3} s: the adapted run reads {:.4} against the coarse run's {:.4}",
                reading.time,
                reading.adaptive,
                reading.coarse
            );
        }
        if reading.changes > 0 && reading.energy >= AMR_ERROR_FLOOR_RATIO {
            assert!(
                (-1.0e-2..=1.0e-6).contains(&reading.handoff),
                "{case} at {:.3} s: a handoff changed the energy by {:.2e}",
                reading.time,
                reading.handoff
            );
        }
    }
    if start == Start::Packet {
        let refined = readings
            .iter()
            .position(|reading| reading.decided == "refine" && !reading.limits_only)
            .unwrap_or_else(|| panic!("{case}: the packet is never refined for its error"));
        let next = &readings[refined + 1];
        assert!(
            next.norm > INSIDE && next.adaptive <= 0.95 * next.coarse,
            "{case}: refined for its error at {:.3} s, the packet reads {:.4} against {:.4} at {:.3} s",
            readings[refined].time,
            next.adaptive,
            next.coarse,
            next.time
        );
    }
    let peak = readings.iter().map(|reading| reading.after).max().unwrap();
    assert!(
        4 * peak < looped.fine_triangles,
        "{case}: the mesh peaked at {peak} triangles against the fine mesh's {}",
        looped.fine_triangles
    );
    let dormant = readings
        .iter()
        .position(|reading| reading.dormant)
        .unwrap_or_else(|| panic!("{case} never goes dormant"));
    for reading in &readings[dormant..] {
        assert!(
            reading.decided != "refine" || reading.limits_only,
            "{case} refines for its error at {:.3} s, dormant since {:.3} s",
            reading.time,
            readings[dormant].time
        );
    }
    let last = readings.last().unwrap();
    assert!(
        last.after as f64 <= 0.8 * peak as f64,
        "{case} ends at {} triangles from a peak of {peak}",
        last.after
    );
    if speed < 1.0 {
        assert!(
            last.coarser > 0,
            "{case}: no quiet region grew past the authored mesh"
        );
    }
}

#[test]
fn a_ring_at_full_speed_closes_the_loop() {
    closes_the_loop(Start::Ring, 1.0);
}

#[test]
fn a_ring_at_a_quarter_of_the_speed_closes_the_loop() {
    closes_the_loop(Start::Ring, 0.25);
}

#[test]
fn a_ring_at_a_tenth_of_the_speed_closes_the_loop() {
    closes_the_loop(Start::Ring, 0.1);
}

#[test]
fn a_packet_at_full_speed_closes_the_loop() {
    closes_the_loop(Start::Packet, 1.0);
}

#[test]
fn a_packet_at_a_quarter_of_the_speed_closes_the_loop() {
    closes_the_loop(Start::Packet, 0.25);
}

#[test]
fn a_packet_at_a_tenth_of_the_speed_closes_the_loop() {
    closes_the_loop(Start::Packet, 0.1);
}

/// A packet at a tenth of the speed ceiling, adapted as the app adapts it.
/// Its first estimate refines around it, and once it has left that patch the
/// estimate reads well under target and asks to coarsen. Judged by where the
/// field was then, the region just ahead of it read as quiet and coarsened
/// to twice the authored edge; the packet crossing it scattered a few
/// percent of its energy back, and the adapted run was the less accurate
/// while the packet was still inside, 0.150 against the coarse run's 0.146
/// at t = 1.28 (at a quarter of the speed 0.154 against 0.135 at 1.13).
/// Held at the authored size within the field's reach, the coarsening grows
/// 188 elements past the authored size ahead of it and the reach refines
/// them back as it comes, the last at 1.36: no element coarser than the
/// authored one lies where the field gets before the next transaction, and
/// while the packet is inside the adapted run is never the less accurate,
/// 0.145 against 0.149 at 1.36 the closest.
#[test]
fn a_slow_packet_never_meets_a_mesh_coarsened_ahead_of_it() {
    let looped = looped(PhysicsModel::Mechanical, Start::Packet, 0.1, 1.6, 1.6).unwrap();
    assert!(
        looped
            .readings
            .iter()
            .any(|reading| reading.decided == "coarsen" && reading.coarser > 0),
        "the packet's patch coarsens behind it, and the region ahead past the authored size"
    );
    for reading in &looped.readings {
        assert_eq!(reading.exposed, 0, "at {:.3} s: {reading:?}", reading.time);
        if reading.norm > INSIDE {
            assert!(
                reading.adaptive <= reading.coarse,
                "at {:.3} s the adapted run reads {:.4} against the coarse run's {:.4}",
                reading.time,
                reading.adaptive,
                reading.coarse
            );
        }
    }
}

/// The handoff's carry leaves part of the flux stationary, and the app takes
/// it away. The packet as it starts, carried onto its first adaptive
/// refinement, whose edge flips leave new elements across old ones, and
/// onto the box meshed again a twentieth finer, as an edit that keeps the
/// field remeshes it: the carry leaves 1.27e-3 and 5.63e-3 of the energy
/// stationary, held to 1e-4 below, and the cleanup takes it, leaving 3e-22.
/// Taken away, the field moves on as it would have: 400 steps on it is the
/// uncleaned run's to 3.9e-7 and 5.2e-7, held to 2e-6, what the solve's
/// tolerance leaves pushing the nodes, and none of the part has grown back.
#[test]
fn a_handoffs_stationary_flux_is_taken_away_and_moves_nothing() {
    let document = box_document(PhysicsModel::Mechanical).unwrap();
    let (_, _, plain) = prepare(&document).unwrap();
    let time_step = 0.4 * plain.recommended_time_step();
    let (coarse, refined, to_refined) = first_refinement(&document, time_step).unwrap();
    let (editor, mut remesher, _) = prepare(&document).unwrap();
    remesher
        .request(
            editor.revision,
            &editor.document,
            editor.compiled_accepted.clone(),
            meshing(
                document.model.accepted.geometry.domain,
                0.95 * document.presentation.mesh_edge,
            ),
            false,
        )
        .unwrap();
    let (remeshed, to_remeshed) = finish_with_maps(&mut remesher).unwrap();
    for (name, target, maps) in [
        ("refinement", &refined, &to_refined),
        ("remesh", &remeshed, &to_remeshed),
    ] {
        assert!(target.recommended_time_step() >= time_step, "{name}");
        let run = started(&coarse, time_step, Start::Packet).unwrap();
        let carried = handed((&coarse, &run), target, maps, time_step).unwrap();
        let left = stationary_share(target, &carried).unwrap();
        let mut clean = carried.clone();
        let taken = cleaned(&mut clean, target).unwrap();
        let after = stationary_share(target, &clean).unwrap();
        let mut kept = carried;
        for _ in 0..400 {
            clean.step(target).unwrap();
            kept.step(target).unwrap();
        }
        let (apart, scale) = clean
            .primary()
            .iter()
            .zip(kept.primary())
            .fold((0.0, 0.0), |(apart, scale), (clean, kept)| {
                (apart + (clean - kept).powi(2), scale + kept * kept)
            });
        let apart = (apart / scale).sqrt();
        let regrown = stationary_share(target, &clean).unwrap();
        eprintln!(
            "{name}: the carry left {left:.2e} stationary, the cleanup took {taken:.2e} and left {after:.1e}; 400 steps on, the field {apart:.1e} from the uncleaned one's, {regrown:.1e} stationary"
        );
        assert!(left > 1.0e-4, "{name}: the carry left only {left:e}");
        assert!(
            (taken - left).abs() < 1.0e-6 * left,
            "{name}: took {taken:e} of {left:e}"
        );
        assert!(after < 1.0e-12, "{name}: {after:e} left stationary");
        assert!(apart < 2.0e-6, "{name}: the field moved {apart:e} apart");
        assert!(regrown < 1.0e-12, "{name}: {regrown:e} grew back");
    }
}

/// The packet on `prepared` at `scale` of its amplitude.
fn packet_at(prepared: &PreparedTopology, time_step: f64, scale: f64) -> Result<Run, String> {
    let operator = &prepared.canonical_operator;
    let (field, potential): (Vec<f64>, Vec<f64>) = operator
        .node_points()
        .iter()
        .map(|point| {
            let (field, potential) = packet(*point);
            (scale * field, scale * potential)
        })
        .unzip();
    CanonicalWaveState::from_primary_and_potential(operator, time_step, &field, &potential)
        .map(Run::Fixed)
        .map_err(|error| error.to_string())
}

/// A packet a two-hundredth of a loud one's amplitude, launched after the
/// loud one: against the loud one's peak it lies under the dormant floor, a
/// four-hundredth of its energy below it, and its error is not chased. Once
/// its own experiment begins, as a pulse, a Switch or a source edit begins
/// one, the estimate reads its error as the loud one's and asks to refine.
#[test]
fn a_weak_packet_after_a_loud_one_is_refined_once_its_experiment_begins() {
    let document = box_document(PhysicsModel::Mechanical).unwrap();
    let settings = document.presentation.adaptation;
    let (_, _, coarse) = prepare(&document).unwrap();
    let time_step = 0.5 * coarse.recommended_time_step();
    let loud = packet_at(&coarse, time_step, 1.0).unwrap();
    let mut controller = AdaptationController::default();
    controller.published(&coarse);
    controller.observe_energy(loud.energy(&coarse).unwrap());
    let mut weak = packet_at(&coarse, time_step, 1.0 / 200.0).unwrap();
    let previous = weak.clone();
    weak.step(&coarse).unwrap();
    let target = target_accuracy(settings);
    let decided = |controller: &mut AdaptationController| {
        let result = estimated(
            controller,
            &coarse,
            &weak,
            &previous,
            &document.presentation,
        )
        .unwrap();
        let decision = adaptation_decision(&result.report, target);
        (result.report, decision)
    };
    let (report, decision) = decided(&mut controller);
    assert!(
        report.dormant && decision != AmrDecision::Refine,
        "under the loud one's floor: dormant {}, {decision:?}",
        report.dormant
    );
    controller.mark_experiment();
    let (report, decision) = decided(&mut controller);
    eprintln!(
        "the weak packet's own experiment reads {:.4} against the target {target:.2}",
        report.global_indicator,
    );
    assert!(
        !report.dormant && decision == AmrDecision::Refine,
        "its own experiment: dormant {}, {decision:?}",
        report.dormant
    );
}

/// Dunavant's six-point rule, exact to degree four, in barycentric
/// coordinates with weights summing to one: where AMR-02 reads each element.
const QUADRATURE: [([f64; 3], f64); 6] = [
    (
        [
            0.108_103_018_168_070,
            0.445_948_490_915_965,
            0.445_948_490_915_965,
        ],
        0.223_381_589_678_011,
    ),
    (
        [
            0.445_948_490_915_965,
            0.108_103_018_168_070,
            0.445_948_490_915_965,
        ],
        0.223_381_589_678_011,
    ),
    (
        [
            0.445_948_490_915_965,
            0.445_948_490_915_965,
            0.108_103_018_168_070,
        ],
        0.223_381_589_678_011,
    ),
    (
        [
            0.816_847_572_980_459,
            0.091_576_213_509_771,
            0.091_576_213_509_771,
        ],
        0.109_951_743_655_322,
    ),
    (
        [
            0.091_576_213_509_771,
            0.816_847_572_980_459,
            0.091_576_213_509_771,
        ],
        0.109_951_743_655_322,
    ),
    (
        [
            0.091_576_213_509_771,
            0.091_576_213_509_771,
            0.816_847_572_980_459,
        ],
        0.109_951_743_655_322,
    ),
];

/// The point probes of `prepared` at `points`, built on every thread: each
/// finds its element by a search over the mesh.
fn stencils_in_parallel(
    prepared: &PreparedTopology,
    points: &[Point2],
) -> Result<Vec<CanonicalPointStencil>, String> {
    in_parallel(points, |point| {
        let stencil = stencil_at(prepared, *point)
            .ok_or_else(|| format!("no element takes the point {point:?}"))?;
        CanonicalPointStencil::from_quadratic(stencil, &prepared.canonical_operator)
            .map_err(|error| format!("{error:?}"))
    })
    .into_iter()
    .collect()
}

/// One uniform mesh of AMR-02's sequence: its generation, the fine one and
/// the maps that carry the reference's state onto it, its own run from the
/// start and that run a step before, and each element's quadrature points,
/// their weights times its area, read on both meshes.
struct Level {
    edge: f64,
    source: Arc<PreparedTopology>,
    prepared: Arc<PreparedTopology>,
    maps: Arc<PreparedCanonicalTransfer>,
    run: Run,
    previous: Run,
    weights: Vec<f64>,
    elements: Vec<usize>,
    fine_stencils: Vec<CanonicalPointStencil>,
    stencils: Vec<CanonicalPointStencil>,
    /// The reference read at the level's own constitutive samples, where the
    /// solver holds its flux.
    sample_stencils: Vec<CanonicalPointStencil>,
    accumulated: AdaptationController,
    instantaneous: AdaptationController,
}

/// Each element's squared error against `reference` and the reference's own
/// square, integrated by [`QUADRATURE`], the field and the complementary
/// field together as a unit medium weighs them.
fn element_errors(
    read: &[(f64, Point2)],
    reference: &[(f64, Point2)],
    weights: &[f64],
    elements: &[usize],
    count: usize,
) -> (Vec<f64>, Vec<f64>) {
    let (mut errors, mut norms) = (vec![0.0; count], vec![0.0; count]);
    for ((((field, complementary), (expected, expected_complementary)), weight), element) in
        read.iter().zip(reference).zip(weights).zip(elements)
    {
        let along = *complementary - *expected_complementary;
        errors[*element] += weight * ((field - expected).powi(2) + along.dot(along));
        norms[*element] +=
            weight * (expected * expected + expected_complementary.dot(*expected_complementary));
    }
    (errors, norms)
}

/// Where an estimate points: ranking `mesh`'s elements by `key`, highest
/// first, the share of the true squared error `errors` the first `share` of
/// the area holds, over the most any elements of that area hold. One is
/// pointing at the error; the area's own share, at random.
fn pointing(mesh: &TriMesh, key: &[f64], errors: &[f64], share: f64) -> f64 {
    let areas = mesh
        .triangles
        .iter()
        .map(|triangle| {
            let [a, b, c] = triangle.vertices.map(|vertex| mesh.vertices[vertex].point);
            0.5 * ((b - a).x * (c - a).y - (b - a).y * (c - a).x).abs()
        })
        .collect::<Vec<_>>();
    let total_area = areas.iter().sum::<f64>();
    let total_error = errors.iter().sum::<f64>();
    let held = |order: &[usize]| {
        let (mut area, mut error) = (0.0, 0.0);
        for element in order {
            let room = share * total_area - area;
            if room <= 0.0 {
                break;
            }
            let part = (room / areas[*element]).min(1.0);
            area += part * areas[*element];
            error += part * errors[*element];
        }
        error / total_error
    };
    let mut by_indicator = (0..errors.len()).collect::<Vec<_>>();
    by_indicator.sort_by(|a, b| key[*b].total_cmp(&key[*a]));
    let mut by_error = (0..errors.len()).collect::<Vec<_>>();
    by_error.sort_by(|a, b| (errors[*b] / areas[*b]).total_cmp(&(errors[*a] / areas[*a])));
    held(&by_indicator) / held(&by_error)
}

impl Level {
    /// The box of `document` meshed uniformly at `edge`, with the maps that
    /// carry the state of `fine`, the box at `fine_edge`, onto it, read at its
    /// elements' quadrature points on both meshes.
    fn new(
        document: &TopologyDocument,
        fine: &PreparedTopology,
        fine_edge: f64,
        edge: f64,
        time_step: f64,
    ) -> Result<Self, String> {
        let domain = document.model.accepted.geometry.domain;
        let (editor, mut runtime, source) = prepare_at(document, fine_edge)?;
        runtime.request(
            editor.revision,
            &editor.document,
            editor.compiled_accepted.clone(),
            meshing(domain, edge),
            false,
        )?;
        let (prepared, maps) = finish_with_maps(&mut runtime)?;
        if prepared.recommended_time_step() < time_step {
            return Err(format!(
                "the mesh at {edge} does not allow the reference's step"
            ));
        }
        let mesh = &prepared.mesh;
        let (mut points, mut weights, mut elements) = (Vec::new(), Vec::new(), Vec::new());
        for (element, triangle) in mesh.triangles.iter().enumerate() {
            let [a, b, c] = triangle.vertices.map(|vertex| mesh.vertices[vertex].point);
            let area = 0.5 * ((b - a).x * (c - a).y - (b - a).y * (c - a).x).abs();
            for (barycentric, weight) in QUADRATURE {
                points.push(a * barycentric[0] + b * barycentric[1] + c * barycentric[2]);
                weights.push(weight * area);
                elements.push(element);
            }
        }
        let fine_stencils = stencils_in_parallel(fine, &points)?;
        let stencils = stencils_in_parallel(&prepared, &points)?;
        let sample_points = prepared
            .canonical_operator
            .constitutive_samples()
            .iter()
            .map(|sample| sample.point)
            .collect::<Vec<_>>();
        let sample_stencils = stencils_in_parallel(fine, &sample_points)?;
        // Each start puts its own run here.
        let run = started(&prepared, time_step, Start::Broad)?;
        Ok(Self {
            edge,
            source,
            prepared,
            maps,
            previous: run.clone(),
            run,
            weights,
            elements,
            fine_stencils,
            stencils,
            sample_stencils,
            accumulated: AdaptationController::default(),
            instantaneous: AdaptationController::default(),
        })
    }

    /// The level's own run struck as `start`, and estimates begun afresh.
    fn start(&mut self, start: Start, time_step: f64) -> Result<(), String> {
        self.run = started(&self.prepared, time_step, start)?;
        self.accumulated = AdaptationController::default();
        self.accumulated.published(&self.prepared);
        self.instantaneous = AdaptationController::default();
        self.instantaneous.published(&self.prepared);
        Ok(())
    }

    /// What the level reads against `fine_run`, the reference stepped with
    /// its own run: both truths and the estimate of each.
    fn calibrated(
        &mut self,
        fine_run: &Run,
        presentation: &PresentationSettings,
        time_step: f64,
    ) -> Result<Calibration, String> {
        let count = self.prepared.mesh.triangles.len();
        let reference = read(&self.fine_stencils, fine_run)?;
        let errors = |read: &[(f64, Point2)]| {
            element_errors(read, &reference, &self.weights, &self.elements, count)
        };
        let (accumulated_errors, norms) = errors(&read(&self.stencils, &self.run)?);
        let norm = norms.iter().sum::<f64>();
        let accumulated = estimated(
            &mut self.accumulated,
            &self.prepared,
            &self.run,
            &self.previous,
            presentation,
        )?;
        let mut carried = handed(
            (&self.source, fine_run),
            &self.prepared,
            &self.maps,
            time_step,
        )?;
        cleaned(&mut carried, &self.prepared)?;
        let carried_readings = read(&self.stencils, &carried)?;
        let (instantaneous_errors, _) = errors(&carried_readings);
        // The field's and the flux's share of that error apart, the field's
        // norm alone, and the flux the solver holds at its own samples against
        // the reference's there: the point probes recover theirs from the
        // samples around them, a post-processed flux that converges faster.
        let mut parts = [0.0; 2];
        let mut field_norm = 0.0;
        for (((field, complementary), (expected, expected_complementary)), weight) in
            carried_readings.iter().zip(&reference).zip(&self.weights)
        {
            let along = *complementary - *expected_complementary;
            parts[0] += weight * (field - expected).powi(2);
            parts[1] += weight * along.dot(along);
            field_norm += weight * expected * expected;
        }
        let reference_samples = read(&self.sample_stencils, fine_run)?;
        let (mut raw_flux, mut flux_norm) = (0.0, 0.0);
        for ((sample, flux), (_, expected)) in self
            .prepared
            .canonical_operator
            .constitutive_samples()
            .iter()
            .zip(carried.complementary())
            .zip(&reference_samples)
        {
            let difference = sample.complementary_inverse.apply(*flux) - *expected;
            raw_flux += sample.integration_weight * difference.dot(difference);
            flux_norm += sample.integration_weight * expected.dot(*expected);
        }
        let raw_norm = field_norm + flux_norm;
        let before = carried.clone();
        carried.step(&self.prepared)?;
        let instantaneous = estimated(
            &mut self.instantaneous,
            &self.prepared,
            &carried,
            &before,
            presentation,
        )?;
        Ok(Calibration {
            norm,
            accumulated_error: (accumulated_errors.iter().sum::<f64>() / norm).sqrt(),
            accumulated,
            accumulated_errors,
            instantaneous_error: (instantaneous_errors.iter().sum::<f64>() / norm).sqrt(),
            instantaneous_parts: parts.map(|part| (part / norm).sqrt()),
            instantaneous_raw: ((parts[0] + raw_flux) / raw_norm).sqrt(),
            instantaneous_raw_flux: (raw_flux / raw_norm).sqrt(),
            instantaneous,
            instantaneous_errors,
        })
    }
}

/// What one level read at one instant: the reference's squared norm over
/// it, and against each truth the relative error, its estimate and each
/// element's squared error.
struct Calibration {
    norm: f64,
    accumulated_error: f64,
    accumulated: SolutionIndicatorResult,
    accumulated_errors: Vec<f64>,
    instantaneous_error: f64,
    /// The field's and the flux's part of the instantaneous error, each over
    /// the reference's whole norm.
    instantaneous_parts: [f64; 2],
    /// The instantaneous error with the flux the solver holds at its own
    /// samples in place of the probes' recovered one, and that flux's part.
    instantaneous_raw: f64,
    instantaneous_raw_flux: f64,
    instantaneous: SolutionIndicatorResult,
    instantaneous_errors: Vec<f64>,
}

/// What [`study_the_estimators_calibration`] fits a rate to, on the carried
/// state: its error, the field's and the flux's part of it, the estimate,
/// and each term's share of the estimate - the complementary recovery, the
/// cell residual, the interior jump and the boundary - with the scalar
/// gradient recovery the canonical estimate leaves out beside them.
const TERMS: [&str; 12] = [
    "error",
    "field",
    "flux",
    "raw error",
    "raw flux",
    "raw index",
    "estimate",
    "recovery",
    "cell",
    "jump",
    "boundary",
    "scalar",
];

/// [`TERMS`] of `calibration`.
fn terms(calibration: &Calibration) -> [f64; 12] {
    let report = &calibration.instantaneous.report;
    let estimate = report.global_indicator;
    let share = |term: f64| estimate * (term / report.total_residual).sqrt();
    [
        calibration.instantaneous_error,
        calibration.instantaneous_parts[0],
        calibration.instantaneous_parts[1],
        calibration.instantaneous_raw,
        calibration.instantaneous_raw_flux,
        estimate / calibration.instantaneous_raw,
        estimate,
        share(report.recovery_contribution),
        share(report.cell_residual_contribution),
        share(report.interior_jump_contribution),
        share(report.boundary_residual_contribution),
        share(report.displacement_recovery_contribution),
    ]
}

/// `start` struck on the reference `fine` and on each of `levels`, stepped
/// together at `time_step` to `until` as the app steps them, grid filter
/// included, each level calibrated every `every` seconds and handed to
/// `observe` with the time.
#[allow(clippy::too_many_arguments)]
fn calibrate(
    fine: &PreparedTopology,
    levels: &mut [Level],
    start: Start,
    until: f64,
    every: f64,
    presentation: &PresentationSettings,
    time_step: f64,
    mut observe: impl FnMut(f64, &Level, &Calibration),
) -> Result<(), String> {
    let mut fine_run = started(fine, time_step, start)?;
    for level in levels.iter_mut() {
        level.start(start, time_step)?;
    }
    let total = (until / time_step).ceil() as u64;
    let mut next = every;
    for step in 1..=total {
        let boundary = step.is_multiple_of(GRID_SCALE_FILTER_CADENCE);
        let due = !boundary && step as f64 * time_step >= next;
        for level in levels.iter_mut() {
            if due {
                level.previous = level.run.clone();
            }
            level.run.step(&level.prepared)?;
            if boundary {
                level.run.filter(&level.prepared)?;
            }
        }
        fine_run.step(fine)?;
        if boundary {
            fine_run.filter(fine)?;
        }
        if !due {
            continue;
        }
        next += every;
        for level in levels.iter_mut() {
            let calibration = level.calibrated(&fine_run, presentation, time_step)?;
            observe(step as f64 * time_step, level, &calibration);
        }
    }
    Ok(())
}

/// An estimate ranks first the elements that hold the error (AMR-02): the
/// packet and the ring on the authored mesh, the reference's state, the box
/// at 0.02, carried onto it every tenth of a second to 0.5 s. Ranked by the
/// indicators the size targets are made from, the tenth of the box ranked
/// first must hold at least 80% of the error the best tenth holds. Weighed
/// against each element's own energy, as they were until 2026-11-05, the
/// tenth they ranked first held under 4%: nearly empty elements read their
/// leftovers over themselves and came first.
#[test]
fn an_estimate_ranks_first_the_elements_holding_the_error() {
    let document = box_document(PhysicsModel::Mechanical).unwrap();
    let fine_edge = document.presentation.adaptation.minimum_edge;
    let (_, _, fine) = prepare_at(&document, fine_edge).unwrap();
    let time_step = STEP_SHARE * fine.recommended_time_step();
    let mut levels = vec![
        Level::new(
            &document,
            &fine,
            fine_edge,
            document.presentation.mesh_edge,
            time_step,
        )
        .unwrap(),
    ];
    for start in [Start::Packet, Start::Ring] {
        calibrate(
            &fine,
            &mut levels,
            start,
            0.5,
            0.1,
            &document.presentation,
            time_step,
            |time, level, calibration| {
                let points = pointing(
                    &level.prepared.mesh,
                    &calibration.instantaneous.element_indicators,
                    &calibration.instantaneous_errors,
                    0.1,
                );
                eprintln!("{start:?} at {time:.3} s: the first tenth holds {points:.2}");
                assert!(
                    points >= 0.8,
                    "{start:?} at {time:.3} s: the tenth ranked first holds {points:.2} of the best"
                );
            },
        )
        .unwrap();
    }
}

/// AMR-02: the estimate reads the solver's error alike on every mesh. The
/// packet's and the ring's reference state, the box at 0.014, carried onto
/// the box meshed uniformly at the authored 0.08 and at half that every tenth
/// of a second while the field is inside; its error against the flux the
/// solver holds at its own samples. The index, estimate over error, must
/// stay within [1.2, 2.4] at every reading on both meshes (1.42 to 1.91
/// measured), and its median on the finer within 0.7 to 1.3 of the coarser's
/// (0.84 and 0.85): an index that moved with the mesh would make the accuracy
/// target mean another accuracy on each. Read against the point probes'
/// recovered flux, which converges as h³ where the solver's own does as h²,
/// it seemed to rise from 1.9 to 3.9 (2026-11-04).
#[test]
fn an_estimates_index_holds_under_refinement() {
    let document = box_document(PhysicsModel::Mechanical).unwrap();
    let fine_edge = 0.014;
    let (_, _, fine) = prepare_at(&document, fine_edge).unwrap();
    let time_step = STEP_SHARE * fine.recommended_time_step();
    let edges = [
        document.presentation.mesh_edge,
        0.5 * document.presentation.mesh_edge,
    ];
    let mut levels = edges
        .iter()
        .map(|edge| Level::new(&document, &fine, fine_edge, *edge, time_step).unwrap())
        .collect::<Vec<_>>();
    for start in [Start::Packet, Start::Ring] {
        let mut largest_norm: f64 = 0.0;
        let mut indices = vec![Vec::new(); edges.len()];
        calibrate(
            &fine,
            &mut levels,
            start,
            0.7,
            0.1,
            &document.presentation,
            time_step,
            |time, level, calibration| {
                largest_norm = largest_norm.max(calibration.norm.sqrt());
                if calibration.norm.sqrt() / largest_norm <= INSIDE {
                    return;
                }
                let index = calibration.instantaneous.report.global_indicator
                    / calibration.instantaneous_raw;
                eprintln!(
                    "{start:?} at {time:.3} s on {}: index {index:.3}",
                    level.edge
                );
                assert!(
                    (1.2..=2.4).contains(&index),
                    "{start:?} at {time:.3} s on {}: the index reads {index:.3}",
                    level.edge
                );
                let level = edges.iter().position(|edge| *edge == level.edge).unwrap();
                indices[level].push(index);
            },
        )
        .unwrap();
        let median = |values: &mut Vec<f64>| {
            values.sort_by(f64::total_cmp);
            values[values.len() / 2]
        };
        let (coarse, finer) = (median(&mut indices[0]), median(&mut indices[1]));
        eprintln!(
            "{start:?}: the median index {coarse:.3} on the authored mesh, {finer:.3} on half"
        );
        assert!(
            (0.7..=1.3).contains(&(finer / coarse)),
            "{start:?}: the median index moved from {coarse:.3} to {finer:.3} with the mesh"
        );
    }
}

/// AMR-02's study: the estimator's efficiency index, its estimate over the
/// true error, for each start in the box on a sequence of uniform meshes,
/// against two truths. Accumulated: the mesh's own run from the start
/// against a reference twice as fine as the smallest element adaptation
/// allows, which carries every phase error since. Instantaneous: the
/// reference's state carried onto the mesh as the app carries a field, the
/// error the mesh cannot avoid now, whatever went before. Each estimate is
/// also asked where it points (see [`pointing`]).
#[test]
#[ignore = "a study: cargo test --release --test adaptation study_the_estimators -- --ignored --nocapture"]
fn study_the_estimators_calibration() {
    let variable = |name: &str| std::env::var(name).ok();
    let starts = match variable("CAL_START").as_deref() {
        Some("ring") => vec![Start::Ring],
        Some("packet") => vec![Start::Packet],
        Some("broad") => vec![Start::Broad],
        _ => vec![Start::Broad, Start::Ring, Start::Packet],
    };
    let edges = variable("CAL_LEVELS")
        .map(|levels| {
            levels
                .split(',')
                .map(|edge| edge.parse::<f64>().unwrap())
                .collect::<Vec<_>>()
        })
        .unwrap_or(vec![0.08, 0.057, 0.04]);
    let fine_edge = variable("CAL_REFERENCE").map_or(0.014, |edge| edge.parse().unwrap());
    let until = variable("CAL_UNTIL").map_or(1.4, |time| time.parse().unwrap());
    let every = variable("CAL_EVERY").map_or(0.1, |time| time.parse().unwrap());
    let document = box_document(PhysicsModel::Mechanical).unwrap();
    let started_at = std::time::Instant::now();
    let (_, _, fine) = prepare_at(&document, fine_edge).unwrap();
    let time_step = STEP_SHARE * fine.recommended_time_step();
    let mut levels = edges
        .iter()
        .map(|edge| Level::new(&document, &fine, fine_edge, *edge, time_step).unwrap())
        .collect::<Vec<_>>();
    eprintln!(
        "reference at {fine_edge}: {} triangles, step {time_step:.3e}; levels {:?} of {:?} triangles; prepared in {:.1} s",
        fine.mesh.triangles.len(),
        edges,
        levels
            .iter()
            .map(|level| level.prepared.mesh.triangles.len())
            .collect::<Vec<_>>(),
        started_at.elapsed().as_secs_f64()
    );
    for start in starts {
        let started_at = std::time::Instant::now();
        eprintln!("{start:?}:");
        eprintln!(
            "{:>6} {:>6} {:>6} | {:>8} {:>8} {:>6} | {:>8} {:>8} {:>6} | {:>6} {:>6} | {:>6} {:>6}",
            "t",
            "norm",
            "edge",
            "acc err",
            "estimate",
            "index",
            "now err",
            "estimate",
            "index",
            "acc10",
            "acc25",
            "now10",
            "now25"
        );
        let mut largest_norm: f64 = 0.0;
        // Each level's readings of the field inside, for the rates.
        let mut inside = vec![Vec::new(); levels.len()];
        calibrate(
            &fine,
            &mut levels,
            start,
            until,
            every,
            &document.presentation,
            time_step,
            |time, level, calibration| {
                largest_norm = largest_norm.max(calibration.norm.sqrt());
                let mesh = &level.prepared.mesh;
                let accumulated = calibration.accumulated.report.global_indicator;
                let instantaneous = calibration.instantaneous.report.global_indicator;
                let points = |result: &SolutionIndicatorResult, errors: &[f64], share: f64| {
                    pointing(mesh, &result.element_indicators, errors, share)
                };
                eprintln!(
                    "{:>6.3} {:>6.3} {:>6.3} | {:>8.4} {:>8.4} {:>6.2} | {:>8.4} {:>8.4} {:>6.2} | {:>6.2} {:>6.2} | {:>6.2} {:>6.2}",
                    time,
                    calibration.norm.sqrt() / largest_norm,
                    level.edge,
                    calibration.accumulated_error,
                    accumulated,
                    accumulated / calibration.accumulated_error,
                    calibration.instantaneous_error,
                    instantaneous,
                    instantaneous / calibration.instantaneous_error,
                    points(&calibration.accumulated, &calibration.accumulated_errors, 0.1),
                    points(&calibration.accumulated, &calibration.accumulated_errors, 0.25),
                    points(&calibration.instantaneous, &calibration.instantaneous_errors, 0.1),
                    points(&calibration.instantaneous, &calibration.instantaneous_errors, 0.25),
                );
                let terms = terms(calibration);
                if std::env::var("CAL_TERMS").is_ok() {
                    eprintln!(
                        "{:>22}| {}",
                        "",
                        TERMS
                            .iter()
                            .zip(terms)
                            .map(|(name, value)| format!("{name} {value:.2e}"))
                            .collect::<Vec<_>>()
                            .join(", ")
                    );
                }
                if calibration.norm.sqrt() / largest_norm > INSIDE {
                    let index = edges.iter().position(|edge| *edge == level.edge).unwrap();
                    inside[index].push(terms);
                }
            },
        )
        .unwrap();
        // The median of each over the readings inside, per level, and the
        // rate it falls at between levels: `log(q₁/q₂)/log(h₁/h₂)`.
        let medians = inside
            .iter()
            .map(|readings| {
                (0..TERMS.len())
                    .map(|term| {
                        let mut values = readings
                            .iter()
                            .map(|reading: &[f64; 12]| reading[term])
                            .collect::<Vec<_>>();
                        values.sort_by(f64::total_cmp);
                        values[values.len() / 2]
                    })
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        eprintln!("{start:?}, the carried state's medians inside and the rates between levels:");
        for (term, name) in TERMS.iter().enumerate() {
            let values = medians
                .iter()
                .map(|median| format!("{:.3e}", median[term]))
                .collect::<Vec<_>>()
                .join(" ");
            let rates = (1..edges.len())
                .map(|level| {
                    format!(
                        "{:.2}",
                        (medians[level - 1][term] / medians[level][term]).ln()
                            / (edges[level - 1] / edges[level]).ln()
                    )
                })
                .collect::<Vec<_>>()
                .join(" ");
            eprintln!("  {name:>9}: {values} | rates {rates}");
        }
        eprintln!("{start:?} in {:.1} s", started_at.elapsed().as_secs_f64());
    }
}

/// The default box with reflecting walls: nothing leaves and nothing is
/// lost, so whatever its energy and its primary total do the run does.
fn cavity_document() -> Result<TopologyDocument, String> {
    let mut document = box_document(PhysicsModel::Mechanical)?;
    let walls = OuterBoundaryConditions::uniform(OuterBoundaryCondition::Reflecting);
    document.model.draft.outer_boundaries = walls;
    document.model.accepted.outer_boundaries = walls;
    Ok(document)
}

/// One of AMR-03's long runs.
struct LongCase {
    name: &'static str,
    document: TopologyDocument,
    start: Start,
    /// What happens to it, and when.
    events: Vec<(f64, Event)>,
    speed: Option<f64>,
    duration: f64,
    reference_until: f64,
}

/// A ring like [`RING`], across the box from it.
const OTHER_RING: Pulse = Pulse {
    centre: Point2 { x: 0.35, y: 0.3 },
    width: 0.06,
    height: 1.0,
};

/// One experiment after another, as the app is played with: the ring struck
/// every `period` seconds until `duration`, by turns across the box and
/// where it began.
fn strikes(period: f64, duration: f64) -> Vec<(f64, Event)> {
    (1..)
        .map(|index| index as f64 * period)
        .take_while(|time| *time < duration)
        .zip([OTHER_RING, RING].into_iter().cycle())
        .map(|(time, pulse)| (time, Event::Strike(pulse)))
        .collect()
}

/// AMR-03's cases, at their full length: the packet in the reflecting box,
/// one experiment after another in the outgoing box, a driven point source
/// in it, the source's frequency halved once its mesh has settled (AMR-04),
/// and the parametric fiber amplifier from the gallery at its own speed,
/// from rest. The ring alone in the reflecting box never adapts: its
/// estimate reads 9.8% at most, inside the 12% target, and the field fills
/// the box, which it holds at the authored size.
fn long_cases() -> Result<Vec<LongCase>, String> {
    let mut source = box_document(PhysicsModel::Mechanical)?;
    source.model.source.enabled = true;
    let fiber = funfern_app::topology_examples::catalog()
        .iter()
        .find(|example| example.name == "Parametric fiber amplifier")
        .ok_or("the gallery has no parametric fiber amplifier")?
        .document
        .clone();
    Ok(vec![
        LongCase {
            name: "cavity-packet",
            document: cavity_document()?,
            start: Start::Packet,
            events: Vec::new(),
            speed: Some(1.0),
            duration: 60.0,
            reference_until: 10.0,
        },
        LongCase {
            name: "strikes",
            document: box_document(PhysicsModel::Mechanical)?,
            start: Start::Ring,
            events: strikes(8.0, 60.0),
            speed: Some(1.0),
            duration: 60.0,
            reference_until: 10.0,
        },
        LongCase {
            name: "source",
            document: source.clone(),
            start: Start::Rest,
            events: Vec::new(),
            speed: Some(1.0),
            duration: 30.0,
            reference_until: 10.0,
        },
        LongCase {
            name: "source-halved",
            document: source,
            start: Start::Rest,
            events: vec![(15.0, Event::Frequency(0.5))],
            speed: Some(1.0),
            duration: 90.0,
            reference_until: 0.0,
        },
        LongCase {
            name: "fiber",
            document: fiber,
            start: Start::Rest,
            events: Vec::new(),
            speed: None,
            duration: 60.0,
            reference_until: 0.0,
        },
    ])
}

/// One experiment of a long run: from its strike to the next.
#[derive(Debug)]
struct Experiment {
    start: f64,
    end: f64,
    /// The most triangles the mesh held, and how many it came down to.
    peak: usize,
    ends: usize,
    /// What the handoffs took, as a share of the experiment's peak energy:
    /// a share of a field that has drained to nothing is nothing.
    lost: f64,
    /// Refinements after it had begun coarsening.
    returns: usize,
    /// Transactions that changed nothing.
    empty: usize,
}

/// `readings`' experiments: the first from the start, then one from each of
/// `events` before `duration`.
fn experiments(readings: &[Reading], events: &[(f64, Event)], duration: f64) -> Vec<Experiment> {
    let starts = std::iter::once(0.0)
        .chain(events.iter().map(|(time, _)| *time))
        .filter(|start| *start < duration)
        .collect::<Vec<_>>();
    starts
        .iter()
        .enumerate()
        .filter_map(|(index, start)| {
            let end = starts.get(index + 1).copied().unwrap_or(duration);
            let experiment = readings
                .iter()
                .filter(|reading| *start <= reading.time && reading.time < end)
                .collect::<Vec<_>>();
            let mut coarsening = false;
            let mut returns = 0;
            for reading in &experiment {
                match reading.decided {
                    "coarsen" => coarsening = true,
                    "refine" if coarsening => returns += 1,
                    _ => {}
                }
            }
            Some(Experiment {
                start: *start,
                end,
                peak: experiment.iter().map(|reading| reading.after).max()?,
                ends: experiment.last()?.after,
                lost: experiment
                    .iter()
                    .filter(|reading| reading.changes > 0)
                    .map(|reading| -reading.handoff * reading.energy)
                    .sum(),
                returns,
                empty: experiment
                    .iter()
                    .filter(|reading| reading.decided == "unchanged")
                    .count(),
            })
        })
        .collect()
}

/// `readings` past half of `duration`, where a long run has settled.
fn second_half(readings: &[Reading], duration: f64) -> Vec<&Reading> {
    readings
        .iter()
        .filter(|reading| reading.time > 0.5 * duration)
        .collect()
}

/// How often a transaction in `readings` changed the mesh the other way from
/// the last one that changed it.
fn reversals(readings: &[&Reading]) -> usize {
    let mut reversals = 0;
    let mut direction = 0i64;
    for reading in readings {
        let change = reading.after as i64 - reading.triangles as i64;
        if change != 0 {
            if direction != 0 && change.signum() != direction {
                reversals += 1;
            }
            direction = change.signum();
        }
    }
    reversals
}

/// Whether AMR-03's tests run their cases at full length against the fine
/// reference, as the nightly job does, rather than the gate's shorter runs
/// without one.
fn long_nightly() -> bool {
    std::env::var_os("FUNFERN_AMR_LONG").is_some()
}

/// AMR-03's case `name` and how long it ran: `gate` seconds in the gate with
/// the adapted run alone, its full length beside the fine and the coarse run
/// with `FUNFERN_AMR_LONG=1`.
fn long_run(name: &str, gate: f64) -> (LongCase, f64, Looped) {
    let case = long_cases()
        .unwrap()
        .into_iter()
        .find(|case| case.name == name)
        .unwrap();
    let (duration, reference_until) = if long_nightly() {
        (case.duration, case.reference_until)
    } else {
        (gate, 0.0)
    };
    let looped = looped_in(
        case.document.clone(),
        case.start,
        &case.events,
        case.speed,
        duration,
        reference_until,
        reference_until,
    )
    .unwrap_or_else(|error| panic!("{name}: {error}"));
    (case, duration, looped)
}

/// Transactions that changed nothing, per second, over `readings`' second
/// half of `duration`.
fn empty_rate(readings: &[Reading], duration: f64) -> f64 {
    let settled = second_half(readings, duration);
    let empty = settled
        .iter()
        .filter(|reading| reading.decided == "unchanged")
        .count();
    empty as f64 / (0.5 * duration)
}

/// AMR-03: one experiment after another, a ring struck every 8 s by turns
/// across the box and where it began, as the app is played with. Each
/// refines, to 4,247-5,410 triangles from the authored 2,772, and comes back
/// down to 0.67-0.72 of its peak before the next; no later one needs more
/// than 1.13 times the first; none refines again once it has begun to
/// coarsen; no coarsening comes back empty; and the handoffs take 1.6e-4 to
/// 4.8e-3 of an experiment's peak energy. 24 s, three experiments, in the
/// gate; 60 s, eight, nightly.
#[test]
fn one_experiment_after_another_refines_and_comes_back_down() {
    let (case, duration, looped) = long_run("strikes", 24.0);
    let experiments = experiments(&looped.readings, &case.events, duration);
    let period = case.events[0].0;
    let first = experiments[0].peak;
    for experiment in &experiments {
        let label = format!("{experiment:?}");
        assert!(
            experiment.peak as f64 >= 1.3 * looped.coarse_triangles as f64,
            "never refined from the authored {}: {label}",
            looped.coarse_triangles
        );
        if experiment.end - experiment.start >= period - 1.0e-9 {
            assert!(
                experiment.ends as f64 <= 0.8 * experiment.peak as f64,
                "never came back down: {label}"
            );
        }
        assert!(
            experiment.peak as f64 <= 1.25 * first as f64,
            "needed more than the first's {first}: {label}"
        );
        assert_eq!(experiment.returns, 0, "{label}");
        assert_eq!(experiment.empty, 0, "{label}");
        assert!(experiment.lost <= 1.0e-2, "{label}");
    }
    for reading in &looped.readings {
        assert_eq!(reading.exposed, 0, "at {:.3} s: {reading:?}", reading.time);
        if reading.changes > 0 && reading.energy >= AMR_ERROR_FLOOR_RATIO {
            assert!(
                (-1.0e-2..=1.0e-6).contains(&reading.handoff),
                "at {:.3} s a handoff changed the energy by {:.2e}",
                reading.time,
                reading.handoff
            );
        }
    }
}

/// AMR-03: a driven point source settles its mesh in 3 s, and the settled
/// mesh stops asking for a coarsening it cannot make. The estimate counts an
/// element by its shortest edge, each collapse the transaction finds would
/// leave an edge past its target, and before the controller remembered the
/// mesher's refusals it asked every second estimate, 0.67 times a second at
/// 14 ms of work each, forever; the parametric fiber 0.63 at 32 ms.
/// Remembered, the source asks 0.07 times a second over its second half as
/// a borderline edge comes into the candidates, none in the gate's 20 s,
/// and the fiber none past 10.5 s. Nightly, 30 s and the fiber's 60 s.
///
/// AMR-04: with the source's frequency halved at 15 s the mesh refined for
/// it keeps 450 to 500 candidates the mesher refuses, shifting with the
/// field, which the refusals remembered never held: 0.58 a second from 21 s
/// to 45 s. Each refusal in a row now waits twice as long as the last, and
/// over the second half of 90 s, nightly, it asks 0.07 times a second while
/// the mesher still makes the few collapses it can, five in four
/// coarsenings.
#[test]
fn a_settled_mesh_stops_asking_for_a_coarsening_it_cannot_make() {
    let names: &[&str] = if long_nightly() {
        &["source", "fiber", "source-halved"]
    } else {
        &["source"]
    };
    for name in names {
        let (_, duration, looped) = long_run(name, 20.0);
        let rate = empty_rate(&looped.readings, duration);
        assert!(
            rate <= 0.1,
            "{name} asks for an empty coarsening {rate:.2} times a second over its second half"
        );
        assert_eq!(
            reversals(&second_half(&looped.readings, duration)),
            0,
            "{name}"
        );
    }
}

/// AMR-03: the packet in the reflecting box, refined once as it starts and
/// coarsened back as it spreads and fades, keeps its primary total: nothing
/// leaves the box, and no handoff may take any. 32 s in the gate, past its
/// first coarsening at 29.3 s; 60 s nightly, from 3,384 triangles at its
/// peak to 2,878. The grid filter is all that drains the box: at the
/// scalar wave's strength, which the harness filtered at until 2026-11-09,
/// the first coarsening came at 20 s and the nightly ended at 2,874.
#[test]
fn a_packet_in_a_reflecting_box_keeps_its_total_as_the_mesh_follows_it() {
    let (_, duration, looped) = long_run("cavity-packet", 32.0);
    let readings = &looped.readings;
    let first = readings[0].total;
    for reading in readings {
        assert!(
            (reading.total - first).abs() <= 1.0e-9 * first.abs(),
            "at {:.3} s the total reads {:.9e} against {first:.9e}",
            reading.time,
            reading.total
        );
        assert_eq!(reading.exposed, 0, "at {:.3} s: {reading:?}", reading.time);
    }
    assert!(
        readings.iter().any(|reading| reading.decided == "coarsen"),
        "the packet's mesh never coarsened"
    );
    assert_eq!(reversals(&second_half(readings, duration)), 0);
}

/// AMR-03's study: each long case adapted as the app adapts it, every
/// estimate's reading printed, and over its second half how the mesh moved,
/// what the transactions did and what the run kept.
#[test]
#[ignore = "a study: cargo test --release --test adaptation study_long -- --ignored --nocapture"]
fn study_long_adaptation() {
    let wanted = std::env::var("LONG_CASE").ok();
    // A study's knobs: another accuracy target, a shorter run.
    let variable = |name: &str| {
        std::env::var(name)
            .ok()
            .and_then(|value| value.parse::<f64>().ok())
    };
    for case in long_cases().unwrap() {
        if wanted.as_deref().is_some_and(|wanted| wanted != case.name) {
            continue;
        }
        let started_at = std::time::Instant::now();
        let mut document = case.document;
        if let Some(accuracy) = variable("AMR_ACCURACY") {
            document.presentation.adaptation.accuracy_percent = accuracy;
        }
        let duration = variable("LONG_DURATION").unwrap_or(case.duration);
        let looped = match looped_in(
            document,
            case.start,
            &case.events,
            case.speed,
            duration,
            case.reference_until.min(duration),
            duration,
        ) {
            Ok(looped) => looped,
            Err(error) => {
                eprintln!("{}: FAILED {error}", case.name);
                continue;
            }
        };
        eprintln!(
            "{}: fine {} triangles, coarse {}, in {:.1} s",
            case.name,
            looped.fine_triangles,
            looped.coarse_triangles,
            started_at.elapsed().as_secs_f64()
        );
        eprintln!(
            "{:>7} {:>6} {:>9} {:>9} {:>5} {:>9} {:>9} {:>8} {:>8} {:>10} {:>10} {:>7} {:>7} {:>7} {:>4} {:>5}",
            "t",
            "tris",
            "estimate",
            "decided",
            "chg",
            "handoff",
            "carried",
            "energy",
            "coarse",
            "total",
            "c total",
            "adapt",
            "c err",
            "job ms",
            "exp",
            "past"
        );
        let mut carried = 1.0;
        for reading in &looped.readings {
            if reading.changes > 0 {
                carried *= 1.0 + reading.handoff;
            }
            eprintln!(
                "{:>7.3} {:>6} {:>8.4}{} {:>9} {:>5} {:>9.2e} {:>9.6} {:>8.2e} {:>8.2e} {:>10.3e} {:>10.3e} {:>7.4} {:>7.4} {:>7.1} {:>4} {:>5}",
                reading.time,
                reading.triangles,
                reading.estimate,
                if reading.dormant { "d" } else { " " },
                reading.decided,
                reading.changes,
                reading.handoff,
                carried,
                reading.energy,
                reading.coarse_energy,
                reading.total,
                reading.coarse_total,
                reading.adaptive,
                reading.coarse,
                reading.job_ms,
                reading.exposed,
                reading.coarser
            );
        }
        let settled = second_half(&looped.readings, duration);
        if settled.is_empty() {
            continue;
        }
        let triangles = settled
            .iter()
            .map(|reading| reading.after)
            .collect::<Vec<_>>();
        let (least, most) = (
            *triangles.iter().min().unwrap(),
            *triangles.iter().max().unwrap(),
        );
        let mean = triangles.iter().sum::<usize>() as f64 / triangles.len() as f64;
        let reversals = reversals(&settled);
        let kinds = ["refine", "coarsen", "unchanged"];
        let span = duration - settled[0].time;
        let summary = kinds
            .iter()
            .map(|kind| {
                let of_kind = settled
                    .iter()
                    .filter(|reading| reading.decided == *kind)
                    .collect::<Vec<_>>();
                let cost = if of_kind.is_empty() {
                    0.0
                } else {
                    of_kind.iter().map(|reading| reading.job_ms).sum::<f64>() / of_kind.len() as f64
                };
                format!(
                    "{kind} {} ({:.2}/s, {cost:.0} ms each)",
                    of_kind.len(),
                    of_kind.len() as f64 / span
                )
            })
            .collect::<Vec<_>>()
            .join(", ");
        for (index, experiment) in experiments(&looped.readings, &case.events, duration)
            .iter()
            .enumerate()
        {
            eprintln!(
                "{} experiment {index} from {:.1} s: peak {}, ends at {}, the handoffs took {:.2e} of its peak, refined again {} times after coarsening, {} empty",
                case.name,
                experiment.start,
                experiment.peak,
                experiment.ends,
                experiment.lost,
                experiment.returns,
                experiment.empty
            );
        }
        let first = &looped.readings[0];
        let last = looped.readings.last().unwrap();
        eprintln!(
            "{} over its second half: {least} to {most} triangles, mean {mean:.0}, {reversals} reversals; {summary}",
            case.name
        );
        eprintln!(
            "{} at the end: the handoffs kept {carried:.6} of the energy; the adapted run's energy over its peak {:.3e}, the coarse run's {:.3e}; the totals from {:.6e} to {:.6e} adapted and {:.6e} coarse; exposed at most {}",
            case.name,
            last.energy,
            last.coarse_energy,
            first.total,
            last.total,
            last.coarse_total,
            looped
                .readings
                .iter()
                .map(|reading| reading.exposed)
                .max()
                .unwrap_or(0)
        );
    }
}
