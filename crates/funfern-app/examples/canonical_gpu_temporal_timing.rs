//! Steady-stepping throughput of the f32 GPU core, driven against fixed.
//!
//! The CPU oracle's driven path costs about thirteen times its fixed path, but
//! that oracle is not what anyone runs. The production core is this one, whose
//! shader reads a stage's coefficients once where the reference recomputes them
//! in every helper that wants them. Whether the drive is affordable is decided
//! here and nowhere else.
//!
//! Pass `--fixed` for the comparison run; everything else about the fixture is
//! identical, including the mesh, the operator and the timestep.
//!
//! `--outgoing` swaps the first-order wall for a second-order one, which is the
//! only composition whose stage does non-local work. Its trace system carries
//! no nodal mass, so a driven generation sweeps it with the stage's own mass
//! rather than refactorizing, and this is where that sweep's cost is read.
//!
//! `--nonlinear` adds Kerr on the mass row and saturation on the stiffness row
//! to the driven medium, at an amplitude where both maps depart from linear,
//! so the step pays every inverse and, with `--outgoing`, the wall's Newton.
//!
//! `--filter` turns the resident grid filter on, which runs every sixteenth
//! step, so the figure includes its amortized cost.
//!
//! `--oscillator` adds a sine-Gordon restoring law (Gate O), so each step
//! also drifts the integrated field and gathers its restoring force;
//! `--van-der-pol` adds a van der Pol primary loss on top, so the loss stages
//! run the Bernoulli map. `--loss` puts a constant primary loss there instead,
//! which runs the same two loss stages without the map, so the two separate
//! what the stages cost from what the map costs. The restoring law's
//! curvature lowers the step ceiling, so the per-step figure is the
//! comparison, not the wall clock.
//!
//! `--gated` puts both drives under a Gaussian gate wide enough to cover the
//! run, so every factor pays for the envelope's exponential: the dearest a
//! gate gets.
//!
//! `--short-wave` adds an authored short-wave loss α = 0.5, so the step runs
//! the short-wave split's seven passes and the first kick applies its first
//! half. Van der Pol carries the same split already.
//!
//! Stepping is unfenced, so the figure is what a step costs the device rather
//! than the readback round trip the interactive lead fence waits on.
//!
//! `--steps=N` times `N` steps instead of 2,000. A run of a second or less
//! can end before the device's clock settles, and then two runs of the same
//! build read apart by a sixth; at 20,000 repeat runs agreed to about 2%,
//! with an occasional slow first run after a heavy scene, so compare medians
//! of runs whose order rotates.
//!
//! `--checksum` times nothing: it runs exactly `STEPS` steps, reads the whole
//! state back and prints a hash of its bits, `Q`, `b`, `r` and the accounting
//! lanes, so a change meant to leave the arithmetic alone can be held to the
//! same bits on every fixture above.

use std::time::{Duration, Instant};

use bevy::{app::AppExit, prelude::*, render::storage::ShaderBuffer};
use funfern_app::canonical_gpu::{
    CanonicalGpuClock, CanonicalGpuDisplay, CanonicalGpuPlan, CanonicalGpuRequest,
    CanonicalWaveGpuPlugin,
};
use funfern_app::wave_gpu::WaveGpuPlugin;
use funfern_core::{
    CanonicalForcing, CanonicalTemporalWaveOperator, CanonicalTemporalWaveState, CoefficientLaw,
    MeshingOptions, OuterBoundaryCondition, QuadraticWaveOperator, ScalarField, Scene, TimeDrive,
    mesh_scene,
};

const STEPS: u64 = 2_000;
const EDGE: f64 = 0.06;

#[derive(Resource)]
struct Pending {
    plan: Option<CanonicalGpuPlan>,
    filter: bool,
}

#[derive(Resource)]
struct Timing {
    label: String,
    checksum: bool,
    steps: u64,
    nodes: usize,
    time_step: f64,
    started: Option<Instant>,
    deadline: Instant,
    finished: bool,
    /// The run timed out or the device failed, which exits with an error: a
    /// timing taken from a failed run is no timing.
    failed: bool,
}

fn main() -> AppExit {
    let driven = !std::env::args().any(|argument| argument == "--fixed");
    let outgoing = std::env::args().any(|argument| argument == "--outgoing");
    let nonlinear = std::env::args().any(|argument| argument == "--nonlinear");
    let filter = std::env::args().any(|argument| argument == "--filter");
    let van_der_pol = std::env::args().any(|argument| argument == "--van-der-pol");
    let constant_loss = std::env::args().any(|argument| argument == "--loss");
    let oscillator = van_der_pol || std::env::args().any(|argument| argument == "--oscillator");
    let gated = std::env::args().any(|argument| argument == "--gated");
    let short_wave = std::env::args().any(|argument| argument == "--short-wave");
    let checksum = std::env::args().any(|argument| argument == "--checksum");
    let steps = std::env::args()
        .find_map(|argument| argument.strip_prefix("--steps=")?.parse().ok())
        .unwrap_or(STEPS);
    let amplitude = if nonlinear { 12.0 } else { 1.0 };
    let mut scene = Scene::initial();
    if driven {
        scene.materials[0].mass_law.drive = TimeDrive::ParametricPump {
            depth: ScalarField::constant(0.22),
            frequency_hz: ScalarField::constant(0.9),
            phase_radians: ScalarField::constant(0.2),
        };
        scene.materials[0].stiffness_law.drive = TimeDrive::TravellingModulation {
            depth: ScalarField::constant(0.15),
            frequency_hz: ScalarField::constant(0.7),
            phase_radians: ScalarField::constant(-0.1),
            wavenumber: ScalarField::constant(2.0),
            angle_radians: ScalarField::constant(0.4),
        };
    }
    if gated {
        let gate = Some(funfern_core::PulseTrain {
            envelope: funfern_core::PulseEnvelope::Gaussian { width: 1.0e3 },
            start: -4.0e3,
            repeat: 0.0,
        });
        scene.materials[0].mass_law.gate = gate;
        scene.materials[0].stiffness_law.gate = gate;
    }

    if nonlinear {
        scene.materials[0].mass_law.field = funfern_core::FieldLaw::Polynomial {
            chi1: ScalarField::constant(0.0),
            chi2: ScalarField::constant(0.8),
            amplitude_bound: None,
        };
        scene.materials[0].stiffness_law.field = funfern_core::FieldLaw::Saturable {
            chi: ScalarField::constant(6.0),
            saturation: ScalarField::constant(0.3),
        };
    }
    if oscillator {
        scene.materials[0].restoring = funfern_core::RestoringLaw::SineGordon {
            omega0: ScalarField::constant(3.0),
        };
    }
    if van_der_pol {
        scene.materials[0].magnetic_loss = Some(funfern_core::LossChannel {
            base_rate: ScalarField::constant(0.8),
            law: funfern_core::DampingLaw {
                rate: funfern_core::RateLaw::VanDerPol {
                    threshold: ScalarField::constant(0.4),
                    amplitude_bound: ScalarField::constant(10.0),
                },
                drive: TimeDrive::None,
                gate: None,
            },
        });
    }
    if constant_loss {
        scene.materials[0].magnetic_loss = Some(funfern_core::LossChannel {
            base_rate: ScalarField::constant(0.8),
            law: funfern_core::DampingLaw {
                rate: funfern_core::RateLaw::Constant,
                drive: TimeDrive::None,
                gate: None,
            },
        });
    }
    if short_wave {
        scene.materials[0].short_wave_loss = 0.5;
    }
    let mut fixed_scene = scene.clone();
    for material in &mut fixed_scene.materials {
        material.mass_law = CoefficientLaw::linear();
        material.stiffness_law = CoefficientLaw::linear();
        material.restoring = funfern_core::RestoringLaw::None;
    }
    let mesh = mesh_scene(
        &fixed_scene,
        1,
        MeshingOptions {
            target_edge_length: EDGE,
            ..MeshingOptions::default()
        },
    )
    .expect("timing mesh");
    let scalar = QuadraticWaveOperator::assemble_scene(
        &mesh,
        &fixed_scene,
        if outgoing {
            OuterBoundaryCondition::SecondOrderOutgoing
        } else {
            OuterBoundaryCondition::FirstOrderOutgoing
        },
    )
    .expect("timing scalar operator");
    let operator = CanonicalTemporalWaveOperator::compile_scene(&mesh, &scalar, &scene, 1)
        .expect("timing temporal operator");
    let base = operator.base();

    // The driven ceiling is the lower of the two, so both runs use it and the
    // comparison is per step at a matched timestep.
    let time_step = 0.4 * operator.maximum_time_step();
    let primary = base
        .primary_mass()
        .iter()
        .zip(base.node_points())
        .map(|(mass, point)| mass * (amplitude * 0.03 * (1.3 * point.x - 0.7 * point.y).sin()))
        .collect::<Vec<_>>();
    let potential = base
        .node_points()
        .iter()
        .map(|point| amplitude * 0.02 * (0.9 * point.x + 1.1 * point.y).cos())
        .collect::<Vec<_>>();
    let complementary = base.compatible_flux(&potential).expect("timing flux");
    let state = CanonicalTemporalWaveState::new(&operator, time_step, primary, complementary)
        .expect("timing state");
    let clock = CanonicalGpuClock::initial(time_step).expect("timing clock");
    let forcing = CanonicalForcing::none(base);
    let plan = CanonicalGpuPlan::compile_temporal(&operator, &state, &forcing, clock)
        .expect("timing GPU plan");

    if let Some(boundary) = base.outgoing_boundary() {
        println!(
            "gpu timing: {} trace nodes solved in {} sweeps",
            boundary.trace_nodes().len(),
            plan.trace_sweeps,
        );
    }
    let label = match (driven, nonlinear, oscillator, van_der_pol) {
        (_, _, _, true) => "van der Pol",
        (_, _, true, _) => "oscillator",
        (_, true, _, _) => "nonlinear",
        (true, false, _, _) => "driven",
        (false, false, _, _) => "fixed",
    };
    let label = if short_wave {
        format!("{label}, short-wave loss")
    } else {
        label.to_string()
    };
    println!(
        "gpu {label} timing: {} Q, {} b, dt {time_step:.4e}, {steps} steps",
        plan.node_count, plan.sample_count
    );

    let mut app = App::new();
    app.add_plugins(DefaultPlugins.set(WindowPlugin {
        primary_window: Some(Window {
            title: "funfern gpu temporal timing".into(),
            visible: false,
            resolution: (64, 64).into(),
            ..default()
        }),
        ..default()
    }))
    .add_plugins(WaveGpuPlugin)
    .add_plugins(CanonicalWaveGpuPlugin)
    .insert_resource(Pending {
        plan: Some(plan),
        filter,
    })
    .insert_resource(Timing {
        label,
        checksum,
        steps,
        nodes: base.degrees_of_freedom(),
        time_step,
        started: None,
        deadline: Instant::now() + Duration::from_secs(300),
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
    timing: Res<Timing>,
) {
    canonical.set_unfenced_stepping(true);
    canonical.set_grid_scale_filter(pending.filter);
    canonical.install(
        &mut assets,
        &mut commands,
        pending.plan.take().expect("one plan"),
    );
    if timing.checksum {
        canonical.request_steps(STEPS);
    }
    commands.spawn(Camera2d);
}

fn drive(
    mut request: ResMut<CanonicalGpuRequest>,
    display: Res<CanonicalGpuDisplay>,
    mut timing: ResMut<Timing>,
    mut exit: MessageWriter<AppExit>,
) {
    if timing.finished {
        exit.write(if timing.failed {
            AppExit::error()
        } else {
            AppExit::Success
        });
        return;
    }
    if Instant::now() >= timing.deadline || request.stats().failure() != 0 {
        eprintln!("gpu timing timed out or failed");
        timing.failed = true;
        timing.finished = true;
        return;
    }
    if timing.checksum {
        if request.stats().completed_steps() < STEPS
            || (!request.request_full_state_readback() && display.full_readbacks == 0)
            || display.primary_flux.len() != timing.nodes
        {
            return;
        }
        println!(
            "gpu {} checksum after {STEPS} steps: {:016x}",
            timing.label,
            state_hash(&display)
        );
        timing.finished = true;
        return;
    }
    if timing.started.is_none() {
        // Let the first frames settle before the clock starts, so pipeline
        // creation and the first buffer upload are not charged to stepping.
        if display.clock.is_none_or(|clock| clock.accepted_steps < 32) {
            request.request_steps(64);
            return;
        }
        timing.started = Some(Instant::now());
        request.request_steps(timing.steps);
        return;
    }
    if request.stats().completed_steps() < timing.steps + 32 {
        return;
    }
    let elapsed = timing.started.expect("started").elapsed().as_secs_f64();
    let per_step = elapsed * 1.0e6 / timing.steps as f64;
    let simulated = timing.time_step * timing.steps as f64;
    println!(
        "gpu {}: {} DOFs, {per_step:.1} us/step, {:.1} simulated s per wall s",
        timing.label,
        timing.nodes,
        simulated / elapsed
    );
    timing.finished = true;
}

/// FNV-1a over the bits of every lane the readback carries.
fn state_hash(display: &CanonicalGpuDisplay) -> u64 {
    let words = display
        .primary_flux
        .iter()
        .chain(display.complementary_flux.iter().flatten())
        .chain(display.integrated_field())
        .chain(&display.accounting)
        .chain([&display.active_gain]);
    words.fold(0xcbf2_9ce4_8422_2325, |hash, value| {
        value
            .to_bits()
            .to_le_bytes()
            .iter()
            .fold(hash, |hash, byte| {
                (hash ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3)
            })
    })
}
