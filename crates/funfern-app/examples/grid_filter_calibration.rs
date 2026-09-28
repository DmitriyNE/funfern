//! What the grid filter costs and what it clears, on gallery scenes stepped by
//! the CPU reference with the core's own filter every
//! `GRID_SCALE_FILTER_CADENCE` steps. The device runs the same arithmetic, so
//! these are the app's rates.
//!
//! Three measurements per scene, at the scene's own mesh edge:
//!
//! - `transfer`: one filter application on a plane wave `cos(k·x)` at 3 to 16
//!   nodes per wavelength, as the energy e-folding rate per second it gives.
//!   The field-scale end is what the filter exists to clear; the resolved end
//!   is what it costs.
//! - `residue`: a pulse one element wide released with every wall
//!   second-order outgoing. What travels leaves; what is left is the
//!   element-scale residue a sharp event leaves behind. The energy remaining
//!   at 1 to 16 s, filtered and not.
//! - `retention`: a plane wave at 8, 12 and 16 nodes per wavelength inside
//!   reflecting walls, the energy it keeps over 32 s filtered against
//!   unfiltered. Nothing else takes energy from a closed lossless scene, so
//!   the ratio is the filter's cost to a resolved wave. A scene with pins is
//!   not a clean cavity: the seed jumps to the pin's data within an element,
//!   and the ratio also counts what that jump makes, so the line says how
//!   many nodes are pinned.
//!
//! A node here is a P2 node, at half the edge. Pins keep their data; sources
//! are left off, so only the seeded field moves.
//!
//! `CALIBRATION_SCENES` lists catalogue names separated by commas (default:
//! five that span the gallery's spread of row bounds), `CALIBRATION_PARTS`
//! the measurements (default all three), `CALIBRATION_STRENGTH` the filter
//! strength (default the resident filter's).

use std::sync::Arc;

use funfern_app::topology_editor::{TopologyDocument, TopologyEditor};
use funfern_app::topology_examples::catalog;
use funfern_app::topology_runtime::{PreparedTopology, TopologyRuntime};
use funfern_core::{
    CANONICAL_GRID_FILTER_STRENGTH, CanonicalForcing, CanonicalWaveState,
    GRID_SCALE_FILTER_CADENCE, MeshingOptions, OuterBoundaryCondition, OuterBoundaryConditions,
    Point2,
};

const DEFAULT_SCENES: [&str; 5] = [
    "Obstacle over a mirror",
    "Phased array",
    "Material lens",
    "Ring resonator",
    "Dielectric whispering gallery",
];

fn prepare(document: &TopologyDocument, walls: OuterBoundaryCondition) -> Arc<PreparedTopology> {
    let mut document = document.clone();
    for scene in [&mut document.model.draft, &mut document.model.accepted] {
        scene.outer_boundaries = OuterBoundaryConditions::uniform(walls);
    }
    let editor = TopologyEditor::from_document(document).unwrap();
    let mut runtime = TopologyRuntime::default();
    let token = runtime
        .request(
            editor.revision,
            &editor.document,
            editor.compiled_accepted.clone(),
            MeshingOptions {
                target_edge_length: editor.document.presentation.mesh_edge,
                ..MeshingOptions::default()
            },
            true,
        )
        .unwrap();
    loop {
        if let Some(result) = runtime.advance(1 << 16) {
            result.unwrap();
            return runtime.commit_ready(token).unwrap();
        }
    }
}

fn plane_wave(prepared: &PreparedTopology, nodes_per_wavelength: f64) -> Vec<f64> {
    let operator = &prepared.canonical_operator;
    let spacing = 0.5 * prepared_edge(prepared);
    let k = std::f64::consts::TAU / (nodes_per_wavelength * spacing);
    let (x, y) = (0.3_f64.cos(), 0.3_f64.sin());
    operator
        .node_points()
        .iter()
        .zip(operator.primary_mass())
        .map(|(point, mass)| mass * (k * (point.x * x + point.y * y)).cos())
        .collect()
}

fn prepared_edge(prepared: &PreparedTopology) -> f64 {
    prepared.meshing.target_edge_length
}

/// Steps `seed` for `seconds`, and returns the energy at each of `marks`
/// against the start.
fn run(
    prepared: &PreparedTopology,
    seed: Vec<f64>,
    seconds: f64,
    strength: Option<f64>,
    marks: &[f64],
) -> Vec<f64> {
    let operator = &prepared.canonical_operator;
    let forcing = CanonicalForcing::from_prescribed(
        operator,
        prepared.canonical_forcing.prescribed().to_vec(),
    )
    .unwrap();
    let dt = operator.recommended_time_step();
    let mut state = CanonicalWaveState::new(
        operator,
        dt,
        seed,
        vec![Point2::new(0.0, 0.0); operator.complementary_degrees_of_freedom()],
    )
    .unwrap();
    let start = state.energy(operator).unwrap();
    let mut energies = Vec::with_capacity(marks.len());
    let steps = (seconds / dt).ceil() as u64;
    for step in 1..=steps {
        state.step_with_forcing(operator, &forcing).unwrap();
        if let Some(strength) = strength
            && step.is_multiple_of(GRID_SCALE_FILTER_CADENCE)
        {
            state
                .apply_grid_filter(operator, &forcing, strength)
                .unwrap();
        }
        while energies.len() < marks.len() && step as f64 * dt >= marks[energies.len()] {
            energies.push(state.energy(operator).unwrap() / start);
        }
    }
    energies
}

fn transfer(name: &str, prepared: &PreparedTopology, strength: f64) {
    let operator = &prepared.canonical_operator;
    let forcing = CanonicalForcing::none(operator);
    let dt = operator.recommended_time_step();
    let mut line = format!("{name:<24} transfer, e-fold rate /s by nodes per wavelength:");
    for npw in [3.0, 4.0, 6.0, 8.0, 12.0, 16.0] {
        let mut state = CanonicalWaveState::new(
            operator,
            dt,
            plane_wave(prepared, npw),
            vec![Point2::new(0.0, 0.0); operator.complementary_degrees_of_freedom()],
        )
        .unwrap();
        let before = state.energy(operator).unwrap();
        state
            .apply_grid_filter(operator, &forcing, strength)
            .unwrap();
        let after = state.energy(operator).unwrap();
        let rate = -(after / before).ln() / (GRID_SCALE_FILTER_CADENCE as f64 * dt);
        line.push_str(&format!(" {npw}: {rate:.2e}"));
    }
    println!("{line}");
}

fn residue(name: &str, document: &TopologyDocument, strength: f64) {
    let prepared = prepare(document, OuterBoundaryCondition::SecondOrderOutgoing);
    let operator = &prepared.canonical_operator;
    let centre = document.model.source.position;
    let width = prepared_edge(&prepared);
    let seed = operator
        .node_points()
        .iter()
        .zip(operator.primary_mass())
        .map(|(point, mass)| mass * (-((*point - centre).norm() / width).powi(2)).exp())
        .collect::<Vec<_>>();
    let marks = [1.0, 2.0, 4.0, 8.0, 16.0];
    let (filtered, unfiltered) = std::thread::scope(|scope| {
        let filtered = scope.spawn(|| run(&prepared, seed.clone(), 16.0, Some(strength), &marks));
        let unfiltered = scope.spawn(|| run(&prepared, seed.clone(), 16.0, None, &marks));
        (filtered.join().unwrap(), unfiltered.join().unwrap())
    });
    let mut line = format!("{name:<24} residue, energy left filtered/unfiltered at s:");
    for ((mark, filtered), unfiltered) in marks.iter().zip(filtered).zip(unfiltered) {
        line.push_str(&format!(" {mark}: {filtered:.2e}/{unfiltered:.2e}"));
    }
    println!("{line}");
}

fn retention(name: &str, document: &TopologyDocument, strength: f64) {
    let prepared = prepare(document, OuterBoundaryCondition::Reflecting);
    let marks = [8.0, 32.0];
    let results = std::thread::scope(|scope| {
        [8.0, 12.0, 16.0]
            .map(|npw| {
                let seed = plane_wave(&prepared, npw);
                let prepared = &prepared;
                let filtered = {
                    let seed = seed.clone();
                    scope.spawn(move || run(prepared, seed, 32.0, Some(strength), &marks))
                };
                let unfiltered = scope.spawn(move || run(prepared, seed, 32.0, None, &marks));
                (npw, filtered, unfiltered)
            })
            .map(|(npw, filtered, unfiltered)| {
                (npw, filtered.join().unwrap(), unfiltered.join().unwrap())
            })
    });
    let pinned = prepared
        .canonical_forcing
        .prescribed()
        .iter()
        .filter(|signal| signal.is_some())
        .count();
    let mut line = format!(
        "{name:<24} retention ({pinned} pinned), energy kept filtered/unfiltered at 8 s, 32 s:"
    );
    for (npw, filtered, unfiltered) in results {
        line.push_str(&format!(
            " {npw}: {:.4}, {:.4}",
            filtered[0] / unfiltered[0],
            filtered[1] / unfiltered[1]
        ));
    }
    println!("{line}");
}

fn main() {
    let scenes = std::env::var("CALIBRATION_SCENES").ok();
    let scenes = scenes.as_deref().map_or_else(
        || DEFAULT_SCENES.to_vec(),
        |names| names.split(',').map(str::trim).collect(),
    );
    let parts = std::env::var("CALIBRATION_PARTS")
        .unwrap_or_else(|_| "transfer,residue,retention".to_owned());
    let strength = std::env::var("CALIBRATION_STRENGTH")
        .ok()
        .map_or(CANONICAL_GRID_FILTER_STRENGTH, |value| {
            value.parse().unwrap()
        });
    println!("grid filter at strength {strength}, every {GRID_SCALE_FILTER_CADENCE} steps");
    for name in scenes {
        let example = catalog()
            .iter()
            .find(|example| example.name == name)
            .unwrap_or_else(|| panic!("no catalogue scene named {name}"));
        let document = &example.document;
        if parts.contains("transfer") {
            let prepared = prepare(document, OuterBoundaryCondition::SecondOrderOutgoing);
            transfer(name, &prepared, strength);
        }
        if parts.contains("residue") {
            residue(name, document, strength);
        }
        if parts.contains("retention") {
            retention(name, document, strength);
        }
    }
}
