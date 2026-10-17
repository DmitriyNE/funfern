//! PH-01 on the device: the modes of a reflecting 2 × 1.4 box in one
//! physics model, `--physics mechanical` (the default), `tm` or `te`, each
//! started at a third of a period with Q and b from the closed forms, as
//! `box_modes_in_*` in `topology_examples.rs` runs them in f64. Every run
//! steps on the device and in f64 from the same state, and the device's
//! series, the mode's mass-weighted projection read every few steps, must
//! stand within 5e-6 of the reference's. Its frequency, read from the
//! series, is then held to the reference's: in space (1,2) and (2,3) at
//! edges 0.16, 0.08 and 0.04 under the fourth-order step, and in time (2,3)
//! at 0.08 under leapfrog at 1, ½ and ¼ of its step, where each halving
//! takes three quarters of the frequency's miss away. Where the device's
//! frequency stops following the reference's, its f32 floor, is reported.
//!
//! Exits 0 when every demand holds.

#[path = "support/check.rs"]
mod check;
use bevy::{app::AppExit, prelude::*, render::storage::ShaderBuffer};
use check::within;
use funfern_app::canonical_gpu::{
    CanonicalGpuClock, CanonicalGpuDisplay, CanonicalGpuPlan, CanonicalGpuRequest,
    CanonicalWaveGpuPlugin,
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

/// How far the device's series may stand from the reference's, over the
/// series' largest value, as the end-to-end fixtures hold the field.
const TOLERANCE: f64 = 5.0e-6;
/// How far the device's frequency may stand from the reference's,
/// relatively: its f32 floor, 5e-9 to 2.8e-8 measured on the M1 Max on
/// 2026-10-13, under the frequency's own error at every mesh PH-01 runs.
const FREQUENCY_FLOOR: f64 = 1.0e-7;
/// The phase the modes start at.
const PHASE: f64 = std::f64::consts::FRAC_PI_3;
/// Samples a period of the mode the series takes.
const SAMPLES_PER_PERIOD: f64 = 12.0;
/// Periods each run lasts.
const PERIODS: f64 = 3.0;
const EDGES: [f64; 3] = [0.16, 0.08, 0.04];

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

fn physics() -> PhysicsModel {
    let mut args = std::env::args().skip(1);
    let mut physics = PhysicsModel::Mechanical;
    while let Some(arg) = args.next() {
        if arg == "--physics" {
            physics = match args.next().as_deref() {
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
    }
    physics
}

/// The reflecting 2 × 1.4 box of the medium in `physics`, no source, no grid
/// filter, adaptation off.
fn document(physics: PhysicsModel) -> TopologyDocument {
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
    document.presentation.grid_scale_filter = false;
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

/// `mode` at `PHASE`, from the closed forms: `Q = M φ cos θ`, and b sampled
/// as `η R∇φ sin θ / ω`, less its part in the coupling's kernel.
fn mode_state(
    prepared: &PreparedTopology,
    physics: PhysicsModel,
    mode: BoxMode,
    integrator: CanonicalIntegrator,
    dt: f64,
) -> CanonicalWaveState {
    let operator = &prepared.canonical_operator;
    let (speed, orientation) = law(physics);
    let primary = operator
        .node_points()
        .iter()
        .zip(operator.primary_mass())
        .map(|(point, mass)| mass * mode.shape(*point) * PHASE.cos())
        .collect::<Vec<_>>();
    let scale = orientation * PHASE.sin() / (speed * mode.wavenumber());
    let sampled = operator
        .constitutive_samples()
        .iter()
        .map(|sample| {
            let gradient = mode.gradient(sample.point);
            Point2::new(-gradient.y, gradient.x) * scale
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
        .expect("the mode's state")
        .with_integrator(integrator)
}

/// One run: a mode on one mesh under one integrator at one step, its state,
/// and the reference's series.
struct Run {
    label: String,
    kind: Kind,
    mode: BoxMode,
    edge: f64,
    integrator: CanonicalIntegrator,
    fraction: f64,
    dt: f64,
    stride: u64,
    prepared: Arc<PreparedTopology>,
    state: CanonicalWaveState,
    /// `φ` at the nodes, over `Σ M φ²`: the projection is `Σ φ Q / Σ M φ²`.
    weights: Vec<f64>,
    reference: Vec<f64>,
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Space,
    Time,
}

impl Run {
    fn new(
        physics: PhysicsModel,
        kind: Kind,
        prepared: Arc<PreparedTopology>,
        edge: f64,
        mode: BoxMode,
        integrator: CanonicalIntegrator,
        fraction: f64,
    ) -> Self {
        let operator = &prepared.canonical_operator;
        let dt = if kind == Kind::Space {
            prepared.recommended_time_step()
        } else {
            fraction * integrator.recommended() * operator.maximum_time_step()
        };
        let state = mode_state(&prepared, physics, mode, integrator, dt);
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
        let weights = shape.iter().map(|shape| shape / norm).collect::<Vec<_>>();
        let (speed, _) = law(physics);
        let period = std::f64::consts::TAU / (speed * mode.wavenumber());
        let stride = ((period / dt / SAMPLES_PER_PERIOD).round() as u64).max(1);
        let samples = (PERIODS * period / dt / stride as f64).ceil() as usize + 1;
        let mut reference = Vec::with_capacity(samples);
        let mut stepped = state.clone();
        for sample in 0..samples {
            if sample > 0 {
                for _ in 0..stride {
                    stepped.step(operator).expect("the reference steps");
                }
            }
            reference.push(project(&weights, stepped.primary_flux()));
        }
        let label = match kind {
            Kind::Space => format!("({},{}) at edge {edge}", mode.m, mode.n),
            Kind::Time => format!(
                "({},{}) {integrator:?} at {fraction} of its step",
                mode.m, mode.n
            ),
        };
        Self {
            label,
            kind,
            mode,
            edge,
            integrator,
            fraction,
            dt,
            stride,
            prepared,
            state,
            weights,
            reference,
        }
    }

    /// The frequency of a series sampled every `stride` steps, from the
    /// recurrence `s(j+1) + s(j−1) = 2 cos(stride ω dt) s(j)` fitted in least
    /// squares.
    fn frequency(&self, series: &[f64]) -> f64 {
        let (mut numerator, mut denominator) = (0.0, 0.0);
        for window in series.windows(3) {
            numerator += window[1] * (window[0] + window[2]);
            denominator += 2.0 * window[1] * window[1];
        }
        (numerator / denominator).acos() / (self.stride as f64 * self.dt)
    }

    /// The semidiscrete frequency a stepped one came from, by the
    /// integrator's dispersion relation.
    fn semidiscrete(&self, stepped: f64) -> f64 {
        let dt = self.dt;
        match self.integrator {
            CanonicalIntegrator::Leapfrog => 2.0 * (0.5 * stepped * dt).sin() / dt,
            CanonicalIntegrator::FourthOrder => {
                let x_squared = 6.0 - (36.0 - 24.0 * (1.0 - (stepped * dt).cos())).sqrt();
                x_squared.sqrt() / dt
            }
        }
    }
}

fn project(weights: &[f64], primary: &[f64]) -> f64 {
    weights.iter().zip(primary).map(|(w, q)| w * q).sum()
}

#[derive(Resource)]
struct Study {
    physics: PhysicsModel,
    runs: Vec<Run>,
    current: usize,
    installed: bool,
    series: Vec<f64>,
    done: Vec<Vec<f64>>,
    started: Instant,
    deadline: Instant,
    finished: bool,
    failed: bool,
}

fn main() -> AppExit {
    let physics = physics();
    let (speed, _) = law(physics);
    let document = document(physics);
    let modes = [BoxMode { m: 1, n: 2 }, BoxMode { m: 2, n: 3 }];
    let mut runs = Vec::new();
    for edge in EDGES {
        let prepared = prepare(&document, edge);
        for mode in modes {
            runs.push(Run::new(
                physics,
                Kind::Space,
                prepared.clone(),
                edge,
                mode,
                CanonicalIntegrator::FourthOrder,
                1.0,
            ));
        }
        if edge == 0.08 {
            for fraction in [1.0, 0.5, 0.25] {
                runs.push(Run::new(
                    physics,
                    Kind::Time,
                    prepared.clone(),
                    edge,
                    modes[1],
                    CanonicalIntegrator::Leapfrog,
                    fraction,
                ));
            }
        }
    }
    println!(
        "{physics:?}: speed {speed}, {} runs prepared in f64",
        runs.len()
    );
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
    .insert_resource(Study {
        physics,
        runs,
        current: 0,
        installed: false,
        series: Vec::new(),
        done: Vec::new(),
        started: Instant::now(),
        deadline: check::deadline(600),
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
    mut study: ResMut<Study>,
    mut exit: MessageWriter<AppExit>,
) {
    if study.finished {
        exit.write(if study.failed {
            AppExit::error()
        } else {
            AppExit::Success
        });
        return;
    }
    if Instant::now() >= study.deadline || request.stats().failure() != 0 {
        eprintln!("the device timed out or failed");
        study.failed = true;
        study.finished = true;
        return;
    }
    if !study.installed {
        let run = &study.runs[study.current];
        let plan = CanonicalGpuPlan::compile_with_quadratic(
            &run.prepared.canonical_operator,
            &run.prepared.operator,
            &run.state,
            &run.prepared.canonical_forcing,
            CanonicalGpuClock::initial(run.dt).expect("the clock"),
        )
        .expect("the plan");
        request.install(&mut assets, &mut commands, plan);
        study.installed = true;
        study.series.clear();
        return;
    }
    let current = study.current;
    let stride = study.runs[current].stride;
    let samples = study.runs[current].reference.len();
    let target = study.series.len() as u64 * stride;
    let completed = request.stats().completed_steps();
    if completed < target {
        return;
    }
    if display.generation != request.generation()
        || display.full_readbacks == 0
        || display.full_readback_at != display.readbacks
        || display.full_snapshot_completed_steps() != target
        || display.primary_flux.len() != study.runs[current].weights.len()
    {
        request.request_full_state_readback();
        return;
    }
    let primary = display
        .primary_flux
        .iter()
        .map(|value| f64::from(*value))
        .collect::<Vec<_>>();
    let value = project(&study.runs[current].weights, &primary);
    study.series.push(value);
    if study.series.len() < samples {
        request.request_steps(stride);
        return;
    }
    let series = std::mem::take(&mut study.series);
    study.done.push(series);
    study.current += 1;
    study.installed = false;
    if study.current == study.runs.len() {
        study.failed = !report(&study);
        study.finished = true;
    }
}

/// Reads every run against its reference and the closed form, prints the
/// table and says whether every demand holds.
fn report(study: &Study) -> bool {
    let (speed, _) = law(study.physics);
    let mut kept = true;
    let mut space = Vec::new();
    let mut time = Vec::new();
    for (run, device) in study.runs.iter().zip(&study.done) {
        let largest = run
            .reference
            .iter()
            .fold(0.0_f64, |most, value| most.max(value.abs()));
        let apart = device
            .iter()
            .zip(&run.reference)
            .map(|(device, reference)| (device - reference).abs())
            .fold(0.0_f64, f64::max)
            / largest;
        let (device_frequency, reference_frequency) =
            (run.frequency(device), run.frequency(&run.reference));
        let frequency_apart = (device_frequency / reference_frequency - 1.0).abs();
        let exact = speed * run.mode.wavenumber();
        let device_error = run.semidiscrete(device_frequency) / exact - 1.0;
        let reference_error = run.semidiscrete(reference_frequency) / exact - 1.0;
        println!(
            "{}: series {apart:.2e} from f64, frequency {frequency_apart:.2e} from f64; \
             ω_h off the closed form {device_error:.3e} on the device, {reference_error:.3e} in f64",
            run.label
        );
        if !within(apart, TOLERANCE) || !within(frequency_apart, FREQUENCY_FLOOR) {
            eprintln!("{}: the device leaves the reference", run.label);
            kept = false;
        }
        match run.kind {
            Kind::Space => space.push((run.mode.n, run.edge, device_error.abs())),
            Kind::Time => time.push((run.fraction, device_frequency, run.dt)),
        }
    }
    // Space: the device's semidiscrete frequency falls with the mesh as the
    // reference's does, at least sixfold each halving: its floor lies under
    // the finest mesh's error.
    for n in [2, 3] {
        let errors = space
            .iter()
            .filter(|(mode, _, _)| *mode == n)
            .map(|(_, _, error)| *error)
            .collect::<Vec<_>>();
        println!(
            "mode n = {n}: the device's ω_h off the closed form {}",
            errors
                .iter()
                .map(|error| format!("{error:.2e}"))
                .collect::<Vec<_>>()
                .join(", ")
        );
        let falling = errors[0] / errors[1] > 6.0 && errors[1] / errors[2] > 6.0;
        if !falling {
            eprintln!("mode n = {n}: the device's frequency does not fall with the mesh");
            kept = false;
        }
    }
    // Time: leapfrog's stepped frequency, `ω_h + ω_h³dt²/24`, loses three
    // quarters of its miss with each halving of the step, so successive
    // differences fall fourfold.
    let first = time[0].1 - time[1].1;
    let second = time[1].1 - time[2].1;
    println!(
        "leapfrog: successive differences {first:.3e} and {second:.3e}, ratio {:.3}",
        first / second
    );
    if !within((first / second - 4.0).abs(), 0.1) {
        eprintln!("leapfrog's stepped frequency does not fall at second order on the device");
        kept = false;
    }
    println!(
        "{:?} done in {:.1} s",
        study.physics,
        study.started.elapsed().as_secs_f64()
    );
    kept
}
