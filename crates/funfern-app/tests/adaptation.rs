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
    AMR_ESTIMATE_SECONDS, AMR_ESTIMATE_STEPS, AMR_FIELD_PRESENCE_RATIO, AdaptationController,
    AdaptationStep, AmrDecision, IndicatorSnapshots, adaptation_decision, target_accuracy,
};
use funfern_app::document::PresentationSettings;
use funfern_app::topology_editor::{TopologyDocument, TopologyEditor};
use funfern_app::topology_runtime::{PreparedCanonicalTransfer, PreparedTopology};
use funfern_core::{
    CanonicalPointStencil, CanonicalWaveState, DomainRect, ElectromagneticPolarization,
    GRID_SCALE_FILTER_CADENCE, PhysicsModel, Point2, SolutionIndicatorResult, TriMesh,
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
}

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
    let mut document = box_document(physics)?;
    // The speed ceiling the app runs this run's speed under, which bounds
    // how far the field gets between estimates.
    document.presentation.simulation_speed = speed;
    let presentation = &document.presentation;
    let settings = presentation.adaptation;
    let domain = document.model.accepted.geometry.domain;
    let (_, _, fine) = prepare_at(&document, settings.minimum_edge)?;
    let (editor, mut runtime, coarse) = prepare(&document)?;
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
    // The lattice's mean density at its largest, the reference's peak energy
    // spread evenly over the box, in the lattice's units.
    let mut largest_mean: f64 = 0.0;
    let mut readings = Vec::new();
    // The app filters the grid scale every few steps, as a new document has
    // it, and estimates no pair of states a filter stands between.
    let filtered = std::env::var("AMR_UNFILTERED").is_err();
    for step in 1..=total {
        let referenced = step as f64 * time_step <= reference_until;
        let boundary = filtered && step.is_multiple_of(GRID_SCALE_FILTER_CADENCE);
        let due = !boundary && last.is_none_or(|last| step >= last + gap);
        let fine_previous = (due && diagnose && referenced).then(|| fine_run.clone());
        let coarse_previous = (due && diagnose && referenced).then(|| coarse_run.clone());
        if referenced {
            fine_run.step(&fine)?;
            coarse_run.step(&coarse)?;
        }
        // The state a step earlier on this generation: a handoff falls
        // between steps, so the generation estimated has stepped since its own.
        let previous = run.clone();
        run.step(&prepared)?;
        if boundary {
            if referenced {
                fine_run.filter(&fine)?;
                coarse_run.filter(&coarse)?;
            }
            run.filter(&prepared)?;
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
            largest_mean = largest_mean.max(norm * norm / points.len() as f64);
        }
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
        let result = estimated(&mut controller, &prepared, &run, &previous, presentation)?;
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
            changes: 0,
            after: prepared.mesh.triangles.len(),
            handoff: 0.0,
            in_band: f64::NAN,
            reach: result.report.field_reach,
            exposed: 0,
            coarser: 0,
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
            AdaptationStep::Adapt(job) => Some(job),
        };
        if let Some(job) = &mut job {
            let result = loop {
                if let Some(result) = job.advance(1 << 16) {
                    break result.map_err(|error| error.to_string())?;
                }
            };
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
                runtime.request_adapted(editor.revision, &editor.document, mesh)?;
                let (target, maps) = finish_with_maps(&mut runtime)?;
                controller.published(&target);
                if target.recommended_time_step() < time_step {
                    return Err(format!(
                        "at {:.2} s an adapted mesh allows {:.3e}, under the step {time_step:.3e}",
                        reading.time,
                        target.recommended_time_step()
                    ));
                }
                let before = run.energy(&prepared)?;
                run = handed((&prepared, &run), &target, &maps, time_step)?;
                // The app takes away the stationary flux the carry left once
                // it has solved for it, which on a lossless generation comes
                // to taking it away here.
                if std::env::var("AMR_UNCLEANED").is_err() {
                    reading.cleaned = cleaned(&mut run, &target)?;
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
    let (editor, mut runtime, coarse) = prepare(document)?;
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
    runtime.request_adapted(
        editor.revision,
        &editor.document,
        mesh.ok_or("the refinement changed nothing")?,
    )?;
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
/// 2026-11-04 (the box in the Mechanical skin, its three skins bit-identical
/// there):
///
/// - while the field is inside, the adapted run is never less accurate than
///   the coarse one; the packet, which refines, at least 5% more accurate
///   after its first estimate (10% at a tenth of the speed the closest). The
///   ring is not refined while inside: its estimate reads 0.09 against the
///   0.12 target (AMR-02);
/// - no element past the authored size lies where the field gets before the
///   next transaction;
/// - the mesh never holds a third of the fine mesh's triangles (12,902 of
///   45,203 the most, the ring at a tenth, chasing what its walls send back);
/// - the field goes dormant, and nothing refines after;
/// - it coarsens back, to at most nine tenths of its peak (0.82 at
///   full speed, 0.35 and less slower), and below full speed quiet
///   regions grow past the authored mesh;
/// - a handoff while the field lives takes at most 1% of its energy (0.7%
///   the most) and adds at most 1e-4 (4.6e-6).
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
    for (index, reading) in readings.iter().enumerate() {
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
            if start == Start::Packet && index > 0 {
                assert!(
                    reading.adaptive <= 0.95 * reading.coarse,
                    "{case} at {:.3} s: the refined packet reads {:.4} against {:.4}",
                    reading.time,
                    reading.adaptive,
                    reading.coarse
                );
            }
        }
        if reading.changes > 0 && !reading.dormant {
            assert!(
                (-1.0e-2..=1.0e-4).contains(&reading.handoff),
                "{case} at {:.3} s: a handoff changed the energy by {:.2e}",
                reading.time,
                reading.handoff
            );
        }
    }
    let peak = readings.iter().map(|reading| reading.after).max().unwrap();
    assert!(
        3 * peak < looped.fine_triangles,
        "{case}: the mesh peaked at {peak} triangles against the fine mesh's {}",
        looped.fine_triangles
    );
    let dormant = readings
        .iter()
        .position(|reading| reading.dormant)
        .unwrap_or_else(|| panic!("{case} never goes dormant"));
    for reading in &readings[dormant..] {
        assert_ne!(
            reading.decided, "refine",
            "{case} refines at {:.3} s, dormant since {:.3} s",
            reading.time, readings[dormant].time
        );
    }
    let last = readings.last().unwrap();
    assert!(
        last.after as f64 <= 0.9 * peak as f64,
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

/// A packet at a quarter of the speed ceiling, adapted as the app adapts it.
/// Its first estimate refines around it, and once it has left that patch the
/// estimate reads well under target and asks to coarsen. Judged by where the
/// field was then, the region just ahead of it read as quiet and coarsened
/// to twice the authored edge; the packet crossing it scattered a few
/// percent of its energy back, and by t = 1.13 the adapted run was the less
/// accurate, 0.154 against the coarse run's 0.135, and 0.234 against 0.167 at
/// 1.5. Held at the authored size within the field's reach, no element
/// coarser than the authored one lies where the field gets before the next
/// transaction, and while the packet is inside the adapted run is the more
/// accurate at every estimate, 0.152 against 0.167 at 1.5 the closest.
#[test]
fn a_slow_packet_never_meets_a_mesh_coarsened_ahead_of_it() {
    let looped = looped(PhysicsModel::Mechanical, Start::Packet, 0.25, 1.6, 1.6).unwrap();
    assert!(
        looped
            .readings
            .iter()
            .any(|reading| reading.decided == "coarsen"),
        "the packet's patch coarsens behind it"
    );
    for reading in &looped.readings {
        assert_eq!(reading.exposed, 0, "at {:.3} s: {reading:?}", reading.time);
        if reading.norm > 0.5 {
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
