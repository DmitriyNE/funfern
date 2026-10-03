//! Does the estimator's relative-error number mean the same thing whatever
//! the medium, the boundaries and the field's frequency?
//!
//! The `6%` target was calibrated by watching a production run settle, not
//! against a known answer, and the material-law review forbids quoting that
//! number for time-driven media without validating it again. This is that
//! validation: the efficiency index, estimator over true error, across a
//! refinement sequence, and the constant that sets its level.
//!
//! True error needs a reference, and the study has to be clean enough that
//! the reference converges. The domain is a bare rectangle and the initial
//! data is a reflecting-box mode, so it satisfies the boundary conditions
//! exactly and stays smooth: a curved hole or data that fights the walls
//! limits convergence by geometry and makes an efficiency index meaningless.
//! Every mesh starts from the same analytic data and runs with the finest
//! mesh's timestep, so the temporal error is common to all of them. What is
//! left is spatial discretization error, which is what the estimator claims
//! to measure.

use std::sync::Arc;
use std::time::Instant;

use funfern_core::{
    BACKGROUND_REGION, CanonicalForcing, CanonicalIndicatorSnapshot, CanonicalTemporalPointStencil,
    CanonicalTemporalWaveOperator, CanonicalTemporalWaveState, CoefficientLaw, DampingLaw,
    FieldLaw, LoopRole, LossChannel, Material, MaterialFrame, MaterialId, MeshingOptions,
    OuterBoundaryCondition, OuterBoundaryConditions, OuterSide, Point2, QuadraticPointStencil,
    QuadraticSolutionSnapshot, QuadraticWaveOperator, RateLaw, Region, RegionId, RestoringLaw,
    ScalarField, Scene, SolutionIndicatorJob, SolutionIndicatorOptions, TimeDrive, TimeSignal,
    TriMesh, canonical_temporal_indicator_supplement, mesh_scene,
};

/// Coarse to fine; the last is the reference.
const EDGES: [f64; 4] = [0.20, 0.14, 0.10, 0.05];
const TARGET_TIME: f64 = 0.35;
const LATTICE: usize = 21;
const ELEMENTS_PER_WAVELENGTH: f64 = 5.0;

struct Solved {
    edge: f64,
    degrees_of_freedom: usize,
    /// Physical primary field and complementary field at the shared lattice.
    primary: Vec<f64>,
    complementary: Vec<Point2>,
    /// Energy density at the lattice, for the norm's denominator.
    density: Vec<f64>,
    estimator: Option<f64>,
    breakdown: Option<funfern_core::SolutionIndicatorReport>,
    steps: u64,
    wall_seconds: f64,
}

fn main() {
    println!("Bare rectangle, reflecting walls, a box mode released from rest.");
    println!(
        "All meshes share the finest timestep, so what differs is space. Target time {TARGET_TIME}.\n"
    );
    println!(
        "The inert sweep is the control. Whatever the driven one does, the question\n\
         is whether modulation changes it: a number that was never a calibrated\n\
         percentage cannot be reported as having lost that property.\n"
    );

    // Row crossed with spatial pattern. A sweep that changed both at once
    // cannot say which one the estimator is charging for, and the first
    // version of this study changed both.
    //
    // The "static path" rows run the fixed generation's supplement, the others
    // the time-driven one. One estimator reads both, so on an inert medium the
    // two must read the same index; that pair is the check that they agree.
    let rows = [
        ("static path, inert", inert_scene(), true),
        ("inert", inert_scene(), false),
        ("mass pumped", driven(true, None), false),
        ("mass travelling k=0.75", driven(true, Some(0.75)), false),
        ("mass travelling k=3", driven(true, Some(3.0)), false),
        ("stiffness pumped", driven(false, None), false),
        ("stiffness travelling k=3", driven(false, Some(3.0)), false),
        ("both travelling k=3", both_scene(), false),
        ("static path, interface", interface_scene(false), true),
        ("interface, inert", interface_scene(false), false),
        ("interface, mass pumped", interface_scene(true), false),
        // Stage 8: field-dependent response, strong enough that the maps
        // depart from linear by roughly 20% at the mode's peak.
        ("mass Kerr", nonlinear(Some(kerr(30.0)), None), false),
        ("stiffness Kerr", nonlinear(None, Some(kerr(20.0))), false),
        (
            "both saturable",
            nonlinear(Some(saturable(60.0, 0.06)), Some(saturable(40.0, 0.08))),
            false,
        ),
        (
            "mass Kerr, pumped",
            {
                let mut scene = driven(true, None);
                scene.materials[0].mass_law.field = kerr(30.0);
                scene
            },
            false,
        ),
        // Stage 11: oscillator media, released from rest as a box mode in
        // the integrated field. At 1.5 rad sine-Gordon's force is 33% below
        // its tangent at the peak.
        (
            "Klein-Gordon",
            restoring(RestoringLaw::KleinGordon {
                omega0: ScalarField::constant(3.0),
            }),
            false,
        ),
        (
            "sine-Gordon",
            restoring(RestoringLaw::SineGordon {
                omega0: ScalarField::constant(3.0),
            }),
            false,
        ),
    ]
    .into_iter()
    .map(|(label, scene, static_path)| (label, scene, static_path, reflecting()))
    .chain(boundary_rows())
    .chain(
        loss_rows()
            .into_iter()
            .map(|(label, scene, static_path)| (label, scene, static_path, reflecting())),
    )
    .map(|(label, scene, static_path, walls)| (label, scene, static_path, walls, 1.0))
    .chain(self_similar_rows());
    // An argument runs only the rows whose label contains it.
    let only = std::env::args().nth(1).unwrap_or_default();
    for (label, scene, static_path, walls, scale) in rows {
        if !label.contains(only.as_str()) {
            continue;
        }
        // The finest mesh sets the timestep every mesh in the sweep uses.
        let edges = EDGES.map(|edge| edge / scale);
        let reference_edge = *edges.last().expect("one reference edge");
        let (_, _, reference_operator) = build(&scene, reference_edge, walls);
        let time_step = 0.4 * reference_operator.maximum_time_step();
        let lattice = lattice_points();
        let solved = edges
            .iter()
            .map(|edge| {
                solve(
                    &scene,
                    *edge,
                    time_step,
                    &lattice,
                    static_path,
                    walls,
                    scale,
                )
            })
            .collect::<Vec<_>>();
        let reference = solved.last().expect("a reference solution");
        println!("{label}: shared time step {time_step:.4e} from h={reference_edge}");
        println!(
            "{:<6} {:>8} {:>7} {:>12} {:>12} {:>10}",
            "h", "DOFs", "steps", "true error", "estimator", "efficiency"
        );
        let mut indices = Vec::new();
        for solution in &solved[..solved.len() - 1] {
            let truth = relative_error(solution, reference);
            let estimate = solution.estimator.unwrap_or(f64::NAN);
            indices.push(estimate / truth);
            println!(
                "{:<6} {:>8} {:>7} {:>12.4e} {:>12.4e} {:>10.3}",
                solution.edge,
                solution.degrees_of_freedom,
                solution.steps,
                truth,
                estimate,
                estimate / truth
            );
            if let Some(report) = &solution.breakdown {
                println!(
                    "       energy {:.3e} | recovery {:.3e} complementary {:.3e} jump {:.3e} drift {:.3e} boundary {:.3e}",
                    report.total_energy,
                    report.displacement_recovery_contribution,
                    report.complementary_recovery_contribution,
                    report.interior_jump_contribution,
                    report.canonical_drift_contribution,
                    report.boundary_residual_contribution
                );
                // The candidate that would replace the scalar jump, printed
                // beside it and summed into nothing. What decides the switch
                // is whether this column converges where that one stalls.
                println!(
                    "       candidate complementary jump {:.3e}",
                    report.complementary_jump_contribution
                );
            }
        }
        let spread = indices.iter().copied().fold(0.0_f64, f64::max)
            / indices.iter().copied().fold(f64::MAX, f64::min);
        println!(
            "{label}: efficiency index spans {spread:.2}x over {}x the unknowns; reference \
             h={} at {} DOFs in {:.1} s\n",
            reference.degrees_of_freedom / solved[0].degrees_of_freedom,
            reference.edge,
            reference.degrees_of_freedom,
            reference.wall_seconds
        );
    }
    println!(
        "An estimator worth reading as a percentage has an efficiency index bounded\n\
         and roughly constant under refinement. A drifting index means the number\n\
         moves with the mesh.\n\n\
         The constant is set so the smooth box mode reads the index the 6% target\n\
         was calibrated at, about 1.4. Every other row, the self-similar ones\n\
         included, should read near it: a row that does not is a medium, boundary\n\
         or frequency the estimate weighs differently."
    );
}

/// The box mode at `s` times the wavenumber, on meshes and over a run `s`
/// times shorter: `s²` tiles of the first problem, so the true relative error
/// is the same and an estimate that reads the error in its own units reads the
/// same index. One whose index moves with `s` is weighting by frequency.
fn self_similar_rows() -> Vec<(&'static str, Scene, bool, OuterBoundaryConditions, f64)> {
    vec![
        (
            "static path, inert, s=2",
            inert_scene(),
            true,
            reflecting(),
            2.0,
        ),
        ("inert, s=2", inert_scene(), false, reflecting(), 2.0),
        (
            "static path, inert, s=3",
            inert_scene(),
            true,
            reflecting(),
            3.0,
        ),
        ("inert, s=3", inert_scene(), false, reflecting(), 3.0),
    ]
}

fn reflecting() -> OuterBoundaryConditions {
    OuterBoundaryConditions::uniform(OuterBoundaryCondition::Reflecting)
}

/// Rows whose boundaries carry data, each beside its static-path control.
///
/// A prescribed side starts from the standing wave `cos(π(x+1))`, which meets
/// the reflecting sides and the left side's `cos(ωt)` data at the start, with
/// `ω` the unit medium's `π`: a box mode varies along that side and a uniform
/// signal would fight it. A Neumann side starts from the box mode, whose normal
/// derivative is zero, against data that starts at zero.
///
/// First-order walls impose `σ·n = −Z u` on the flux the direct state holds at
/// every instant, and a box mode released from rest has no flux and a field on
/// the walls, so those rows start from a `sin⁴` bump that meets the walls with
/// no field, slope or curvature. Its wave barely reaches them in the run, so a
/// channel with one absorbing end starts the bump 0.4 from that wall instead,
/// and half of it arrives there.
fn boundary_rows() -> Vec<(&'static str, Scene, bool, OuterBoundaryConditions)> {
    let mut prescribed = reflecting();
    prescribed.sides[OuterSide::Left.index()] = OuterBoundaryCondition::Dirichlet {
        signal: TimeSignal::harmonic(0.0, 0.08, 0.5, std::f64::consts::FRAC_PI_2),
    };
    let mut neumann = reflecting();
    neumann.sides[OuterSide::Left.index()] = OuterBoundaryCondition::Neumann {
        signal: TimeSignal::harmonic(0.0, 0.1, 0.7, 0.0),
    };
    let absorbing = OuterBoundaryConditions::uniform(OuterBoundaryCondition::FirstOrderOutgoing);
    let mut absorbing_end = reflecting();
    absorbing_end.sides[OuterSide::Right.index()] = OuterBoundaryCondition::FirstOrderOutgoing;
    vec![
        (
            "static path, prescribed side",
            inert_scene(),
            true,
            prescribed,
        ),
        ("prescribed side, inert", inert_scene(), false, prescribed),
        (
            "prescribed side, mass pumped",
            driven(true, None),
            false,
            prescribed,
        ),
        ("static path, Neumann side", inert_scene(), true, neumann),
        (
            "Neumann side, mass pumped",
            driven(true, None),
            false,
            neumann,
        ),
        (
            "static path, first-order walls",
            inert_scene(),
            true,
            absorbing,
        ),
        ("first-order walls, inert", inert_scene(), false, absorbing),
        (
            "first-order walls, mass pumped",
            driven(true, None),
            false,
            absorbing,
        ),
        (
            "first-order walls, mass travelling k=3",
            driven(true, Some(3.0)),
            false,
            absorbing,
        ),
        (
            "first-order walls, stiffness pumped",
            driven(false, None),
            false,
            absorbing,
        ),
        (
            "first-order walls, mass Kerr",
            nonlinear(Some(kerr(30.0)), None),
            false,
            absorbing,
        ),
        (
            "static path, absorbing end",
            inert_scene(),
            true,
            absorbing_end,
        ),
        ("absorbing end, inert", inert_scene(), false, absorbing_end),
        (
            "absorbing end, mass pumped",
            driven(true, None),
            false,
            absorbing_end,
        ),
        (
            "absorbing end, stiffness pumped",
            driven(false, None),
            false,
            absorbing_end,
        ),
        (
            "absorbing end, mass travelling k=3",
            driven(true, Some(3.0)),
            false,
            absorbing_end,
        ),
    ]
}

/// Rows carrying loss, in the reflecting box: a constant primary loss, a
/// constant complementary one - which contracts `b` itself between the step's
/// endpoints - loss beside a restoring law, and a self-oscillating medium like
/// the self-sustained emitter's, van der Pol gain beside Klein-Gordon with a
/// complementary loss.
fn loss_rows() -> Vec<(&'static str, Scene, bool)> {
    let constant_loss = |rate: f64| LossChannel {
        base_rate: ScalarField::constant(rate),
        law: DampingLaw {
            rate: RateLaw::Constant,
            drive: TimeDrive::None,
            gate: None,
        },
    };
    // The mechanical skin's complementary row carries the electric channel.
    let primary_loss = |mut scene: Scene| {
        scene.materials[0].damping = ScalarField::constant(0.45);
        scene
    };
    let complementary_loss = |mut scene: Scene| {
        scene.materials[0].electric_loss = Some(constant_loss(5.0));
        scene
    };
    let mut emitter = complementary_loss(restoring(RestoringLaw::KleinGordon {
        omega0: ScalarField::constant(3.0),
    }));
    emitter.materials[0].magnetic_loss = Some(LossChannel {
        base_rate: ScalarField::constant(4.0),
        law: DampingLaw {
            rate: RateLaw::VanDerPol {
                threshold: ScalarField::constant(5.0),
                amplitude_bound: ScalarField::constant(50.0),
            },
            drive: TimeDrive::None,
            gate: None,
        },
    });
    vec![
        (
            "static path, primary loss",
            primary_loss(inert_scene()),
            true,
        ),
        ("primary loss, inert", primary_loss(inert_scene()), false),
        (
            "primary loss, mass pumped",
            primary_loss(driven(true, None)),
            false,
        ),
        (
            "static path, complementary loss",
            complementary_loss(inert_scene()),
            true,
        ),
        (
            "complementary loss, inert",
            complementary_loss(inert_scene()),
            false,
        ),
        (
            "complementary loss, mass pumped",
            complementary_loss(driven(true, None)),
            false,
        ),
        (
            "complementary loss, mass travelling k=3",
            complementary_loss(driven(true, Some(3.0))),
            false,
        ),
        (
            "Klein-Gordon, primary loss",
            primary_loss(restoring(RestoringLaw::KleinGordon {
                omega0: ScalarField::constant(3.0),
            })),
            false,
        ),
        (
            "van der Pol beside Klein-Gordon, complementary loss",
            emitter,
            false,
        ),
    ]
}

fn kerr(chi2: f64) -> FieldLaw {
    FieldLaw::Polynomial {
        chi1: ScalarField::constant(0.0),
        chi2: ScalarField::constant(chi2),
        amplitude_bound: None,
    }
}

fn saturable(chi: f64, saturation: f64) -> FieldLaw {
    FieldLaw::Saturable {
        chi: ScalarField::constant(chi),
        saturation: ScalarField::constant(saturation),
    }
}

/// A field-dependent medium on either row.
fn nonlinear(mass: Option<FieldLaw>, stiffness: Option<FieldLaw>) -> Scene {
    let mut scene = Scene::default();
    if let Some(law) = mass {
        scene.materials[0].mass_law.field = law;
    }
    if let Some(law) = stiffness {
        scene.materials[0].stiffness_law.field = law;
    }
    scene
}

/// A restoring law on the default medium.
fn restoring(law: RestoringLaw) -> Scene {
    let mut scene = Scene::default();
    scene.materials[0].restoring = law;
    scene
}

fn inert_scene() -> Scene {
    Scene::default()
}

/// One driven row, optionally carrying a spatial pattern at `wavenumber`.
///
/// The depth and frequency of a row are the same either way, so the pumped and
/// travelling members of a pair differ in nothing but the pattern. That is
/// what makes them a crossed design: the pattern is isolated within a row, and
/// the row is isolated at a fixed pattern. Two wavenumbers on the same row
/// then say whether what the estimator charges for scales with the pattern's
/// own gradient.
fn driven(mass: bool, wavenumber: Option<f64>) -> Scene {
    let mut scene = Scene::default();
    let (depth, frequency_hz, phase_radians) = if mass {
        (0.22, 0.9, 0.15)
    } else {
        (0.18, 0.7, -0.2)
    };
    let drive = match wavenumber {
        Some(wavenumber) => TimeDrive::TravellingModulation {
            depth: ScalarField::constant(depth),
            frequency_hz: ScalarField::constant(frequency_hz),
            phase_radians: ScalarField::constant(phase_radians),
            wavenumber: ScalarField::constant(wavenumber),
            angle_radians: ScalarField::constant(0.3),
        },
        None => TimeDrive::ParametricPump {
            depth: ScalarField::constant(depth),
            frequency_hz: ScalarField::constant(frequency_hz),
            phase_radians: ScalarField::constant(phase_radians),
        },
    };
    let material = &mut scene.materials[0];
    if mass {
        material.mass_law.drive = drive;
    } else {
        material.stiffness_law.drive = drive;
    }
    scene
}

/// Both rows patterned at once, to check the two effects compose rather than
/// cancel.
/// A material interface, which is what an authored scene has and the smooth
/// box does not.
///
/// The substituted estimate measures the solver's own complementary flux
/// instead of recovering a gradient from the scalar field. Across a material
/// interface that flux is continuous by construction while the scalar gradient
/// is not, so a term built on the flux may see very little of a defect that
/// lives exactly there. Every fixture above is a single medium, so none of
/// them can tell.
///
/// The reference mesh is only twice as fine as the finest tested one and the
/// solution is kinked here rather than smooth, so the measured true error is
/// contaminated downward by the reference's own error. That inflates the
/// efficiency index, which makes a low index conservative: it cannot be an
/// artefact of the reference.
fn interface_scene(driven: bool) -> Scene {
    let mut scene = Scene::initial();
    let interior = RegionId(2);
    scene.obstacles[0].role = LoopRole::MaterialInterface {
        exterior: BACKGROUND_REGION,
        interior,
    };
    scene.materials.push(Material {
        id: MaterialId(2),
        name: "Interface interior".into(),
        mass_density: ScalarField::constant(1.0),
        // A contrast worth resolving: the wave speed steps by 1.6.
        stiffness: ScalarField::constant(2.5),
        damping: ScalarField::constant(0.0),
        axis_ratio: ScalarField::constant(1.0),
        parameters: vec![],
        color: [80, 120, 160],
        ..Material::default_medium()
    });
    scene.regions.push(Region {
        id: interior,
        material: MaterialId(2),
        frame: MaterialFrame::world(),
    });
    if driven {
        scene.materials[0].mass_law.drive = TimeDrive::ParametricPump {
            depth: ScalarField::constant(0.22),
            frequency_hz: ScalarField::constant(0.9),
            phase_radians: ScalarField::constant(0.15),
        };
    }
    scene
}

fn both_scene() -> Scene {
    let mut scene = driven(true, Some(3.0));
    let stiffness = driven(false, Some(3.0));
    scene.materials[0].stiffness_law.drive = stiffness.materials[0].stiffness_law.drive.clone();
    scene
}

fn build(
    scene: &Scene,
    edge: f64,
    walls: OuterBoundaryConditions,
) -> (
    Arc<TriMesh>,
    Arc<QuadraticWaveOperator>,
    CanonicalTemporalWaveOperator,
) {
    let mut fixed = scene.clone();
    for material in &mut fixed.materials {
        material.mass_law = CoefficientLaw::linear();
        material.stiffness_law = CoefficientLaw::linear();
        material.restoring = RestoringLaw::None;
    }
    let mesh = Arc::new(
        mesh_scene(
            &fixed,
            1,
            MeshingOptions {
                target_edge_length: edge,
                // The self-similar rows' finest meshes are nine times the
                // largest of the others.
                max_vertices: 1_000_000,
                max_triangles: 2_000_000,
                max_refinement_steps: 1_000_000,
                ..MeshingOptions::default()
            },
        )
        .expect("calibration mesh"),
    );
    let quadratic = Arc::new(
        QuadraticWaveOperator::assemble_scene_with_boundaries(&mesh, &fixed, walls)
            .expect("calibration scalar operator"),
    );
    let temporal = CanonicalTemporalWaveOperator::compile_scene(&mesh, &quadratic, scene, 1)
        .expect("calibration temporal operator");
    (mesh, quadratic, temporal)
}

/// A reflecting-box mode released from rest, analytic on every mesh so none
/// starts from another's interpolation. `cos(n*pi*(x+1)/2)` has zero normal
/// derivative on both walls, which is what reflecting means here, so the
/// initial state is compatible and the solution stays smooth.
///
/// An oscillator medium is released from rest in `u` instead, with the mode in
/// the integrated field `r` and its compatible companion `b = ηC r`, so the
/// restoring law is loaded from the first step.
fn initial(
    operator: &CanonicalTemporalWaveOperator,
    walls: OuterBoundaryConditions,
    scale: f64,
) -> (Vec<f64>, Vec<Point2>, Vec<f64>) {
    let mode_x = 2.0 * scale;
    let mode_y = 1.0 * scale;
    let base = operator.base();
    let half = std::f64::consts::PI / 2.0;
    if operator.has_restoring() {
        let integrated = base
            .node_points()
            .iter()
            .map(|point| {
                1.5 * (mode_x * half * (point.x + 1.0)).cos()
                    * (mode_y * half * (point.y + 1.0)).cos()
            })
            .collect::<Vec<_>>();
        let complementary = base
            .compatible_flux(&integrated)
            .expect("compatible integrated mode");
        return (
            vec![0.0; base.degrees_of_freedom()],
            complementary,
            integrated,
        );
    }
    let prescribed = matches!(
        walls.get(OuterSide::Left),
        OuterBoundaryCondition::Dirichlet { .. }
    );
    let absorbing =
        |side: &OuterBoundaryCondition| matches!(side, OuterBoundaryCondition::FirstOrderOutgoing);
    let absorbing_end = walls.sides.iter().filter(|side| absorbing(side)).count() == 1
        && absorbing(&walls.get(OuterSide::Right));
    let absorbing = walls.sides.iter().any(absorbing);
    // Against a prescribed side the start has to be the field the side
    // holds, so its flux is taken at the mass in force then; the authored
    // mass would start a pumped medium off its own boundary data.
    let mass = if prescribed {
        operator
            .primary_mass_at(0.0, &operator.initial_runtime())
            .expect("initial mass")
    } else {
        base.primary_mass().to_vec()
    };
    let primary = mass
        .iter()
        .zip(base.node_points())
        .map(|(mass, point)| {
            if prescribed {
                return mass * 0.08 * (2.0 * half * (point.x + 1.0)).cos();
            }
            if absorbing_end {
                // `sin⁴` over [0.2, 1] in x, zero to its left, which meets
                // both the absorbing end and the reflecting sides smoothly.
                let across = if point.x > 0.2 {
                    (std::f64::consts::PI * (point.x - 0.2) / 0.8).sin().powi(4)
                } else {
                    0.0
                };
                return mass * 0.08 * across * (half * (point.y + 1.0)).sin().powi(4);
            }
            if absorbing {
                let bump = (half * (point.x + 1.0)).sin() * (half * (point.y + 1.0)).sin();
                return mass * 0.08 * bump.powi(4);
            }
            mass * 0.08
                * (mode_x * half * (point.x + 1.0)).cos()
                * (mode_y * half * (point.y + 1.0)).cos()
        })
        .collect::<Vec<_>>();
    // From rest: zero complementary flux is the compatible companion of a
    // displacement-only start.
    let complementary = vec![Point2::default(); base.complementary_degrees_of_freedom()];
    (primary, complementary, vec![])
}

fn lattice_points() -> Vec<Point2> {
    let mut points = Vec::new();
    for row in 0..LATTICE {
        for column in 0..LATTICE {
            let x = -0.9 + 1.8 * column as f64 / (LATTICE - 1) as f64;
            let y = -0.9 + 1.8 * row as f64 / (LATTICE - 1) as f64;
            points.push(Point2::new(x, y));
        }
    }
    points
}

fn solve(
    scene: &Scene,
    edge: f64,
    time_step: f64,
    lattice: &[Point2],
    static_path: bool,
    walls: OuterBoundaryConditions,
    scale: f64,
) -> Solved {
    let started = Instant::now();
    let (mesh, quadratic, operator) = build(scene, edge, walls);
    let forcing = CanonicalForcing::from_legacy_boundaries(operator.base(), &quadratic, 0.0)
        .expect("calibration forcing");
    // The lattice sampler reads geometry and coefficients only, and refuses
    // any law it cannot execute, so its copy carries no loss channel either.
    let mut fixed = scene.clone();
    for material in &mut fixed.materials {
        material.mass_law = CoefficientLaw::linear();
        material.stiffness_law = CoefficientLaw::linear();
        material.restoring = RestoringLaw::None;
        material.electric_loss = None;
        material.magnetic_loss = None;
    }
    let (primary, complementary, integrated) = initial(&operator, walls, scale);
    let mut state = CanonicalTemporalWaveState::new(&operator, time_step, primary, complementary)
        .expect("calibration state");
    if !integrated.is_empty() {
        state = state
            .with_integrated_field(&operator, integrated)
            .expect("calibration integrated field");
    }
    let steps = (TARGET_TIME / scale / time_step).round().max(1.0) as u64;
    let mut previous = state.primary_flux().to_vec();
    let mut previous_complementary = state.complementary_flux().to_vec();
    let mut previous_integrated = state.integrated_field().to_vec();
    for _ in 0..steps {
        previous = state.primary_flux().to_vec();
        previous_complementary = state.complementary_flux().to_vec();
        previous_integrated = state.integrated_field().to_vec();
        state
            .step_with_forcing(&operator, &forcing)
            .expect("calibration step");
    }

    // Sample the solution where every mesh can be compared.
    let mut sampled_primary = Vec::with_capacity(lattice.len());
    let mut sampled_complementary = Vec::with_capacity(lattice.len());
    let mut density = Vec::with_capacity(lattice.len());
    for point in lattice {
        let stencil = QuadraticPointStencil::build(&mesh, &quadratic, &fixed, *point)
            .expect("lattice stencil");
        let consumer = CanonicalTemporalPointStencil::from_quadratic(stencil, &operator)
            .expect("lattice consumer");
        let sample = consumer
            .sample(
                &operator,
                state.primary_flux(),
                &previous,
                state.complementary_flux(),
                state.time(),
                time_step,
                state.runtime(),
            )
            .expect("lattice sample");
        sampled_primary.push(sample.primary);
        sampled_complementary.push(sample.complementary);
        density.push(sample.energy_density);
    }

    // The estimator gets the authored scene, laws and all, which is what
    // production holds. The law-stripped copy exists only because the scalar
    // operator and the point stencils cannot carry a law.
    let estimator = estimate(
        &mesh,
        &quadratic,
        scene,
        &operator,
        &state,
        &previous,
        &previous_complementary,
        &previous_integrated,
        static_path,
        &forcing,
        walls,
    );
    let breakdown = estimator.clone();
    Solved {
        edge,
        degrees_of_freedom: operator.base().degrees_of_freedom(),
        primary: sampled_primary,
        complementary: sampled_complementary,
        density,
        estimator: breakdown.as_ref().map(|report| report.global_indicator),
        breakdown,
        steps,
        wall_seconds: started.elapsed().as_secs_f64(),
    }
}

/// One estimate, by whichever of the two estimators the row names.
#[allow(clippy::too_many_arguments)]
fn estimate(
    mesh: &Arc<TriMesh>,
    quadratic: &Arc<QuadraticWaveOperator>,
    authored: &Scene,
    operator: &CanonicalTemporalWaveOperator,
    state: &CanonicalTemporalWaveState,
    previous: &[f64],
    previous_complementary: &[Point2],
    previous_integrated: &[f64],
    static_path: bool,
    forcing: &CanonicalForcing,
    walls: OuterBoundaryConditions,
) -> Option<funfern_core::SolutionIndicatorReport> {
    let count = operator.base().degrees_of_freedom();
    let patterns = operator.resolution_demand_by_material();
    let snapshot = CanonicalIndicatorSnapshot {
        mesh_revision: mesh.mesh_revision,
        primary_flux: state.primary_flux().to_vec(),
        previous_primary_flux: previous.to_vec(),
        complementary_flux: state.complementary_flux().to_vec(),
        previous_complementary_flux: previous_complementary.to_vec(),
        auxiliary: vec![],
        previous_auxiliary: vec![],
        integrated_field: state.integrated_field().to_vec(),
        previous_integrated_field: previous_integrated.to_vec(),
        time: state.time(),
        time_step: state.time_step(),
    };
    // The static path is the production estimator: the fixed supplement and no
    // runtime, so the gradient terms stay on the reconstructed scalar field and
    // the material samples stay authored. On an inert medium that is exactly
    // what the application runs today.
    let supplement = if static_path {
        funfern_core::canonical_indicator_supplement(mesh, operator.base(), forcing, &snapshot)
    } else {
        canonical_temporal_indicator_supplement(
            mesh,
            operator,
            forcing,
            &snapshot,
            state.runtime(),
            0.0,
        )
    };
    let supplement = match supplement {
        Ok(supplement) => supplement,
        Err(error) => {
            eprintln!("supplement refused: {error}");
            return None;
        }
    };
    // Production zeroes acceleration on purpose, because the canonical
    // estimator excludes the scalar strong cell residual, but it does supply
    // a real rate: the energy denominator uses it. It is the rate production
    // computes, the fixed operator's on the static path and the driven
    // generation's own otherwise, so the index measured here is the one the
    // application's estimate has.
    let current = operator
        .primary_field_at(state.primary_flux(), state.time(), state.runtime())
        .ok()?;
    let velocity = if static_path {
        funfern_core::canonical_primary_rate(operator.base(), forcing, &snapshot).ok()?
    } else {
        funfern_core::canonical_temporal_primary_rate(operator, forcing, &snapshot, state.runtime())
            .ok()?
    };
    let scalar_snapshot = QuadraticSolutionSnapshot {
        mesh_revision: mesh.mesh_revision,
        displacement: current,
        velocity,
        acceleration: vec![0.0; count],
        auxiliary: vec![0.0; count],
        volume_acceleration: vec![0.0; count],
        time: state.time(),
        time_step: state.time_step(),
    };
    // The size rule's instantaneous materials evaluate coefficients without a
    // field, so a field-dependent row hands the job its small-signal
    // (law-stripped) medium; the supplement carries the nonlinear maps.
    //
    // A restoring law is not a coefficient, so the size rule's materials do
    // not carry it either; the supplement holds its store and force.
    let nonlinear = operator.has_field_laws();
    let mut sized = authored.clone();
    // The walls the solver ran, which is what production's model carries;
    // the scene's own default is second-order outgoing, which the canonical
    // estimate skips, and would leave every wall's residual out.
    sized.outer_boundaries = walls;
    for material in &mut sized.materials {
        material.restoring = RestoringLaw::None;
    }
    if nonlinear {
        for material in &mut sized.materials {
            material.mass_law.field = FieldLaw::Linear;
            material.stiffness_law.field = FieldLaw::Linear;
        }
    }
    let mut job = SolutionIndicatorJob::new(
        mesh.clone(),
        quadratic.clone(),
        sized,
        scalar_snapshot,
        SolutionIndicatorOptions {
            minimum_edge_length: 0.005,
            maximum_edge_length: 0.3,
            elements_per_wavelength: ELEMENTS_PER_WAVELENGTH,
            // No sources here, so the field's own scale is zero.
            forcing_frequency_hz: 0.0,
            ..Default::default()
        },
    )
    .with_canonical_supplement(supplement)
    .with_coefficient_patterns(patterns);
    if !static_path {
        job = job.with_instantaneous_materials(state.runtime().clone());
    }
    loop {
        if let Some(result) = job.advance(8_192) {
            // A swallowed estimate reads as a missing number rather than a
            // wrong one, which is worse than either; say why.
            match result {
                Ok(result) => return Some(result.report),
                Err(error) => {
                    eprintln!("estimate refused: {error}");
                    return None;
                }
            }
        }
    }
}

/// Relative difference from the reference in a discrete energy norm over the
/// shared lattice.
fn relative_error(solution: &Solved, reference: &Solved) -> f64 {
    let mut difference = 0.0;
    let mut scale = 0.0;
    for index in 0..solution.primary.len() {
        let primary = solution.primary[index] - reference.primary[index];
        let complementary = solution.complementary[index] - reference.complementary[index];
        difference += primary * primary + complementary.dot(complementary);
        scale += reference.primary[index] * reference.primary[index]
            + reference.complementary[index].dot(reference.complementary[index]);
        let _ = reference.density[index];
    }
    if scale <= 0.0 {
        return f64::NAN;
    }
    (difference / scale).sqrt()
}
