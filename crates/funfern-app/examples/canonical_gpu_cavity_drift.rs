//! C9's long run on the device: PH-01's reflecting 2 × 1.4 box on edge
//! 0.08, mode (1,0) and half of (2,3) started at a third of a period, for
//! `--steps` steps (200,000 unless given; 3,500 s of the mechanical box)
//! under the fourth-order step in `--physics mechanical` (the default), `tm`
//! or `te`, and with `--filter` the grid filter every sixteen steps as the
//! app applies it. Device and f64 step from the same state and are read at
//! forty checkpoints, each mode's phasor from three samples in a row. Over
//! a long run the two differ only in what f32 makes of the state. The step
//! is the app's, `device_time_step`, an f32 the device steps by exactly:
//! - the device's clock reads `n dt` within 2e-6 s, what f32 makes of a
//!   time up to an epoch's 16 s (5e-7 measured);
//! - each mode's phase drifts from the reference's as a frequency within
//!   1e-8 of its own, from a tenth of the run on (8e-10 to 3.6e-9 on
//!   lavapipe; `canonical_gpu_cavity_modes` holds the device's frequency to
//!   1e-7), and its amplitude keeps within 1e-5 (9.7e-7 to 3e-6 measured:
//!   the cache's rounding, held up to sixteen steps between rebuilds, is a
//!   faint noise in the force, which warms the modes as any noise does,
//!   three to eight times what the force gathered every step leaves);
//! - the flux's stationary part, which nothing moves, holds under 1e-9 of
//!   the energy (4.4e-11; it gathers 2e-10 over a million steps);
//! - the energy within 1e-5 of the reference's, relative to the start
//!   (1.3e-6 to 3.4e-6);
//! - the sum of `Q`, which the reflecting box conserves, within 2e-5 of the
//!   reference's (1.7e-6 to 4.6e-6), and the uniform mode it would carry
//!   under 1e-10 of the energy (1.8e-13 to 1.2e-12). The force cache a fixed
//!   generation keeps beside the flux (`F = Cᵀ b`, advanced each step) is
//!   rebuilt from the flux every sixteen steps; before, its f32 rounding
//!   walked unchecked and the sum integrated it: here 6.2e-4 by 200,000
//!   steps and the uniform mode 2.1e-8 of the energy (the study's run read
//!   3.3e-3 and 6e-7, and 3e-4 and 5e-9 under the filter), a uniform offset
//!   of 0.4% of the field by a million (C9, 2026-11-14).
//!
//! The readings are lavapipe's; 200,000 steps take about ten minutes there
//! under amd64 emulation.
//!
//! Exits 0 when every demand holds.

#[path = "support/check.rs"]
mod check;
use bevy::{app::AppExit, prelude::*, render::storage::ShaderBuffer};
use check::within;
use funfern_app::canonical_gpu::{
    CanonicalGpuClock, CanonicalGpuDisplay, CanonicalGpuPlan, CanonicalGpuRequest,
    CanonicalWaveGpuPlugin, device_time_step,
};
use funfern_app::topology_editor::{TopologyDocument, TopologyEditor};
use funfern_app::topology_runtime::{PreparedTopology, TopologyRuntime};
use funfern_app::wave_gpu::WaveGpuPlugin;
use funfern_core::{
    CanonicalIntegrator, CanonicalWaveState, DomainRect, ElectromagneticPolarization,
    MeshingOptions, OuterBoundaryCondition, OuterBoundaryConditions, PhysicsModel, Point2,
    ScalarField,
};
use std::sync::Arc;
use std::time::Instant;

/// The modes' starting phase, as in `canonical_gpu_cavity_modes`.
const PHASE: f64 = std::f64::consts::FRAC_PI_3;
/// The checkpoints the run is read at, past the start.
const CHECKPOINTS: u64 = 40;
/// How far the device's clock may stand from the steps it took, in seconds.
const CLOCK: f64 = 2.0e-6;
/// How far a mode's frequency, read from its phase against the device's
/// clock, and its amplitude may stand from the reference's, relatively.
const FREQUENCY: f64 = 1.0e-8;
const AMPLITUDE: f64 = 1.0e-5;
/// The stationary flux's energy, and the energy's difference from the
/// reference's, over the starting energy.
const STATIONARY: f64 = 1.0e-9;
const ENERGY: f64 = 1.0e-5;
/// How far the sum of `Q` may stand from the reference's, and the energy of
/// the uniform mode it carries, over the starting energy.
const TOTAL: f64 = 2.0e-5;
const UNIFORM: f64 = 1.0e-10;

#[derive(Clone, Copy, Debug)]
struct BoxMode {
    m: u32,
    n: u32,
}

impl BoxMode {
    fn wavenumbers(self) -> Point2 {
        Point2::new(
            f64::from(self.m) * std::f64::consts::PI / 2.0,
            f64::from(self.n) * std::f64::consts::PI / 1.4,
        )
    }

    fn wavenumber(self) -> f64 {
        self.wavenumbers().norm()
    }

    fn shape(self, point: Point2) -> f64 {
        let k = self.wavenumbers();
        (k.x * (point.x + 1.0)).cos() * (k.y * (point.y + 0.7)).cos()
    }

    fn gradient(self, point: Point2) -> Point2 {
        let k = self.wavenumbers();
        let (x, y) = (k.x * (point.x + 1.0), k.y * (point.y + 0.7));
        Point2::new(-k.x * x.sin() * y.cos(), -k.y * x.cos() * y.sin())
    }
}

/// The wave speed and orientation each model's law gives the medium, mass
/// slot 2 and stiffness slot 0.5, as `box_law` in the f64 test writes them.
fn law(physics: PhysicsModel) -> (f64, f64) {
    match physics {
        PhysicsModel::Mechanical => (0.5, -1.0),
        PhysicsModel::Electromagnetic {
            polarization: ElectromagneticPolarization::Tm,
        } => (1.0, 1.0),
        PhysicsModel::Electromagnetic {
            polarization: ElectromagneticPolarization::Te,
        } => (1.0, -1.0),
    }
}

/// The run's arguments: `--physics`, `--filter` and `--steps`.
struct Arguments {
    physics: PhysicsModel,
    filtered: bool,
    steps: u64,
}

fn arguments() -> Arguments {
    let mut parsed = Arguments {
        physics: PhysicsModel::Mechanical,
        filtered: false,
        steps: 200_000,
    };
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--physics" => {
                parsed.physics = match args.next().as_deref() {
                    Some("mechanical") => PhysicsModel::Mechanical,
                    Some("tm") => PhysicsModel::Electromagnetic {
                        polarization: ElectromagneticPolarization::Tm,
                    },
                    Some("te") => PhysicsModel::Electromagnetic {
                        polarization: ElectromagneticPolarization::Te,
                    },
                    other => panic!("--physics takes mechanical, tm or te, not {other:?}"),
                };
            }
            "--filter" => parsed.filtered = true,
            "--steps" => {
                parsed.steps = args
                    .next()
                    .and_then(|value| value.parse().ok())
                    .filter(|steps| *steps >= CHECKPOINTS)
                    .expect("--steps takes a count of at least forty");
            }
            other => panic!("unknown argument {other}"),
        }
    }
    parsed
}

/// The reflecting 2 × 1.4 box of the medium in `physics`, no source, the
/// grid filter as asked, adaptation off.
fn document(physics: PhysicsModel, filtered: bool) -> TopologyDocument {
    let mut document = TopologyEditor::default().document;
    for scene in [&mut document.model.draft, &mut document.model.accepted] {
        scene.physics = physics;
        scene.geometry.domain = DomainRect {
            min_x: -1.0,
            max_x: 1.0,
            min_y: -0.7,
            max_y: 0.7,
        };
        scene.materials[0].mass_density = ScalarField::constant(2.0);
        scene.materials[0].stiffness = ScalarField::constant(0.5);
        scene.outer_boundaries =
            OuterBoundaryConditions::uniform(OuterBoundaryCondition::Reflecting);
    }
    document.model.source.enabled = false;
    document.presentation.adaptation.enabled = false;
    document.presentation.grid_scale_filter = filtered;
    document
}

/// Prepares `document` at `edge` as the app prepares a generation.
fn prepare(document: &TopologyDocument, edge: f64) -> Arc<PreparedTopology> {
    let mut runtime = TopologyRuntime::default();
    let editor = TopologyEditor::from_document(document.clone()).expect("the document opens");
    let domain = document.model.accepted.geometry.domain;
    let options = MeshingOptions {
        target_edge_length: edge,
        curve_tolerance: (edge * 0.02).min(5e-4),
        ..MeshingOptions::default()
    }
    .sized_for_area(domain.width() * domain.height());
    let token = runtime
        .request(
            1,
            &editor.document,
            editor.compiled_accepted.clone(),
            options,
            true,
        )
        .expect("the generation is requested");
    loop {
        if let Some(result) = runtime.advance(1 << 16) {
            result.expect("the generation prepares");
            break;
        }
    }
    runtime.commit_ready(token).expect("the generation commits")
}

/// `modes`, each `(mode, amplitude)`, at `PHASE` from the closed forms:
/// `Q = M φ cos θ`, and b sampled as `η R∇φ sin θ / ω`, less its part in the
/// coupling's kernel.
fn mode_state(
    prepared: &PreparedTopology,
    physics: PhysicsModel,
    modes: &[(BoxMode, f64)],
    integrator: CanonicalIntegrator,
    dt: f64,
) -> CanonicalWaveState {
    let operator = &prepared.canonical_operator;
    let (speed, orientation) = law(physics);
    let primary = operator
        .node_points()
        .iter()
        .zip(operator.primary_mass())
        .map(|(point, mass)| {
            let field = modes
                .iter()
                .map(|(mode, amplitude)| amplitude * mode.shape(*point))
                .sum::<f64>();
            mass * field * PHASE.cos()
        })
        .collect::<Vec<_>>();
    let sampled = operator
        .constitutive_samples()
        .iter()
        .map(|sample| {
            modes
                .iter()
                .fold(Point2::default(), |flux, (mode, amplitude)| {
                    let gradient = mode.gradient(sample.point);
                    let scale = orientation * amplitude * PHASE.sin() / (speed * mode.wavenumber());
                    flux + Point2::new(-gradient.y, gradient.x) * scale
                })
        })
        .collect::<Vec<_>>();
    let stationary = operator
        .stationary_complementary_component(&sampled)
        .expect("the sampled flux's stationary part");
    let flux = sampled
        .iter()
        .zip(stationary)
        .map(|(sampled, stationary)| *sampled - stationary)
        .collect();
    CanonicalWaveState::new(operator, dt, primary, flux)
        .expect("the modes' state")
        .with_integrator(integrator)
}

/// What one sample reads: each mode's projection, the energy, the clock,
/// the sum of `Q`, and the energies of the flux's stationary part and of the
/// uniform mode.
#[derive(Clone, Copy, Debug, Default)]
struct Sample {
    modes: [f64; 2],
    energy: f64,
    clock: f64,
    total: f64,
    stationary: f64,
    uniform: f64,
}

fn sample_of(
    prepared: &PreparedTopology,
    dt: f64,
    weights: &[Vec<f64>; 2],
    primary: Vec<f64>,
    complementary: Vec<Point2>,
    clock: f64,
) -> Sample {
    let operator = &prepared.canonical_operator;
    let energy_of = |primary: Vec<f64>, complementary: Vec<Point2>| {
        CanonicalWaveState::new(operator, dt, primary, complementary)
            .and_then(|state| state.energy(operator))
            .expect("a state's energy")
    };
    let project = |weights: &[f64]| weights.iter().zip(&primary).map(|(w, q)| w * q).sum();
    let modes = [project(&weights[0]), project(&weights[1])];
    let total = primary.iter().sum::<f64>();
    let masses = operator.primary_mass();
    let level = total / masses.iter().sum::<f64>();
    let uniform = energy_of(
        masses.iter().map(|mass| mass * level).collect(),
        vec![Point2::default(); complementary.len()],
    );
    let still = operator
        .stationary_complementary_component(&complementary)
        .expect("the flux's stationary part");
    let stationary = energy_of(vec![0.0; primary.len()], still);
    Sample {
        modes,
        energy: energy_of(primary, complementary),
        clock,
        total,
        stationary,
        uniform,
    }
}

#[derive(Resource)]
struct Run {
    prepared: Arc<PreparedTopology>,
    state: Option<CanonicalWaveState>,
    filtered: bool,
    dt: f64,
    weights: [Vec<f64>; 2],
    omegas: [f64; 2],
    targets: Vec<u64>,
    reference: Vec<Sample>,
    device: Vec<Sample>,
    installed: bool,
    deadline: Instant,
    finished: bool,
    failed: bool,
}

fn main() -> AppExit {
    let Arguments {
        physics,
        filtered,
        steps,
    } = arguments();
    let (speed, _) = law(physics);
    let prepared = prepare(&document(physics, filtered), 0.08);
    let operator = &prepared.canonical_operator;
    let integrator = CanonicalIntegrator::FourthOrder;
    // The step as the app paces it: the f32 at or below the integrator's
    // share of the bound, so that device and reference count the same step.
    let dt = device_time_step(integrator.recommended() * operator.maximum_time_step());
    let modes = [(BoxMode { m: 1, n: 0 }, 1.0), (BoxMode { m: 2, n: 3 }, 0.5)];
    let state = mode_state(&prepared, physics, &modes, integrator, dt);
    let weights = modes.map(|(mode, _)| {
        let shape = operator
            .node_points()
            .iter()
            .map(|point| mode.shape(*point))
            .collect::<Vec<_>>();
        let norm = shape
            .iter()
            .zip(operator.primary_mass())
            .map(|(shape, mass)| mass * shape * shape)
            .sum::<f64>();
        shape.iter().map(|shape| shape / norm).collect::<Vec<_>>()
    });
    let omegas = modes.map(|(mode, _)| speed * mode.wavenumber());
    let targets = (0..=CHECKPOINTS)
        .flat_map(|k| {
            let at = steps * k / CHECKPOINTS;
            [at, at + 1, at + 2]
        })
        .collect::<Vec<_>>();
    let clock = Instant::now();
    let mut reference = Vec::with_capacity(targets.len());
    let mut stepped = state.clone();
    let mut done = 0_u64;
    for target in &targets {
        while done < *target {
            stepped.step(operator).expect("the reference steps");
            done += 1;
            if filtered && done.is_multiple_of(funfern_core::GRID_SCALE_FILTER_CADENCE) {
                stepped
                    .apply_grid_filter(
                        operator,
                        &prepared.canonical_forcing,
                        funfern_core::CANONICAL_GRID_FILTER_STRENGTH,
                    )
                    .expect("the reference filters");
            }
        }
        reference.push(sample_of(
            &prepared,
            dt,
            &weights,
            stepped.primary_flux().to_vec(),
            stepped.complementary_flux().to_vec(),
            done as f64 * dt,
        ));
    }
    println!(
        "{physics:?}{}: {} dofs, {steps} steps of {dt:.6e} ({:.0} s), reference in {:.1} s",
        if filtered { ", filtered" } else { "" },
        operator.degrees_of_freedom(),
        steps as f64 * dt,
        clock.elapsed().as_secs_f64()
    );
    let mut app = App::new();
    app.add_plugins(DefaultPlugins.set(WindowPlugin {
        primary_window: Some(Window {
            title: "canonical GPU cavity drift".into(),
            visible: false,
            resolution: (64, 64).into(),
            ..default()
        }),
        ..default()
    }))
    .add_plugins(WaveGpuPlugin)
    .add_plugins(CanonicalWaveGpuPlugin)
    .insert_resource(Run {
        prepared,
        state: Some(state),
        filtered,
        dt,
        weights,
        omegas,
        targets,
        reference,
        device: Vec::new(),
        installed: false,
        deadline: check::deadline(1800),
        finished: false,
        failed: false,
    })
    .add_systems(Startup, |mut commands: Commands| {
        commands.spawn(Camera2d);
    })
    .add_systems(Update, drive)
    .run()
}

fn drive(
    mut commands: Commands,
    mut assets: ResMut<Assets<ShaderBuffer>>,
    mut request: ResMut<CanonicalGpuRequest>,
    display: Res<CanonicalGpuDisplay>,
    mut run: ResMut<Run>,
    mut exit: MessageWriter<AppExit>,
) {
    if run.finished {
        exit.write(if run.failed {
            AppExit::error()
        } else {
            AppExit::Success
        });
        return;
    }
    if Instant::now() >= run.deadline || request.stats().failure() != 0 {
        eprintln!(
            "the device timed out or failed after {} steps (status {})",
            request.stats().completed_steps(),
            request.stats().failure()
        );
        run.failed = true;
        run.finished = true;
        return;
    }
    if !run.installed {
        let state = run.state.take().expect("one state");
        let plan = CanonicalGpuPlan::compile_with_quadratic(
            &run.prepared.canonical_operator,
            &run.prepared.operator,
            &state,
            &run.prepared.canonical_forcing,
            CanonicalGpuClock::initial(run.dt).expect("the clock"),
        )
        .expect("the plan");
        // Every readback full: where several land in a frame, as on
        // lavapipe, waiting for the newest to be a full one starves.
        request
            .set_continuous_full_state_readback(true)
            .expect("select validation readback mode");
        request.install(&mut assets, &mut commands, plan);
        request.set_grid_scale_filter(run.filtered);
        run.installed = true;
        return;
    }
    let target = run.targets[run.device.len()];
    if request.stats().completed_steps() < target {
        return;
    }
    if display.generation != request.generation()
        || !display.holds_snapshot_at(target)
        || display.primary_flux.len() != run.weights[0].len()
    {
        request.request_full_state_readback();
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
    let clock = display
        .clock
        .map_or(f64::NAN, |clock| clock.absolute_seconds);
    let sample = sample_of(
        &run.prepared,
        run.dt,
        &run.weights,
        primary,
        complementary,
        clock,
    );
    run.device.push(sample);
    if run.device.len() < run.targets.len() {
        let next = run.targets[run.device.len()];
        request.request_steps(next - target);
        return;
    }
    run.failed = !report(&run);
    run.finished = true;
}

/// A mode's amplitude and phase at a checkpoint from its three samples.
fn phasor(samples: &[Sample], mode: usize, omega_dt: f64) -> (f64, f64) {
    let (a, b, c) = (
        samples[0].modes[mode],
        samples[1].modes[mode],
        samples[2].modes[mode],
    );
    let sine = (c - a) / (2.0 * omega_dt.sin());
    (b.hypot(sine), (-sine).atan2(b))
}

/// Prints each checkpoint and judges the run; true when every demand holds.
fn report(run: &Run) -> bool {
    let start = run.reference[0].energy;
    let wrap = |x: f64| {
        (x + std::f64::consts::PI).rem_euclid(std::f64::consts::TAU) - std::f64::consts::PI
    };
    let total_steps = run.targets[run.targets.len() - 3];
    let mut worst = [0.0_f64; 7];
    let mut held = true;
    println!(
        "{:>9} {:>11} {:>11} {:>11} {:>11} {:>11} {:>11} {:>11} {:>11} {:>11}",
        "steps",
        "clock lead",
        "A10 d/c-1",
        "ph10",
        "A23 d/c-1",
        "ph23",
        "E d-c",
        "stationary",
        "total d-c",
        "uniform"
    );
    for k in 0..run.targets.len() / 3 {
        let device = &run.device[3 * k..3 * k + 3];
        let reference = &run.reference[3 * k..3 * k + 3];
        let steps = run.targets[3 * k];
        let lead = device[0].clock - reference[0].clock;
        worst[0] = check::worse(worst[0], lead.abs());
        // A frequency's error, from a tenth of the run on, where it stands
        // above the state's own rounding.
        let judged = steps >= total_steps / 10;
        let mut line = format!("{steps:>9} {lead:>+11.3e}");
        for mode in 0..2 {
            let omega_dt = run.omegas[mode] * run.dt;
            let (read, read_phase) = phasor(device, mode, omega_dt);
            let (exact, exact_phase) = phasor(reference, mode, omega_dt);
            let phase = wrap(read_phase - exact_phase);
            let amplitude = read / exact - 1.0;
            if judged {
                let turned = run.omegas[mode] * reference[0].clock;
                worst[1] = check::worse(worst[1], (phase / turned).abs());
            }
            worst[2] = check::worse(worst[2], amplitude.abs());
            line += &format!(" {amplitude:>+11.3e} {phase:>+11.3e}");
        }
        let energy = (device[1].energy - reference[1].energy) / start;
        let stationary = device[1].stationary / start;
        let total = device[1].total - reference[1].total;
        let uniform = device[1].uniform / start;
        worst[3] = check::worse(worst[3], energy.abs());
        worst[4] = check::worse(worst[4], stationary);
        worst[5] = check::worse(worst[5], total.abs());
        worst[6] = check::worse(worst[6], uniform);
        println!("{line} {energy:>+11.3e} {stationary:>11.3e} {total:>+11.3e} {uniform:>11.3e}");
    }
    for (what, error, bound) in [
        ("the clock's distance from the steps taken", worst[0], CLOCK),
        (
            "a mode's frequency at the device's clock",
            worst[1],
            FREQUENCY,
        ),
        ("a mode's amplitude", worst[2], AMPLITUDE),
        ("the energy", worst[3], ENERGY),
        ("the stationary flux's energy", worst[4], STATIONARY),
        ("the sum of Q", worst[5], TOTAL),
        ("the uniform mode's energy", worst[6], UNIFORM),
    ] {
        let verdict = if within(error, bound) {
            "holds"
        } else {
            held = false;
            "FAILS"
        };
        println!("{what}: {error:.3e} against {bound:.0e}, {verdict}");
    }
    held
}
