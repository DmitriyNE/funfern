//! A complementary mode on the device: a state with Q zero and b in the
//! discrete kernel of the coupling, a curl-free flux that pushes no field,
//! the gradient of a potential that vanishes on every wall, `cos(πx/2)
//! cos(πy/2)` on a reflecting box, and with `--baffle`, on a box a
//! reflecting baffle divides from wall to wall, `sin(πx) cos(πy/2)` on the
//! left of the baffle alone, which vanishes on the baffle too: a tangential
//! flux along a wall is a force on it. Stepped on the device and in f64, the
//! demand on the fixed mesh is exact: Q stays at roundoff, b and the
//! complementary energy are kept, and nothing crosses the baffle.
//!
//! With `--remesh` the state is carried onto a mesh 1.4 times finer through
//! the maps the runtime prepares for the edit, as the app's handoff carries
//! it, and the demand is not invariance, since a vector in the old discrete
//! kernel does not land in the new one. The carry's budget is its own
//! discretization error, the carried flux against the mode sampled on the
//! new mesh, and within it the physical component is kept, the flux's first
//! moment, which no mesh owns; the coupling residual the
//! carried flux has in the new space, its compatible share, lies within it,
//! and what that residual pushes into Q over the following steps stays
//! within it too; and the component stays on its side of the baffle. The
//! budget falls with the mesh, at least threefold from the edge to half of
//! it. The device then steps the carried state against the f64 reference.
//!
//! `COMPLEMENTARY_EDGE` sets the mesh edge (default 0.16) and
//! `COMPLEMENTARY_STEPS` the steps (default 400). Exits 0 when every demand
//! holds.

#[path = "support/check.rs"]
mod check;
use bevy::{app::AppExit, prelude::*, render::storage::ShaderBuffer};
use check::within;
use funfern_app::canonical_gpu::{
    CanonicalGpuClock, CanonicalGpuDisplay, CanonicalGpuPlan, CanonicalGpuRequest,
    CanonicalGpuRuntimeTransfer, CanonicalWaveGpuPlugin,
};
use funfern_app::topology_editor::{
    OpenCurvePurpose, TopologyAcceptance, TopologyAttachment, TopologyDocument, TopologyEditor,
};
use funfern_app::topology_runtime::{PreparedCanonicalTransfer, PreparedTopology, TopologyRuntime};
use funfern_app::wave_gpu::WaveGpuPlugin;
use funfern_core::{
    CanonicalWaveOperator, CanonicalWaveState, FaceAnchor, MeshingOptions, OpenCubicSpline,
    OuterBoundaryCondition, OuterBoundaryConditions, OuterSide, Point2,
};
use std::sync::Arc;
use std::time::Instant;

/// The relative L2 error b may reach on the device against the f64
/// reference, as the end-to-end fixtures hold it.
const TOLERANCE: f64 = 5.0e-6;
/// What the f64 reference may push into Q, or lose of b's energy, or leak
/// across the baffle, as a share of the mode's energy: roundoff of a
/// stationary state stepped in f64.
const EXACT: f64 = 1.0e-10;
/// The same on the device, which steps in f32: a force of roundoff size
/// integrated over the run. Measured 2026-10-13; see the engineering log.
const DEVICE_EXACT: f64 = 1.0e-8;

fn edge() -> f64 {
    std::env::var("COMPLEMENTARY_EDGE")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(0.16)
}

fn steps() -> u64 {
    std::env::var("COMPLEMENTARY_STEPS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(400)
}

/// A curl-free flux, the gradient of a potential that vanishes on every
/// wall: it pushes no field, a stored field that never moves. On the box,
/// `cos(πx/2) cos(πy/2)`; on the baffled box, `sin(πx) cos(πy/2)` on the
/// left of the baffle and nothing on its right.
fn static_flux(point: Point2, baffle: bool) -> Point2 {
    let pi = std::f64::consts::PI;
    let half = 0.5 * pi;
    if !baffle {
        return Point2::new(
            -half * (half * point.x).sin() * (half * point.y).cos(),
            -half * (half * point.x).cos() * (half * point.y).sin(),
        );
    }
    if point.x >= 0.0 {
        return Point2::default();
    }
    Point2::new(
        pi * (pi * point.x).cos() * (half * point.y).cos(),
        -half * (pi * point.x).sin() * (half * point.y).sin(),
    )
}

/// The reflecting box of the default medium, no source, no grid filter,
/// adaptation off; with `baffle`, a reflecting baffle from the bottom side to
/// the top at x = 0, drawn as the editor draws one, dividing the box in two.
fn document(baffle: bool) -> TopologyDocument {
    let mut document = TopologyEditor::default().document;
    let walls = OuterBoundaryConditions::uniform(OuterBoundaryCondition::Reflecting);
    document.model.draft.outer_boundaries = walls;
    document.model.accepted.outer_boundaries = walls;
    document.model.source.enabled = false;
    document.presentation.mesh_edge = edge();
    document.presentation.adaptation.enabled = false;
    document.presentation.grid_scale_filter = false;
    if !baffle {
        return document;
    }
    let mut editor = TopologyEditor::from_document(document).expect("the box opens");
    settle(&mut editor);
    editor
        .create_open_curve(
            OpenCubicSpline::polyline(vec![Point2::new(0.0, -1.0), Point2::new(0.0, 1.0)])
                .expect("the baffle's line"),
            OpenCurvePurpose::BoundaryBaffle,
            Some(TopologyAttachment::Boundary(FaceAnchor::Outer {
                side: OuterSide::Bottom,
                fraction: 0.5,
            })),
            Some(TopologyAttachment::Boundary(FaceAnchor::Outer {
                side: OuterSide::Top,
                fraction: 0.5,
            })),
        )
        .expect("the baffle");
    settle(&mut editor);
    let mut document = editor.document;
    document.model.accepted = document.model.draft.clone();
    document
}

/// Runs the editor's validation to its verdict, which must be that the
/// draft is valid.
fn settle(editor: &mut TopologyEditor) {
    for _ in 0..100_000 {
        editor.validate_frame(64);
        if editor.acceptance != TopologyAcceptance::Pending {
            break;
        }
    }
    assert_eq!(
        editor.acceptance,
        TopologyAcceptance::Valid,
        "the draft is accepted"
    );
}

/// Prepares `document` at `edge` on `runtime`, as the app prepares a
/// generation; with `handoff`, as the candidate of an edit of the active
/// generation, whose maps are answered beside it.
fn prepare(
    runtime: &mut TopologyRuntime,
    document: &TopologyDocument,
    revision: u64,
    edge: f64,
    handoff: bool,
) -> (
    Arc<PreparedTopology>,
    Option<Arc<PreparedCanonicalTransfer>>,
) {
    let editor = TopologyEditor::from_document(document.clone()).expect("the document opens");
    let domain = document.model.accepted.geometry.domain;
    let options = MeshingOptions {
        target_edge_length: edge,
        curve_tolerance: (edge * 0.02).min(5e-4),
        ..MeshingOptions::default()
    }
    .sized_for_area(domain.width() * domain.height());
    let token = if handoff {
        runtime.request_with_handoff(
            revision,
            &editor.document,
            editor.compiled_accepted.clone(),
            options,
            false,
            true,
        )
    } else {
        runtime.request(
            revision,
            &editor.document,
            editor.compiled_accepted.clone(),
            options,
            true,
        )
    }
    .expect("the generation is requested");
    loop {
        if let Some(result) = runtime.advance(1 << 16) {
            result.expect("the generation prepares");
            break;
        }
    }
    let maps = runtime
        .ready()
        .and_then(|candidate| candidate.canonical_transfer.clone());
    let prepared = runtime.commit_ready(token).expect("the generation commits");
    (prepared, maps)
}

/// The mode on `operator`: the static flux sampled, less its compatible
/// part.
fn mode(operator: &CanonicalWaveOperator, time_step: f64, baffle: bool) -> CanonicalWaveState {
    let sampled = sampled_flux(operator, baffle);
    let stationary = operator
        .stationary_complementary_component(&sampled)
        .expect("the sampled flux less its compatible part is stationary");
    CanonicalWaveState::new(
        operator,
        time_step,
        vec![0.0; operator.degrees_of_freedom()],
        stationary,
    )
    .expect("the mode")
}

/// The static flux at the operator's samples.
fn sampled_flux(operator: &CanonicalWaveOperator, baffle: bool) -> Vec<Point2> {
    operator
        .constitutive_samples()
        .iter()
        .map(|sample| static_flux(sample.point, baffle))
        .collect()
}

/// The complementary energy of `flux`, on each side of x = 0.
fn side_energies(operator: &CanonicalWaveOperator, flux: &[Point2]) -> [f64; 2] {
    let mut sides = [0.0; 2];
    for (sample, flux) in operator.constitutive_samples().iter().zip(flux) {
        let energy =
            0.5 * sample.integration_weight * flux.dot(sample.complementary_inverse.apply(*flux));
        sides[usize::from(sample.point.x >= 0.0)] += energy;
    }
    sides
}

/// The energy a primary flux holds: `½ Q·u`, with `u` the field it makes.
fn pushed(operator: &CanonicalWaveOperator, primary: &[f64]) -> f64 {
    primary
        .iter()
        .zip(operator.primary_field(primary).expect("the field of Q"))
        .map(|(q, u)| 0.5 * q * u)
        .sum()
}

/// The share of a flux's energy its compatible part holds, the part that
/// pushes the field.
fn compatible_share(operator: &CanonicalWaveOperator, flux: &[Point2]) -> f64 {
    let stationary = operator
        .stationary_complementary_component(flux)
        .expect("the compatible part");
    let compatible = flux
        .iter()
        .zip(&stationary)
        .map(|(flux, stationary)| *flux - *stationary)
        .collect::<Vec<_>>();
    let energy = |flux: &[Point2]| side_energies(operator, flux).iter().sum::<f64>();
    energy(&compatible) / energy(flux)
}

/// The flux's first moment, `∫ x b_x dA`, a quantity no mesh owns: for the
/// gradient of a potential `φ` that vanishes on the walls it is `−∫ φ dA`,
/// `−(4/π)²` for the box's mode and `8/π²` for the baffled box's, where the
/// flux's plain integral is zero for any such gradient and says nothing.
fn integral(operator: &CanonicalWaveOperator, flux: &[Point2]) -> f64 {
    operator
        .constitutive_samples()
        .iter()
        .zip(flux)
        .map(|(sample, flux)| sample.point.x * flux.x * sample.integration_weight)
        .sum()
}

fn relative_l2(actual: impl Iterator<Item = f64>, expected: impl Iterator<Item = f64>) -> f64 {
    let (difference, scale) = actual.zip(expected).fold((0.0, 0.0), |sum, pair| {
        (sum.0 + (pair.0 - pair.1).powi(2), sum.1 + pair.1 * pair.1)
    });
    (difference / scale.max(1.0e-300)).sqrt()
}

/// What a state reads against the mode's energy: Q's energy, b's energy
/// change, and the right side's share.
struct Reading {
    pushed: f64,
    energy_change: f64,
    right: f64,
}

fn reading(
    operator: &CanonicalWaveOperator,
    primary: &[f64],
    complementary: &[Point2],
    mode_energy: f64,
) -> Reading {
    let sides = side_energies(operator, complementary);
    Reading {
        pushed: pushed(operator, primary) / mode_energy,
        energy_change: ((sides[0] + sides[1]) / mode_energy - 1.0).abs(),
        right: sides[1] / mode_energy,
    }
}

/// The state carried from `source` to `target` through `maps`, as the app's
/// handoff carries the reference's: Q through the primary map with its
/// component totals kept on the same geometry, b through its map.
fn carry(
    state: &CanonicalWaveState,
    source: &PreparedTopology,
    target: &PreparedTopology,
    maps: &PreparedCanonicalTransfer,
) -> CanonicalWaveState {
    let runtime = CanonicalGpuRuntimeTransfer::from_primary_transfer(
        &source.canonical_operator,
        &target.canonical_operator,
        &source.canonical_forcing,
        &target.canonical_forcing,
        &maps.primary,
        [0; 4],
    )
    .expect("the runtime transfer");
    let runtime = if target.mesh.geometry_revision == source.mesh.geometry_revision {
        runtime
    } else {
        runtime.without_total_correction()
    };
    let mut totals = vec![0.0; source.canonical_operator.component_count()];
    for (value, label) in state
        .primary_flux()
        .iter()
        .zip(source.canonical_operator.component_labels())
    {
        totals[*label as usize] += value;
    }
    let desired = runtime
        .components
        .iter()
        .map(|component| {
            (!component.sources.is_empty()).then(|| {
                component
                    .sources
                    .iter()
                    .map(|(source, share)| share * totals[*source as usize])
                    .sum()
            })
        })
        .collect::<Vec<Option<f64>>>();
    let prescribed = target
        .canonical_forcing
        .prescribed()
        .iter()
        .map(Option::is_some)
        .collect::<Vec<_>>();
    let (primary, _) = maps
        .primary
        .transfer(state.primary_flux(), &desired, &prescribed)
        .expect("the primary carry");
    let (complementary, _) = maps
        .complementary
        .transfer(state.complementary_flux())
        .expect("the complementary carry");
    CanonicalWaveState::new(
        &target.canonical_operator,
        state.time_step(),
        primary,
        complementary,
    )
    .expect("the carried state")
}

/// The carry from the mesh at `edge` to one 1.4 times finer, on the
/// reference: its budget and what it must keep within it.
struct Carry {
    fine: Arc<PreparedTopology>,
    carried: CanonicalWaveState,
    /// The carried flux against the mode sampled on the fine mesh.
    budget: f64,
    /// The flux's integral, carried against the source's.
    physical: f64,
    /// The compatible share the carried flux has in the new space.
    coupling: f64,
    /// What that share pushed into Q over the following steps.
    leakage: f64,
    /// The right side's share of the carried flux's energy, at the carry and
    /// after the steps.
    crossing: [f64; 2],
}

fn carry_at(edge: f64, baffle: bool, time_step: Option<f64>) -> Carry {
    let document = document(baffle);
    let mut runtime = TopologyRuntime::default();
    let (coarse, _) = prepare(&mut runtime, &document, 1, edge, false);
    let (fine, maps) = prepare(&mut runtime, &document, 2, edge / 1.4, true);
    let maps = maps.expect("the edit's maps");
    let time_step = time_step.unwrap_or_else(|| {
        coarse
            .recommended_time_step()
            .min(fine.recommended_time_step())
    });
    let state = mode(&coarse.canonical_operator, time_step, baffle);
    let mut carried = carry(&state, &coarse, &fine, &maps);
    let operator = &fine.canonical_operator;
    let truth = sampled_flux(operator, baffle);
    let budget = relative_l2(
        carried
            .complementary_flux()
            .iter()
            .flat_map(|value| [value.x, value.y]),
        truth.iter().flat_map(|value| [value.x, value.y]),
    );
    let before = integral(&coarse.canonical_operator, state.complementary_flux());
    let after = integral(operator, carried.complementary_flux());
    let physical = ((after - before) / before).abs();
    let coupling = compatible_share(operator, carried.complementary_flux());
    let mode_energy = side_energies(operator, carried.complementary_flux())
        .iter()
        .sum::<f64>();
    let crossing_before = side_energies(operator, carried.complementary_flux())[1] / mode_energy;
    let at_carry = carried.clone();
    for _ in 0..steps() {
        carried
            .step_with_forcing(operator, &fine.canonical_forcing)
            .expect("the carried state steps");
    }
    let after_steps = reading(
        operator,
        carried.primary_flux(),
        carried.complementary_flux(),
        mode_energy,
    );
    Carry {
        fine,
        carried: at_carry,
        budget,
        physical,
        coupling,
        leakage: after_steps.pushed,
        crossing: [crossing_before, after_steps.right],
    }
}

#[derive(Resource)]
struct Pending {
    plan: Option<CanonicalGpuPlan>,
}

#[derive(Resource)]
struct Expected {
    operator: Arc<CanonicalWaveOperator>,
    complementary: Vec<Point2>,
    mode_energy: f64,
    baffle: bool,
    /// The carry's budget, where the device steps the carried state; none on
    /// the fixed mesh, where the demand is exact.
    budget: Option<f64>,
    started: Instant,
    deadline: Instant,
    finished: bool,
    failed: bool,
}

fn main() -> AppExit {
    let baffle = std::env::args().any(|argument| argument == "--baffle");
    let remesh = std::env::args().any(|argument| argument == "--remesh");
    let edge = edge();
    let mut failed = false;

    // The fixed mesh, in f64: exact.
    let document = document(baffle);
    let mut runtime = TopologyRuntime::default();
    let (prepared, _) = prepare(&mut runtime, &document, 1, edge, false);
    let operator = prepared.canonical_operator.clone();
    let time_step = prepared.recommended_time_step();
    let initial = mode(&operator, time_step, baffle);
    let mode_energy = side_energies(&operator, initial.complementary_flux())
        .iter()
        .sum::<f64>();
    let mut reference = initial.clone();
    for _ in 0..steps() {
        reference
            .step_with_forcing(&operator, &prepared.canonical_forcing)
            .expect("the reference steps");
    }
    let fixed = reading(
        &operator,
        reference.primary_flux(),
        reference.complementary_flux(),
        mode_energy,
    );
    println!(
        "complementary mode{}: {} dofs at edge {edge}, dt {time_step:.4e}, energy {mode_energy:.4}, \
         {} steps in f64: Q holds {:.1e} of it, b's energy changed {:.1e}, the right side holds {:.1e}",
        if baffle { " on the baffled box" } else { "" },
        operator.degrees_of_freedom(),
        steps(),
        fixed.pushed,
        fixed.energy_change,
        fixed.right
    );
    if !within(fixed.pushed, EXACT)
        || !within(fixed.energy_change, EXACT)
        || (baffle && !within(fixed.right, EXACT))
    {
        eprintln!("the mode is not stationary in f64");
        failed = true;
    }

    // The carry, in f64, at the edge and at half of it.
    let carries = [edge, 0.5 * edge].map(|edge| carry_at(edge, baffle, None));
    for (edge, carry) in [edge, 0.5 * edge].iter().zip(&carries) {
        println!(
            "carry at edge {edge}: budget {:.2e}; the integral {:.2e} off, compatible share {:.2e}, \
             {:.2e} of the energy in Q after {} steps, the right side {:.2e} at the carry and \
             {:.2e} after",
            carry.budget,
            carry.physical,
            carry.coupling,
            carry.leakage,
            steps(),
            carry.crossing[0],
            carry.crossing[1]
        );
        let bound = carry.budget;
        if !within(carry.physical, bound)
            || !within(carry.coupling, bound)
            || !within(carry.leakage, bound)
            || (baffle && (!within(carry.crossing[0], bound) || !within(carry.crossing[1], bound)))
        {
            eprintln!("the carry at edge {edge} is not within its budget");
            failed = true;
        }
    }
    if carries[0].budget < 3.0 * carries[1].budget {
        eprintln!(
            "the budget does not fall with the mesh: {:.2e} at the edge, {:.2e} at half of it",
            carries[0].budget, carries[1].budget
        );
        failed = true;
    }

    // The device: the mode on the fixed mesh, or the carried state.
    let (device_operator, device_prepared, device_state) = if remesh {
        let carry = &carries[0];
        (
            carry.fine.canonical_operator.clone(),
            carry.fine.clone(),
            carry.carried.clone(),
        )
    } else {
        (operator.clone(), prepared.clone(), initial)
    };
    let plan = CanonicalGpuPlan::compile_with_quadratic(
        &device_operator,
        &device_prepared.operator,
        &device_state,
        &device_prepared.canonical_forcing,
        CanonicalGpuClock::initial(device_state.time_step()).expect("the clock"),
    )
    .expect("the plan");
    let mut expected = device_state.clone();
    for _ in 0..steps() {
        expected
            .step_with_forcing(&device_operator, &device_prepared.canonical_forcing)
            .expect("the reference steps");
    }
    let device_mode_energy = side_energies(&device_operator, device_state.complementary_flux())
        .iter()
        .sum::<f64>();
    if failed {
        eprintln!("the f64 demands failed; the device is not run");
        return AppExit::error();
    }
    let mut app = App::new();
    app.add_plugins(DefaultPlugins.set(WindowPlugin {
        primary_window: Some(Window {
            title: "p".into(),
            visible: false,
            resolution: (64, 64).into(),
            ..default()
        }),
        ..default()
    }))
    .add_plugins(WaveGpuPlugin)
    .add_plugins(CanonicalWaveGpuPlugin)
    .insert_resource(Pending { plan: Some(plan) })
    .insert_resource(Expected {
        operator: device_operator,
        complementary: expected.complementary_flux().to_vec(),
        mode_energy: device_mode_energy,
        baffle,
        budget: remesh.then_some(carries[0].budget),
        started: Instant::now(),
        deadline: check::deadline(300),
        finished: false,
        failed: false,
    })
    .add_systems(Startup, install)
    .add_systems(Update, drive)
    .run()
}

fn install(
    mut commands: Commands,
    mut pending: ResMut<Pending>,
    mut assets: ResMut<Assets<ShaderBuffer>>,
    mut canonical: ResMut<CanonicalGpuRequest>,
) {
    canonical.install(
        &mut assets,
        &mut commands,
        pending.plan.take().expect("one plan"),
    );
    canonical.request_steps(steps());
    commands.spawn(Camera2d);
}

fn drive(
    mut request: ResMut<CanonicalGpuRequest>,
    display: Res<CanonicalGpuDisplay>,
    mut expected: ResMut<Expected>,
    mut exit: MessageWriter<AppExit>,
) {
    if expected.finished {
        exit.write(if expected.failed {
            AppExit::error()
        } else {
            AppExit::Success
        });
        return;
    }
    if Instant::now() >= expected.deadline || request.stats().failure() != 0 {
        eprintln!("the device timed out or failed");
        expected.failed = true;
        expected.finished = true;
        return;
    }
    if request.stats().completed_steps() < steps() {
        return;
    }
    if !request.request_full_state_readback() && display.full_readbacks == 0 {
        return;
    }
    if display.primary_flux.len() != expected.operator.degrees_of_freedom()
        || display.complementary_flux.len() != expected.complementary.len()
    {
        return;
    }
    let primary = display
        .primary_flux
        .iter()
        .map(|value| f64::from(*value))
        .collect::<Vec<_>>();
    let complementary = display
        .complementary_flux
        .iter()
        .map(|value| Point2::new(f64::from(value[0]), f64::from(value[1])))
        .collect::<Vec<_>>();
    let b = relative_l2(
        complementary.iter().flat_map(|value| [value.x, value.y]),
        expected
            .complementary
            .iter()
            .flat_map(|value| [value.x, value.y]),
    );
    let device = reading(
        &expected.operator,
        &primary,
        &complementary,
        expected.mode_energy,
    );
    println!(
        "device after {:.2} ms: b {b:.3e} from the reference; Q holds {:.1e} of the mode's energy, \
         b's energy changed {:.1e}, the right side holds {:.1e}",
        expected.started.elapsed().as_secs_f64() * 1_000.0,
        device.pushed,
        device.energy_change,
        device.right
    );
    expected.finished = true;
    let kept = match expected.budget {
        // The carried state is the reference's: what it pushes and leaks is
        // the carry's, within its budget, and the device must follow it.
        Some(budget) => within(b, TOLERANCE) && (!expected.baffle || within(device.right, budget)),
        None => {
            within(b, TOLERANCE)
                && within(device.pushed, DEVICE_EXACT)
                && within(device.energy_change, DEVICE_EXACT)
                && (!expected.baffle || device.right == 0.0)
        }
    };
    if !kept {
        eprintln!("the device does not keep the mode");
        expected.failed = true;
    }
}
