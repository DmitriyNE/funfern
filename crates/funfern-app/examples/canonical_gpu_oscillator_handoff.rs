//! Real-device validation of the integrated field across a generation
//! handoff (Stage 11, Gate O).
//!
//! `r = ∫u dt` is a nodal field, not a conserved one, so a handoff
//! interpolates it on the primary map's rows while `Q` is conserved and `b`
//! reconstructed. The host does the same through `transfer_integrated_field`,
//! steps both generations in f64, and the device is held to it.
//!
//! `OSCILLATOR_HANDOFF` picks the handoff:
//! - `remesh` (default): a sine-Gordon kink running at half the wave speed,
//!   handed from an edge-0.1 mesh to a non-nested edge-0.07 one;
//! - `identity`: sine-Gordon on one mesh, handed to itself;
//! - `from-linear`: a plain medium handed to Klein-Gordon, which starts at
//!   `r = 0`;
//! - `to-linear`: Klein-Gordon handed to a plain medium, which drops `r`;
//! - `reject`: a φ⁴ wall handed to a φ⁴ medium whose bound it passes. The
//!   device must reject the handoff with the restoring-domain status and keep
//!   stepping the source, as the reference refuses the same state.
//! - `opened`: sine-Gordon around a hole that moves by 0.05, so the target
//!   has nodes the source never covered. Each takes `r` from the neighbours
//!   `Q`'s extension reads, on the device as on the reference, instead of
//!   starting at zero.
//!
//! Every mode prepares its primary map with the meshes, as the app's runtime
//! does, so the extension rows are the production ones.

use std::time::{Duration, Instant};

use bevy::{app::AppExit, prelude::*, render::storage::ShaderBuffer};
use funfern_app::canonical_gpu::{
    CANONICAL_FAILURE_RESTORING_DOMAIN, CanonicalGpuClock, CanonicalGpuDisplay,
    CanonicalGpuHandoffOutcome, CanonicalGpuPlan, CanonicalGpuRequest, CanonicalGpuRuntimeTransfer,
    CanonicalGpuTransferPlan, CanonicalWaveGpuPlugin,
};
use funfern_app::wave_gpu::WaveGpuPlugin;
use funfern_core::{
    CanonicalForcing, CanonicalOutgoingHistoryTransferMap, CanonicalPrimaryTransferMap,
    CanonicalTemporalWaveOperator, CanonicalTemporalWaveState, CanonicalThinGapHistoryTransferMap,
    CanonicalVectorTransferMap, CoefficientLaw, MeshingOptions, Obstacle, ObstacleId,
    OuterBoundaryCondition, PeriodicCubicSpline, Point2, QuadraticTransferMap,
    QuadraticWaveOperator, RestoringLaw, ScalarField, Scene, TriMesh, mesh_scene,
    transfer_integrated_field,
};

const WARMUP_STEPS: u64 = 60;
const TARGET_STEPS: u64 = 60;

#[derive(Resource)]
struct Pending {
    source: Option<CanonicalGpuPlan>,
    target: Option<CanonicalGpuPlan>,
    transfer: Option<CanonicalGpuTransferPlan>,
}

#[derive(Resource)]
struct Expected {
    primary: Vec<f64>,
    complementary: Vec<Point2>,
    integrated: Vec<f64>,
    reject: bool,
    phase: Phase,
    started: Instant,
    deadline: Instant,
    failed: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Warmup,
    Handoff,
    Evolution,
    /// After a rejected handoff: the source must keep stepping.
    Survived,
    Done,
}

struct Generation {
    mesh: TriMesh,
    scalar: QuadraticWaveOperator,
    operator: CanonicalTemporalWaveOperator,
}

fn generation(scene: &Scene, edge: f64, revision: u64) -> Generation {
    let mut fixed = scene.clone();
    for material in &mut fixed.materials {
        material.mass_law = CoefficientLaw::linear();
        material.stiffness_law = CoefficientLaw::linear();
        material.restoring = RestoringLaw::None;
    }
    let mesh = mesh_scene(
        &fixed,
        revision,
        MeshingOptions {
            target_edge_length: edge,
            ..MeshingOptions::default()
        },
    )
    .expect("handoff mesh");
    let scalar =
        QuadraticWaveOperator::assemble_scene(&mesh, &fixed, OuterBoundaryCondition::Reflecting)
            .expect("handoff scalar operator");
    let operator = CanonicalTemporalWaveOperator::compile_scene(&mesh, &scalar, scene, revision)
        .expect("handoff operator");
    Generation {
        mesh,
        scalar,
        operator,
    }
}

fn with_law(law: RestoringLaw) -> Scene {
    let mut scene = Scene::default();
    scene.materials[0].restoring = law;
    scene
}

fn main() -> AppExit {
    let mode = std::env::var("OSCILLATOR_HANDOFF").unwrap_or_else(|_| "remesh".into());
    let sine_gordon = |omega0: f64| {
        with_law(RestoringLaw::SineGordon {
            omega0: ScalarField::constant(omega0),
        })
    };
    let klein_gordon = with_law(RestoringLaw::KleinGordon {
        omega0: ScalarField::constant(3.0),
    });
    let phi4 = |bound: f64| {
        with_law(RestoringLaw::Phi4 {
            lambda: ScalarField::constant(16.0),
            amplitude_bound: ScalarField::constant(bound),
        })
    };
    let (source, target) = match mode.as_str() {
        "remesh" => (
            generation(&sine_gordon(4.0), 0.1, 1),
            generation(&sine_gordon(4.0), 0.07, 2),
        ),
        "identity" => {
            let source = generation(&sine_gordon(3.0), 0.12, 1);
            let target = Generation {
                mesh: source.mesh.clone(),
                scalar: source.scalar.clone(),
                operator: source.operator.clone(),
            };
            (source, target)
        }
        "from-linear" => (
            generation(&Scene::default(), 0.12, 1),
            generation(&klein_gordon, 0.12, 1),
        ),
        "to-linear" => (
            generation(&klein_gordon, 0.12, 1),
            generation(&Scene::default(), 0.12, 1),
        ),
        "reject" => (
            generation(&phi4(1.6), 0.12, 1),
            generation(&phi4(0.9), 0.12, 1),
        ),
        "opened" => {
            let holed = |x: f64| {
                let mut scene = sine_gordon(3.0);
                scene.obstacles = vec![Obstacle::hole(
                    ObstacleId(1),
                    PeriodicCubicSpline::rounded(Point2::new(x, 0.1), 0.3),
                )];
                scene
            };
            (
                generation(&holed(0.0), 0.12, 1),
                generation(&holed(0.05), 0.12, 2),
            )
        }
        other => panic!("unknown OSCILLATOR_HANDOFF {other}"),
    };
    let reject = mode == "reject";
    let same_mesh = source.mesh.vertices.len() == target.mesh.vertices.len()
        && source.mesh.triangles == target.mesh.triangles;
    let time_step = 0.38
        * source
            .operator
            .maximum_time_step()
            .min(target.operator.maximum_time_step());

    // The source state: a moving kink for the remesh, a φ⁴ wall for the
    // rejection, and otherwise a smooth field whose `r` is seeded wherever
    // the source can hold one.
    let base = source.operator.base();
    let (primary, integrated) = match mode.as_str() {
        "remesh" => {
            let (length, speed, start) = (0.25, 0.5, -0.3);
            let gamma = 1.0 / (1.0_f64 - speed * speed).sqrt();
            let r = base
                .node_points()
                .iter()
                .map(|point| 4.0 * (gamma * (point.x - start) / length).exp().atan())
                .collect::<Vec<_>>();
            let q = base
                .node_points()
                .iter()
                .zip(base.primary_mass())
                .map(|(point, mass)| {
                    let s = gamma * (point.x - start) / length;
                    mass * (-speed * 2.0 * gamma / length / s.cosh())
                })
                .collect::<Vec<_>>();
            (q, r)
        }
        "reject" => {
            let width = 2.0_f64.sqrt() / 4.0;
            let r = base
                .node_points()
                .iter()
                .map(|point| (point.x / width).tanh())
                .collect::<Vec<_>>();
            (vec![0.0; base.degrees_of_freedom()], r)
        }
        _ => {
            let q = base
                .node_points()
                .iter()
                .zip(base.primary_mass())
                .map(|(point, mass)| mass * 0.5 * (1.4 * point.x - 0.9 * point.y).sin())
                .collect::<Vec<_>>();
            let r = base
                .node_points()
                .iter()
                .map(|point| 1.2 * (0.7 * point.x + 0.5 * point.y).cos())
                .collect::<Vec<_>>();
            (q, r)
        }
    };
    let complementary = if source.operator.has_restoring() {
        base.compatible_flux(&integrated)
            .expect("compatible source flux")
    } else {
        vec![Point2::default(); base.complementary_degrees_of_freedom()]
    };
    let mut source_state =
        CanonicalTemporalWaveState::new(&source.operator, time_step, primary, complementary)
            .expect("source state");
    if source.operator.has_restoring() {
        source_state = source_state
            .with_integrated_field(&source.operator, integrated)
            .expect("source integrated field");
    }
    let clock = CanonicalGpuClock::initial(time_step).expect("handoff clock");
    let source_forcing = CanonicalForcing::none(source.operator.base());
    let target_forcing = CanonicalForcing::none(target.operator.base());
    let source_plan =
        CanonicalGpuPlan::compile_temporal(&source.operator, &source_state, &source_forcing, clock)
            .expect("source GPU plan");
    for _ in 0..WARMUP_STEPS {
        source_state
            .step(&source.operator)
            .expect("source oracle step");
    }

    // The maps, shared by the host oracle and the device transfer.
    let interpolation = if same_mesh {
        QuadraticTransferMap::identity_on_mesh(&source.mesh, &source.scalar, &target.scalar)
    } else {
        QuadraticTransferMap::build(&source.mesh, &source.scalar, &target.mesh, &target.scalar)
    }
    .expect("handoff interpolation");
    let (source_base, target_base) = (source.operator.base(), target.operator.base());
    let primary_map = CanonicalPrimaryTransferMap::prepare_with_meshes(
        &interpolation,
        &source.mesh,
        source_base,
        &target.mesh,
        target_base,
    )
    .expect("primary map");
    let vector_map =
        CanonicalVectorTransferMap::prepare(&source.mesh, source_base, &target.mesh, target_base)
            .expect("vector map");
    let gap_map = CanonicalThinGapHistoryTransferMap::prepare(
        source_base.thin_gap_samples(),
        target_base.thin_gap_samples(),
    )
    .expect("gap map");
    let outgoing_map =
        CanonicalOutgoingHistoryTransferMap::prepare(&interpolation, source_base, target_base)
            .expect("outgoing map");

    // The host oracle: `Q` with each component's total carried, as the
    // device does, `b` reconstructed, `r` interpolated.
    let labels = source_base.component_labels();
    let mut totals = vec![0.0; primary_map.target_component_count()];
    for (value, label) in source_state.primary_flux().iter().zip(labels) {
        totals[*label as usize] += value;
    }
    let totals = totals.into_iter().map(Some).collect::<Vec<_>>();
    let target_primary = primary_map
        .transfer(
            source_state.primary_flux(),
            &totals,
            &vec![false; target_base.degrees_of_freedom()],
        )
        .expect("primary transfer")
        .0;
    let target_complementary = vector_map
        .transfer(source_state.complementary_flux())
        .expect("vector transfer")
        .0;
    let integrated = transfer_integrated_field(
        &interpolation,
        &primary_map,
        source_state.integrated_field(),
        &target.operator,
    )
    .expect("integrated transfer");
    println!(
        "uncovered target nodes: {} extended from their neighbours, {} at r = 0",
        integrated.extended_nodes, integrated.exposed_nodes
    );
    if mode == "opened" {
        assert!(
            integrated.extended_nodes > 0,
            "the moved hole uncovered nothing"
        );
    }
    println!(
        "oscillator handoff ({mode}): {} → {} nodes; r {}{}{}",
        source_base.degrees_of_freedom(),
        target_base.degrees_of_freedom(),
        if integrated.discarded { "dropped" } else { "" },
        if integrated.started_at_zero {
            "started at zero"
        } else {
            ""
        },
        if !integrated.discarded && !integrated.started_at_zero {
            "interpolated"
        } else {
            ""
        },
    );
    let target_state = CanonicalTemporalWaveState::new_at(
        &target.operator,
        time_step,
        target_primary,
        target_complementary,
        source_state.time(),
    )
    .expect("target oracle state");
    let target_state = if integrated.field.is_empty() {
        Ok(target_state)
    } else {
        target_state.with_integrated_field(&target.operator, integrated.field.clone())
    };
    if reject {
        assert!(
            target_state.is_err(),
            "the fixture must pass the target's bound"
        );
    }
    let mut target_state = match target_state {
        Ok(state) => state,
        Err(_) => CanonicalTemporalWaveState::zero(&target.operator, time_step)
            .expect("placeholder for the rejected target"),
    };
    if !reject {
        for _ in 0..TARGET_STEPS {
            target_state
                .step(&target.operator)
                .expect("target oracle step");
        }
    }

    let placeholder =
        CanonicalTemporalWaveState::zero(&target.operator, time_step).expect("target placeholder");
    let target_plan =
        CanonicalGpuPlan::compile_temporal(&target.operator, &placeholder, &target_forcing, clock)
            .expect("target GPU plan");
    let runtime = CanonicalGpuRuntimeTransfer::from_primary_transfer(
        source_base,
        target_base,
        &source_forcing,
        &target_forcing,
        &primary_map,
        [0; 4],
    )
    .expect("runtime ownership");
    let transfer = CanonicalGpuTransferPlan::compile(
        source_base,
        target_base,
        &source_forcing,
        &target_forcing,
        &primary_map,
        &vector_map,
        &gap_map,
        &outgoing_map,
        &runtime,
    )
    .expect("geometry transfer")
    .with_temporal_material_runtime(&source_plan, &target_plan)
    .expect("material runtime transfer")
    .with_integrated_field(&primary_map, &source_plan, &target_plan)
    .expect("integrated-field transfer");

    let mut app = App::new();
    app.add_plugins(DefaultPlugins.set(WindowPlugin {
        primary_window: Some(Window {
            title: "funfern oscillator handoff validation".into(),
            visible: false,
            resolution: (64, 64).into(),
            ..default()
        }),
        ..default()
    }))
    .add_plugins(WaveGpuPlugin)
    .add_plugins(CanonicalWaveGpuPlugin)
    .insert_resource(Pending {
        source: Some(source_plan),
        target: Some(target_plan),
        transfer: Some(transfer),
    })
    .insert_resource(Expected {
        primary: target_state.primary_flux().to_vec(),
        complementary: target_state.complementary_flux().to_vec(),
        integrated: target_state.integrated_field().to_vec(),
        reject,
        phase: Phase::Warmup,
        started: Instant::now(),
        deadline: Instant::now() + Duration::from_secs(120),
        failed: false,
    })
    .add_systems(Startup, install)
    .add_systems(Update, validate);
    app.run()
}

fn install(
    mut commands: Commands,
    mut pending: ResMut<Pending>,
    mut assets: ResMut<Assets<ShaderBuffer>>,
    mut request: ResMut<CanonicalGpuRequest>,
) {
    request
        .set_continuous_full_state_readback(true)
        .expect("select handoff readback mode");
    request.install(
        &mut assets,
        &mut commands,
        pending.source.take().expect("source plan"),
    );
    request.request_steps(WARMUP_STEPS);
    commands.spawn(Camera2d);
}

fn validate(
    mut commands: Commands,
    mut pending: ResMut<Pending>,
    mut assets: ResMut<Assets<ShaderBuffer>>,
    mut request: ResMut<CanonicalGpuRequest>,
    display: Res<CanonicalGpuDisplay>,
    mut expected: ResMut<Expected>,
    mut exit: MessageWriter<AppExit>,
) {
    if expected.phase == Phase::Done {
        exit.write(if expected.failed {
            AppExit::error()
        } else {
            AppExit::Success
        });
        return;
    }
    let rejected_as_expected = expected.reject
        && matches!(
            request.handoff_outcome(),
            CanonicalGpuHandoffOutcome::Rejected(_)
        );
    if Instant::now() >= expected.deadline
        || (request.stats().failure() != 0 && !rejected_as_expected)
    {
        eprintln!(
            "oscillator handoff timed out or failed: status {}",
            request.stats().failure()
        );
        expected.failed = true;
        expected.phase = Phase::Done;
        return;
    }
    match expected.phase {
        Phase::Warmup
            if display
                .clock
                .is_some_and(|clock| clock.accepted_steps >= WARMUP_STEPS as u32) =>
        {
            request
                .begin_handoff(
                    &mut assets,
                    &mut commands,
                    pending.target.take().expect("target plan"),
                    pending.transfer.take().expect("transfer plan"),
                )
                .expect("begin oscillator handoff");
            expected.phase = Phase::Handoff;
        }
        Phase::Handoff => match request.handoff_outcome() {
            CanonicalGpuHandoffOutcome::Accepted if expected.reject => {
                eprintln!("the device admitted an integrated field past the target's bound");
                expected.failed = true;
                expected.phase = Phase::Done;
            }
            CanonicalGpuHandoffOutcome::Accepted => {
                request.request_steps(TARGET_STEPS);
                expected.phase = Phase::Evolution;
            }
            CanonicalGpuHandoffOutcome::Rejected(reason) if expected.reject => {
                println!(
                    "oscillator handoff rejected with status {reason} ({})",
                    funfern_app::canonical_gpu::canonical_failure_description(reason)
                );
                if reason != CANONICAL_FAILURE_RESTORING_DOMAIN {
                    expected.failed = true;
                    expected.phase = Phase::Done;
                    return;
                }
                request.request_steps(8);
                expected.phase = Phase::Survived;
            }
            CanonicalGpuHandoffOutcome::Rejected(reason) => {
                eprintln!("oscillator handoff rejected with status {reason}");
                expected.failed = true;
                expected.phase = Phase::Done;
            }
            _ => {}
        },
        Phase::Survived => {
            if display
                .clock
                .is_some_and(|clock| clock.accepted_steps >= WARMUP_STEPS as u32 + 8)
            {
                println!("the source kept stepping after the rejection");
                expected.phase = Phase::Done;
            }
        }
        Phase::Evolution => {
            if request.stats().completed_steps() < WARMUP_STEPS + TARGET_STEPS {
                return;
            }
            if !request.request_full_state_readback(&mut commands) && display.full_readbacks == 0 {
                return;
            }
            if display.primary_flux.len() != expected.primary.len()
                || display.complementary_flux.len() != expected.complementary.len()
                || display.integrated_field().len() != expected.integrated.len()
                || display
                    .clock
                    .is_none_or(|clock| clock.accepted_steps < (WARMUP_STEPS + TARGET_STEPS) as u32)
            {
                return;
            }
            let primary = relative_l2(
                display.primary_flux.iter().map(|value| f64::from(*value)),
                expected.primary.iter().copied(),
            );
            let complementary = relative_l2(
                display
                    .complementary_flux
                    .iter()
                    .flat_map(|value| value.iter().map(|lane| f64::from(*lane))),
                expected
                    .complementary
                    .iter()
                    .flat_map(|value| [value.x, value.y]),
            );
            let integrated = if expected.integrated.is_empty() {
                0.0
            } else {
                relative_l2(
                    display
                        .integrated_field()
                        .iter()
                        .map(|value| f64::from(*value)),
                    expected.integrated.iter().copied(),
                )
            };
            println!(
                "oscillator handoff errors after {:.2} ms: Q {primary:.3e}, b \
                 {complementary:.3e}, r {integrated:.3e}",
                expected.started.elapsed().as_secs_f64() * 1_000.0
            );
            if primary > 3.0e-5 || complementary > 3.0e-5 || integrated > 3.0e-5 {
                expected.failed = true;
            }
            expected.phase = Phase::Done;
        }
        _ => {}
    }
}

fn relative_l2(
    actual: impl Iterator<Item = f64>,
    expected: impl Iterator<Item = f64> + Clone,
) -> f64 {
    let scale = expected
        .clone()
        .map(|value| value * value)
        .sum::<f64>()
        .max(1.0e-30);
    let difference = actual
        .zip(expected)
        .map(|(actual, expected)| (actual - expected).powi(2))
        .sum::<f64>();
    (difference / scale).sqrt()
}
