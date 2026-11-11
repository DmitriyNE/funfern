//! C8.2's study: the 11 October plan's section 5.2 on the device. `edges`
//! opens each case's scene through the app's own handlers, strikes it with
//! the app's pulses, runs it a fixed number of steps and reads the device's
//! snapshot against the f64 reference stepped on the generation the app
//! accepted, which takes the events the device took. Where either side
//! refuses an event or a step, the case says where and with what; where the
//! device runs, it reads the relative errors, the peak and whether the
//! device's books stayed finite. Where a case asks, its mesh is then
//! refined, and what the handoff did is read.
//!
//! Where `FUNFERN_E2E_EDGES` is not set the cases are the gate's, which the
//! device suite runs: the workgroups' tails and one case for each defect the
//! study found, each held to what it must do. Otherwise they are the study's
//! categories it names, separated by `,` (`empty`, `amplitudes`, `sources`,
//! `contrast`, `geometry`, `cancellation`, `tails`, `probes`, `failure`,
//! `gate`), which print `e2e edges:` lines and hold nothing;
//! `FUNFERN_E2E_EDGES_ONLY` keeps the cases whose label holds it. The fixture
//! fails where a gate case missed or the driver itself broke.

use super::edge_scenes::*;
use super::*;
use crate::canonical_gpu::{CANONICAL_FAILURE_NON_FINITE, CANONICAL_FAILURE_RESTORING_DOMAIN};
use crate::topology_editor::TopologyProbeTarget;
use crate::topology_runtime::{TopologyProbeCompilation, TopologyProbeStencil};
use funfern_core::{
    CanonicalPointStencil, FieldLaw, PeriodicCubicSpline, Point2, QuadraticAreaStencil, TimeSignal,
    sample_canonical_area,
};

/// Seconds a case may take, where the fixture's deadline does not say.
const CASE_SECONDS: u64 = 120;

/// Seconds to wait for a snapshot or a handoff before the case says so.
const WAIT_SECONDS: u64 = 30;

/// The pulse's width every case strikes with: a few elements across at the
/// cases' edges.
const WIDTH: f32 = 0.15;

#[derive(Resource)]
pub(super) struct EdgeDriver {
    cases: VecDeque<Case>,
    phase: EdgePhase,
    /// The device's completed-step count when the case's generation was
    /// installed.
    base: u64,
    mirror: Option<Mirror>,
    per_case: std::time::Duration,
    deadline: Instant,
    ran: usize,
    /// What went wrong with the driver itself, as opposed to what a case
    /// found.
    broken: Vec<String>,
}

struct Case {
    category: &'static str,
    label: String,
    document: TopologyDocument,
    strikes: Vec<Strike>,
    steps: u64,
    /// The mesh edge to refine to once the run has ended, where the case
    /// reads a handoff.
    remesh: Option<f64>,
    /// The step past which a fault is armed on the running generation and
    /// pulses fired until the app has seen the failure.
    fault: Option<u64>,
    /// What the case must do, for the gate's.
    expect: Expect,
    /// The step past which the generation is latched failed directly and a
    /// pulse sent at once, before the host has read the latch.
    latch: Option<u64>,
}

/// What a gate case must do.
#[derive(Clone, Copy, Debug)]
enum Expect {
    /// A study's: what it found is printed, and nothing is held.
    Studied,
    /// Every strike taken and every step run, the field and the books
    /// finite, and the field within `.0` of the reference's scale.
    Matches(f64),
    /// The device fails a step with the code, and the reference refuses the
    /// same step.
    BothRefuse(u32),
    /// The device fails a step with the code where the reference goes on;
    /// its accepted field within `.1` of the reference's, its books finite.
    DeviceFails(u32, f64),
    /// Every step run, finite, and the peak within `.0` times the
    /// reference's.
    Bounded(f64),
    /// The pulse sent behind the latch refused with the latch's code, and
    /// the field within `.0` of the reference's, which never took it.
    LatchedPulseRefused(f64),
}

/// What a case did, as the gate reads it.
#[derive(Default)]
struct Outcome {
    /// Each strike's refusal code, 0 where the device took it.
    strikes: Vec<u32>,
    completed: u64,
    failure: u32,
    /// Whether the reference took the step the device failed.
    reference_went_on: bool,
    /// Whether the reference refused a step the device took.
    reference_stopped: bool,
    /// The largest of the relative errors in `Q`, `b` and `r`.
    error: Option<f64>,
    peak: f64,
    reference_peak: f64,
    finite: bool,
    books_finite: bool,
    /// The refusal code the pulse behind the latch was answered with.
    latched_pulse: Option<u32>,
}

#[derive(Clone, Copy)]
struct Strike {
    at: Point2,
    amplitude: f32,
}

/// The reference beside the device, and what the case found so far.
struct Mirror {
    active: Arc<PreparedTopology>,
    state: State,
    /// Whether the reference took exactly the events the device took, so
    /// that the two can be read against each other.
    comparable: bool,
    found: Vec<String>,
    outcome: Outcome,
}

enum EdgePhase {
    Open,
    Install,
    /// A strike was queued and the device is to process it.
    Strike {
        index: usize,
        processed: u32,
        queued: (Point2, RegionId),
    },
    Run,
    Snapshot {
        seen: u64,
        completed: u64,
        failure: u32,
        asked: Instant,
    },
    Remesh {
        generation: u64,
        asked: Instant,
    },
    /// The generation was latched at step `at` and a pulse placed, which
    /// the device is to answer.
    Latched {
        at: u64,
        processed: u32,
        queued: bool,
        since: Instant,
    },
    /// A fault was armed at step `armed`, and pulses go in until the app has
    /// seen the device fail.
    Faulted {
        armed: u64,
        last: u32,
        sent: usize,
        events: Vec<String>,
        since: Instant,
        first: bool,
    },
    Done,
}

impl EdgePhase {
    fn name(&self) -> &'static str {
        match self {
            EdgePhase::Open => "opening the scene",
            EdgePhase::Install => "installing its generation",
            EdgePhase::Strike { .. } => "waiting for a pulse to land",
            EdgePhase::Run => "running",
            EdgePhase::Snapshot { .. } => "waiting for the snapshot",
            EdgePhase::Remesh { .. } => "waiting for the handoff",
            EdgePhase::Latched { .. } => "waiting for the pulse behind the latch",
            EdgePhase::Faulted { .. } => "firing pulses into a failed run",
            EdgePhase::Done => "done",
        }
    }
}

/// Adds the edge cases' driver.
pub(super) fn add(app: &mut App, seconds: Option<u64>) {
    let cases = cases();
    if cases.is_empty() {
        let failure = "no case matches FUNFERN_E2E_EDGES and FUNFERN_E2E_EDGES_ONLY";
        report(&format!("e2e edges: {failure}"));
        mark("fail", failure);
        #[cfg(not(target_arch = "wasm32"))]
        std::process::exit(2);
        #[cfg(target_arch = "wasm32")]
        return;
    }
    mark("running", "");
    report(&format!(
        "e2e build {}; edges: {} cases",
        crate::BUILD_ID,
        cases.len()
    ));
    let per_case = std::time::Duration::from_secs(seconds.unwrap_or(CASE_SECONDS));
    app.insert_resource(EdgeDriver {
        cases: cases.into(),
        phase: EdgePhase::Open,
        base: 0,
        mirror: None,
        per_case,
        deadline: Instant::now() + per_case,
        ran: 0,
        broken: Vec::new(),
    })
    .add_systems(EguiPrimaryContextPass, drive_edges.before(frame));
}

/// The box as the cases open it: no grid filter, which the reference would
/// have to follow, no adaptation, and meshed at `edge`.
fn plain(mut document: TopologyDocument, edge: f64) -> TopologyDocument {
    document.presentation.mesh_edge = edge;
    document.presentation.adaptation.enabled = false;
    document.presentation.grid_scale_filter = false;
    document
}

/// `document` with its point source on, driven at `amplitude` and 1 Hz.
fn sourced(mut document: TopologyDocument, amplitude: f64) -> TopologyDocument {
    document.model.source.enabled = true;
    document.model.source.signal = TimeSignal::harmonic(0.0, amplitude, 1.0, 0.0);
    document
}

fn strike(x: f64, y: f64, amplitude: f32) -> Strike {
    Strike {
        at: Point2::new(x, y),
        amplitude,
    }
}

/// Whether a law is a field law, whose generations the device takes no
/// pulse on.
fn field_law(law: &Law) -> bool {
    let mut material = TopologyEditor::default().document.model.draft.materials[0].clone();
    (law.author)(&mut material);
    !matches!(material.mass_law.field, FieldLaw::Linear)
        || !matches!(material.stiffness_law.field, FieldLaw::Linear)
}

fn cases() -> Vec<Case> {
    #[cfg(not(target_arch = "wasm32"))]
    let (asked, only) = (
        std::env::var("FUNFERN_E2E_EDGES").ok(),
        std::env::var("FUNFERN_E2E_EDGES_ONLY").ok(),
    );
    #[cfg(target_arch = "wasm32")]
    let (asked, only): (Option<String>, Option<String>) = (None, None);
    let wanted = |category: &str| {
        asked.as_deref().map_or(category == "gate", |asked| {
            asked.split(',').any(|name| name.trim() == category)
        })
    };
    let mut cases = Vec::new();
    if wanted("gate") {
        cases.extend(gate_cases());
    }
    if wanted("tails") {
        cases.extend(tail_cases());
    }
    if wanted("empty") {
        cases.extend(empty_cases());
    }
    if wanted("amplitudes") {
        cases.extend(amplitude_cases());
    }
    if wanted("sources") {
        cases.extend(source_cases());
    }
    if wanted("contrast") {
        cases.extend(contrast_cases());
    }
    if wanted("geometry") {
        cases.extend(geometry_cases());
    }
    if wanted("cancellation") {
        cases.extend(cancellation_cases());
    }
    if wanted("probes") {
        cases.extend(probe_cases());
    }
    if wanted("failure") {
        cases.extend(failure_cases());
    }
    cases.retain(|case| only.as_deref().is_none_or(|only| case.label.contains(only)));
    cases
}

/// Counts around the workgroups' 128: the box at sizes and edges whose node
/// counts, found on the CPU, fall at 127, 128 and 129 and at 255, 256 and
/// 257, and outgoing traces of 127, 128 and 129 nodes, through the direct
/// solve (a fixed medium), the swept one (a pumped medium) and the
/// nonlinear one (focusing Kerr).
fn tail_cases() -> Vec<Case> {
    let mut cases = Vec::new();
    for &(nodes, width, edge, holed) in TAIL_MESHES {
        let cut = if holed {
            vec![small_square()]
        } else {
            Vec::new()
        };
        let Ok(mut document) = holes(cut) else {
            continue;
        };
        sized(&mut document, width, 1.0);
        cases.push(Case {
            category: "tails",
            label: format!("{nodes} nodes"),
            document: plain(document, edge),
            strikes: vec![strike(0.3 * width, 0.2, 1.0)],
            steps: 200,
            remesh: None,
            fault: None,
            expect: Expect::Studied,
            latch: None,
        });
    }
    for &(trace, width, edge, sides) in TAIL_TRACES {
        for kind in ["fixed", "pumped", "Kerr"] {
            let mut document = document(OuterBoundaryCondition::Reflecting, |editor| match kind {
                "pumped" => medium(editor, |m| {
                    m.mass_law.drive = funfern_core::TimeDrive::ParametricPump {
                        depth: constant(0.2),
                        frequency_hz: constant(1.0),
                        phase_radians: constant(0.0),
                    }
                }),
                "Kerr" => medium(editor, |m| m.stiffness_law.field = kerr(0.5, None)),
                _ => Ok(()),
            });
            sized(&mut document, width, 1.0);
            for side in sides {
                for model in [&mut document.model.draft, &mut document.model.accepted] {
                    model.outer_boundaries.sides[side.index()] =
                        OuterBoundaryCondition::SecondOrderOutgoing;
                }
            }
            let document = plain(document, edge);
            // A field law takes no pulse: its source drives it.
            let (document, strikes) = if kind == "Kerr" {
                let mut document = sourced(document, 1.0);
                // The default source stands at x = -0.45, past the narrow
                // boxes' sides.
                document.model.source.position = Point2::new(0.0, 0.3);
                (document, Vec::new())
            } else {
                (document, vec![strike(0.3 * width, 0.2, 1.0)])
            };
            cases.push(Case {
                category: "tails",
                label: format!("a trace of {trace} nodes, {kind}"),
                document,
                strikes,
                steps: 200,
                remesh: None,
                fault: None,
                expect: Expect::Studied,
                latch: None,
            });
        }
    }
    cases
}

/// The gate's: the tails, which must match, and a case for each defect the
/// study found (C8.2, 2026-11-11), each held to what the fix makes it do.
fn gate_cases() -> Vec<Case> {
    let mut cases = tail_cases();
    for case in &mut cases {
        case.category = "gate";
        case.expect = Expect::Matches(GATE_TOLERANCE);
    }
    let law = |name: &str| {
        laws()
            .into_iter()
            .find(|law| law.name == name)
            .expect("a law the sweep knows")
    };
    let opened = |name: &str| {
        let law = law(name);
        plain(
            document(OuterBoundaryCondition::Reflecting, |editor| {
                medium(editor, |m| (law.author)(m))
            }),
            0.2,
        )
    };
    let case = |label: String, document, strikes, steps, expect| Case {
        category: "gate",
        label,
        document,
        strikes,
        steps,
        remesh: None,
        fault: None,
        expect,
        latch: None,
    };
    // Its field at 5.7e7 put `r` past the phase f32 holds in one step, where
    // the device's sine came back as large as 81 and the field grew to
    // 2.3e18 in six steps before the device failed; both sides refuse that
    // step now.
    cases.push(case(
        "sine-Gordon, a pulse of 1e10".into(),
        opened("sine-Gordon"),
        vec![strike(0.3, 0.2, 1.0e10)],
        200,
        Expect::BothRefuse(CANONICAL_FAILURE_RESTORING_DOMAIN),
    ));
    // A pulse's carrier counts from the pulse's centre: at 1 MHz, 1,000 s
    // before it, its argument is 6.3e9 from the first step, where the
    // device's sine, reduced once, came back as large as 4e18 and the field
    // reached 2.4e11. A carrier run from zero gets there only late in an
    // epoch, 19,200 steps on. No f32 argument there holds a phase, so the
    // device's carrier is noise of its amplitude: the field reaches 7.3 times
    // the reference's on lavapipe, and is held to a hundred (C8.3's).
    let mut carrier = opened("linear");
    carrier.model.source.enabled = true;
    carrier.model.source.signal = TimeSignal::pulsed(
        [0.0, 1.0, 1.0e6, 0.0],
        funfern_core::PulseEnvelope::FlatTop {
            duration: 2_000.0,
            edge: 0.5,
        },
        0.0,
        0.0,
    );
    cases.push(case(
        "linear, a 1 MHz pulse 1,000 s before its centre".into(),
        carrier,
        Vec::new(),
        200,
        Expect::Bounded(100.0),
    ));
    // Damped from 5.7e7 in a stage, the van der Pol multiplier kept only its
    // f32 unit of one and the field ended 118% off.
    for amplitude in [1.0e10, 1.0e18] {
        cases.push(case(
            format!("van der Pol, a pulse of {amplitude:e}"),
            opened("van der Pol"),
            vec![strike(0.3, 0.2, amplitude)],
            200,
            Expect::Matches(VAN_DER_POL_TOLERANCE),
        ));
    }
    // Driven at 1e10 the Kerr inverse ran out of its 40 steps at step 140.
    cases.push(case(
        "focusing Kerr on the stiffness, driven at 1e10".into(),
        sourced(opened("focusing Kerr on the stiffness"), 1.0e10),
        Vec::new(),
        400,
        Expect::Matches(GATE_TOLERANCE),
    ));
    // Driven at 1e30 the books went NaN with the field finite, and every step
    // after committed; the step that overflows them now fails.
    cases.push(case(
        "linear, driven at 1e30".into(),
        sourced(opened("linear"), 1.0e30),
        Vec::new(),
        400,
        Expect::DeviceFails(CANONICAL_FAILURE_NON_FINITE, GATE_TOLERANCE),
    ));
    // A pulse after a latched failure the host has not read committed the
    // failed step's lane.
    cases.push(Case {
        latch: Some(20),
        ..case(
            "a pulse behind a latched failure".into(),
            opened("linear"),
            vec![strike(0.3, 0.2, 1.0)],
            100_000,
            Expect::LatchedPulseRefused(GATE_TOLERANCE),
        )
    });
    cases
}

/// The gate's tolerance on the field against the reference, of its scale:
/// the tails read 1.6e-6 at worst on lavapipe.
const GATE_TOLERANCE: f64 = 1.0e-5;

/// The van der Pol cases' tolerance, damped from 5.7e7 and 5.7e15.
const VAN_DER_POL_TOLERANCE: f64 = 1.0e-5;

/// Meshes whose node counts fall about the workgroups' 128 and twice it:
/// the node count, the box's width (its height 2), the edge, and whether
/// [`small_square`] is cut from it, a mesh without a hole holding an odd
/// count. Found on the CPU by `study_tail_meshes`, written as it formed them.
const TAIL_MESHES: &[(usize, f64, f64, bool)] = &[
    (127, 0.025 * 13.0, 0.12 + 0.005 * 44.0, false),
    (128, 0.025 * 5.0, 0.12 + 0.005 * 26.0, true),
    (129, 0.025 * 37.0, 0.12 + 0.005 * 69.0, false),
    (255, 0.025 * 9.0, 0.12 + 0.005 * 2.0, false),
    (256, 0.025 * 6.0, 0.12 + 0.005 * 2.0, true),
    (257, 0.025 * 7.0, 0.12, false),
];

/// Outgoing traces about 128 nodes: the trace's node count, the box's width
/// (its height 2), the edge and the sides made second-order outgoing. A
/// side's trace holds an odd count; all four sides' an even one. Found on
/// the CPU by `study_tail_meshes`.
const TAIL_TRACES: &[(usize, f64, f64, &[OuterSide])] = &[
    (127, 0.3, 0.026 + 0.00005 * 254.0, &[OuterSide::Right]),
    (128, 2.0, 0.05 + 0.0005 * 219.0, &OuterSide::ALL),
    (129, 0.3, 0.026 + 0.00005 * 210.0, &[OuterSide::Right]),
];

/// C8.1's 216 combinations of sources, probes, walls, histories and drives,
/// each struck once and run 40 steps.
fn empty_cases() -> Vec<Case> {
    use OuterBoundaryCondition::*;
    let mut cases = Vec::new();
    for source in ["none", "point", "volume"] {
        for probe in ["none", "point", "segment", "disc"] {
            for walls in [Reflecting, FirstOrderOutgoing, SecondOrderOutgoing] {
                for history in ["none", "loss", "restoring"] {
                    for driven in [false, true] {
                        cases.push(Case {
                            category: "empty",
                            label: format!(
                                "source {source}, probe {probe}, walls {walls:?}, history \
                                 {history}, {}",
                                if driven { "driven" } else { "fixed" }
                            ),
                            document: plain(empty_case(source, probe, walls, history, driven), 0.2),
                            strikes: vec![strike(0.3, 0.2, 1.0)],
                            steps: 40,
                            remesh: None,
                            fault: None,
                            expect: Expect::Studied,
                            latch: None,
                        });
                    }
                }
            }
        }
    }
    cases
}

/// The pulse's amplitudes the sweep strikes with: zero, f32's smallest
/// subnormal and one above it, its smallest normal, small and ordinary
/// values, and the energy's overflow about 1e19.
const AMPLITUDES: [f32; 12] = [
    0.0,
    1.0e-45,
    1.0e-39,
    f32::MIN_POSITIVE,
    1.0e-30,
    1.0e-6,
    1.0,
    1.0e10,
    1.0e18,
    1.0e19,
    1.0e20,
    1.0e30,
];

/// Each law the device takes pulses on, struck at [`AMPLITUDES`] and at
/// shares of its bound, run 200 steps; the linear medium at 1e18 and 1e19
/// is then refined, its books near their overflow.
fn amplitude_cases() -> Vec<Case> {
    let mut cases = Vec::new();
    for law in laws().into_iter().filter(|law| !field_law(law)) {
        let mut amplitudes: Vec<f32> = AMPLITUDES.to_vec();
        if let Some(bound) = law.bound {
            amplitudes.extend(
                [0.5, 0.9, 0.99, 1.01, 1.5]
                    .into_iter()
                    .map(|share| (share * bound) as f32),
            );
        }
        for amplitude in amplitudes {
            let document = plain(
                document(OuterBoundaryCondition::Reflecting, |editor| {
                    medium(editor, |m| (law.author)(m))
                }),
                0.2,
            );
            let remesh = (law.name == "linear" && (amplitude == 1.0e18 || amplitude == 1.0e19))
                .then_some(0.15);
            cases.push(Case {
                category: "amplitudes",
                label: format!("{}, a pulse of {amplitude:e}", law.name),
                document,
                strikes: vec![strike(0.3, 0.2, amplitude)],
                steps: 200,
                remesh,
                fault: None,
                expect: Expect::Studied,
                latch: None,
            });
        }
    }
    cases
}

/// The source's amplitudes the field laws are driven at, where the device
/// takes no pulse, and the linear medium beside them.
const SOURCE_AMPLITUDES: [f64; 10] = [
    1.0e-39, 1.0e-30, 1.0e-6, 1.0, 1.0e2, 1.0e4, 1.0e6, 1.0e10, 1.0e19, 1.0e30,
];

/// Each field law, and the linear medium, driven by the point source at
/// [`SOURCE_AMPLITUDES`] for 400 steps; the linear medium at 1e19 and 1e30
/// then refined.
fn source_cases() -> Vec<Case> {
    let mut cases = Vec::new();
    for law in laws()
        .into_iter()
        .filter(|law| law.name == "linear" || field_law(law))
    {
        for amplitude in SOURCE_AMPLITUDES {
            let document = plain(
                document(OuterBoundaryCondition::Reflecting, |editor| {
                    medium(editor, |m| (law.author)(m))
                }),
                0.2,
            );
            let remesh = (law.name == "linear" && amplitude >= 1.0e19).then_some(0.15);
            cases.push(Case {
                category: "sources",
                label: format!("{}, driven at {amplitude:e}", law.name),
                document: sourced(document, amplitude),
                strikes: Vec::new(),
                steps: 400,
                remesh,
                fault: None,
                expect: Expect::Studied,
                latch: None,
            });
        }
    }
    cases
}

/// C8.1's inclusions, in impedance, in stiffness and in compliance, and the
/// box at axis ratios 4 to 1e4, at edge 0.1.
fn contrast_cases() -> Vec<Case> {
    let mut cases = Vec::new();
    for contrast in [25.0, 1.0e2, 1.0e4, 1.0e6] {
        for (kind, density, stiffness) in [
            ("impedance", contrast, contrast),
            ("stiffness", 1.0, contrast),
            ("compliance", 1.0, 1.0 / contrast),
        ] {
            cases.push(Case {
                category: "contrast",
                label: format!("{kind} {contrast:e}"),
                document: plain(inclusion(density, stiffness), 0.1),
                strikes: vec![strike(0.65, 0.3, 1.0)],
                steps: 400,
                remesh: None,
                fault: None,
                expect: Expect::Studied,
                latch: None,
            });
        }
    }
    for ratio in [4.0, 16.0, 64.0, 1.0e3, 1.0e4] {
        cases.push(Case {
            category: "contrast",
            label: format!("axis ratio {ratio}"),
            document: plain(anisotropic(ratio), 0.1),
            strikes: vec![strike(0.3, 0.2, 1.0)],
            steps: 400,
            remesh: None,
            fault: None,
            expect: Expect::Studied,
            latch: None,
        });
    }
    cases
}

/// Discs down to 1e-6 off a wall and off each other, a channel 3e-4 wide,
/// a polygon's edge 1e-3 long and a disc's controls 1e-6 apart, at edge 0.1.
fn geometry_cases() -> Vec<Case> {
    let mut cut: Vec<(String, Vec<PeriodicCubicSpline>)> = Vec::new();
    for gap in [1.0e-2, 1.0e-4, 1.0e-6] {
        cut.push((
            format!("a disc {gap:e} off the wall"),
            vec![disc_off(1.0, 1.0, gap, 0.3)],
        ));
        cut.push((
            format!("two discs {gap:e} apart"),
            vec![
                disc_off(-0.5 * gap, 1.0, 0.0, 0.3),
                disc_off(0.5 * gap, -1.0, 0.0, 0.3),
            ],
        ));
    }
    cut.push(("a channel 3e-4 wide".into(), vec![channel(3.0e-4)]));
    cut.push((
        "a polygon with an edge 1e-3 long".into(),
        vec![short_edge(1.0e-3)],
    ));
    cut.push((
        "a disc with two controls 1e-6 apart".into(),
        vec![pinched(1.0e-6)],
    ));
    cut.into_iter()
        .filter_map(|(label, splines)| {
            let document = holes(splines)
                .inspect_err(|error| report(&format!("e2e edges: {label}: refused, {error}")))
                .ok()?;
            Some(Case {
                category: "geometry",
                label,
                document: plain(document, 0.1),
                strikes: vec![strike(-0.6, 0.6, 1.0)],
                steps: 400,
                remesh: None,
                fault: None,
                expect: Expect::Studied,
                latch: None,
            })
        })
        .collect()
}

/// Pulses of opposite sign: at two points mirrored through the middle, at
/// one point to nothing, and at one point to a millionth of one; each then
/// refined, the handoff carrying each component's total.
fn cancellation_cases() -> Vec<Case> {
    let box_ = || {
        plain(
            document(OuterBoundaryCondition::Reflecting, |_| Ok(())),
            0.2,
        )
    };
    vec![
        Case {
            category: "cancellation",
            label: "mirrored pulses of opposite sign".into(),
            document: box_(),
            strikes: vec![strike(0.4, 0.2, 1.0), strike(-0.4, -0.2, -1.0)],
            steps: 200,
            remesh: Some(0.15),
            fault: None,
            expect: Expect::Studied,
            latch: None,
        },
        Case {
            category: "cancellation",
            label: "a pulse taken back".into(),
            document: box_(),
            strikes: vec![strike(0.4, 0.2, 1.0), strike(0.4, 0.2, -1.0)],
            steps: 200,
            remesh: Some(0.15),
            fault: None,
            expect: Expect::Studied,
            latch: None,
        },
        Case {
            category: "cancellation",
            label: "a pulse taken back to a millionth".into(),
            document: box_(),
            strikes: vec![strike(0.4, 0.2, 1.0), strike(0.4, 0.2, -0.999_999)],
            steps: 200,
            remesh: Some(0.15),
            fault: None,
            expect: Expect::Studied,
            latch: None,
        },
    ]
}

/// Probe counts about the recorders' workgroups: 15 and 16 point probes and
/// discs, the point kernel's 16 and all a document holds, and line probes of
/// 32, 480 and 512 points in all, about the line kernel's 64 and at the
/// document's cap.
fn probe_cases() -> Vec<Case> {
    let grid = |count: usize| {
        (0..count)
            .map(|k| Point2::new(-0.75 + 0.5 * (k % 4) as f64, -0.75 + 0.5 * (k / 4) as f64))
            .collect::<Vec<_>>()
    };
    let mut cases = Vec::new();
    for count in [15, 16] {
        for kind in ["point", "disc"] {
            let document = document(OuterBoundaryCondition::Reflecting, |editor| {
                for (k, at) in grid(count).into_iter().enumerate() {
                    let target = match kind {
                        "point" => TopologyProbeTarget::Point(at),
                        _ => TopologyProbeTarget::AreaDisk {
                            center: at,
                            radius: 0.15,
                        },
                    };
                    editor.create_probe(format!("probe {k}"), [200, 80, 80], target)?;
                }
                Ok(())
            });
            cases.push(Case {
                category: "probes",
                label: format!("{count} {kind} probes"),
                document: plain(document, 0.2),
                strikes: vec![strike(0.3, 0.2, 1.0)],
                steps: 200,
                remesh: None,
                fault: None,
                expect: Expect::Studied,
                latch: None,
            });
        }
    }
    use ProbeSamplingPreset::{High, Low};
    for (points, presets) in [
        (32, vec![Low]),
        (480, vec![High, High, High, Low, Low, Low]),
        (512, vec![High; 4]),
    ] {
        let document = document(OuterBoundaryCondition::Reflecting, |editor| {
            for (k, preset) in presets.iter().enumerate() {
                let y = -0.8 + 0.3 * k as f64;
                editor.create_probe(
                    format!("line {k}"),
                    [80, 80, 200],
                    TopologyProbeTarget::Segment {
                        start: Point2::new(-0.9, y),
                        end: Point2::new(0.9, y + 0.05),
                        preset: *preset,
                    },
                )?;
            }
            Ok(())
        });
        cases.push(Case {
            category: "probes",
            label: format!("line probes of {points} points"),
            document: plain(document, 0.2),
            strikes: vec![strike(0.3, 0.2, 1.0)],
            steps: 200,
            remesh: None,
            fault: None,
            expect: Expect::Studied,
            latch: None,
        });
    }
    cases
}

/// Pulses fired into a run whose device has just failed a step: a fault is
/// armed on the running generation, and from the next frame a pulse is
/// placed whenever the app will take one, until the app has seen the
/// failure. A pulse the device takes after it refused one for the failed
/// step is one only a commit that ignores the latch would take.
fn failure_cases() -> Vec<Case> {
    (0..12)
        .map(|trial| Case {
            category: "failure",
            label: format!("pulses into a failed run, trial {trial}"),
            document: plain(
                document(OuterBoundaryCondition::Reflecting, |_| Ok(())),
                0.2,
            ),
            strikes: vec![strike(0.3, 0.2, 1.0)],
            steps: 100_000,
            remesh: None,
            fault: Some(40 + 7 * trial),
            expect: Expect::Studied,
            latch: None,
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn drive_edges(
    mut driver: ResMut<EdgeDriver>,
    mut state: ResMut<Playground>,
    mut request: ResMut<CanonicalGpuRequest>,
    display: Res<CanonicalGpuDisplay>,
    mut assets: ResMut<Assets<ShaderBuffer>>,
    mut commands: Commands,
    mut exit: MessageWriter<AppExit>,
) {
    if matches!(driver.phase, EdgePhase::Done) {
        return;
    }
    let finished = match advance(
        &mut driver,
        &mut state,
        &mut request,
        &display,
        &mut assets,
        &mut commands,
    ) {
        Some(finished) => finished,
        None if Instant::now() >= driver.deadline => {
            Err(format!("did not finish in time, {}", driver.phase.name()))
        }
        None => return,
    };
    let Some(case) = driver.cases.pop_front() else {
        return;
    };
    let (found, outcome) = driver.mirror.take().map_or((Vec::new(), None), |mirror| {
        (mirror.found, Some(mirror.outcome))
    });
    let mut line = format!("e2e edges: {} | {}", case.category, case.label);
    for part in &found {
        line.push_str(" | ");
        line.push_str(part);
    }
    if let Err(failure) = &finished {
        line.push_str(&format!(" | STOPPED: {failure}"));
        if failure.starts_with("the driver") {
            driver.broken.push(format!("{}: {failure}", case.label));
        }
    }
    if !matches!(case.expect, Expect::Studied) {
        let verdict = match &finished {
            Err(failure) => Err(format!("stopped: {failure}")),
            Ok(()) => judge(case.expect, outcome.as_ref(), case.strikes.len()),
        };
        match verdict {
            Ok(()) => line.push_str(&format!(" | HELD {:?}", case.expect)),
            Err(miss) => {
                line.push_str(&format!(" | MISSED {:?}: {miss}", case.expect));
                driver.broken.push(format!("{}: {miss}", case.label));
            }
        }
    }
    report(&line);
    driver.ran += 1;
    state.e2e_steps.limit = Some(0);
    driver.deadline = Instant::now() + driver.per_case;
    if !driver.cases.is_empty() {
        driver.phase = EdgePhase::Open;
        return;
    }
    driver.phase = EdgePhase::Done;
    let summary = format!("{} cases run", driver.ran);
    if driver.broken.is_empty() {
        report(&format!("e2e edges: {summary}"));
        mark("pass", &summary);
        exit.write(AppExit::Success);
    } else {
        let failure = format!("{summary}; broke on {}", driver.broken.join("; "));
        report(&format!("e2e edges FAILED: {failure}"));
        mark("fail", &failure);
        exit.write(AppExit::error());
    }
}

/// One frame of the current case; its end, once it has one.
fn advance(
    driver: &mut EdgeDriver,
    state: &mut Playground,
    request: &mut CanonicalGpuRequest,
    display: &CanonicalGpuDisplay,
    assets: &mut Assets<ShaderBuffer>,
    commands: &mut Commands,
) -> Option<Result<(), String>> {
    match std::mem::replace(&mut driver.phase, EdgePhase::Done) {
        EdgePhase::Open => {
            if !state.startup_done {
                driver.phase = EdgePhase::Open;
                return None;
            }
            let document = driver.cases.front()?.document.clone();
            // A case's time runs from its opening, the app's start up aside.
            driver.deadline = Instant::now() + driver.per_case;
            *FRESH_CLOCK.lock().unwrap() = None;
            state.e2e_steps.limit = Some(0);
            state.e2e_steps.batch = None;
            state.e2e_steps.burst = false;
            state.wave_running = true;
            state.amr_error = None;
            if let Err(error) = state.set_document(document, false, true) {
                return Some(Err(format!("the scene did not open: {error}")));
            }
            driver.phase = EdgePhase::Install;
            None
        }
        EdgePhase::Install => {
            let installed = state.runtime.active().is_some_and(|active| {
                active.bundle.token.document_revision == state.editor.revision
                    && *active.bundle.authored == state.editor.document.model.accepted
                    && state.coordinator.uploading.is_none()
                    && request.generation() != 0
                    && display.generation == request.generation()
            });
            if !installed {
                if let Some(error) = &state.amr_error {
                    return Some(Err(format!("the scene was not installed: {error}")));
                }
                // A law the operator refuses fails the preparation, not the
                // device: the scene never installs.
                if let Some(error) = state.runtime.last_error()
                    && error.token.document_revision == state.editor.revision
                {
                    return Some(Err(format!("the scene was refused: {}", error.message)));
                }
                driver.phase = EdgePhase::Install;
                return None;
            }
            let active = state.runtime.active()?.clone();
            let time_step = state.coordinator.uploaded_time_step;
            let reference = fresh_clock(time_step)
                .and_then(|clock| State::zero(&active, time_step)?.starting_at(clock.time()));
            let reference = match reference {
                Ok(reference) => reference,
                Err(error) => {
                    return Some(Err(format!(
                        "the driver's reference did not start: {error}"
                    )));
                }
            };
            let operator = &active.canonical_operator;
            let trace = operator
                .outgoing_boundary()
                .map_or(0, |boundary| boundary.trace_nodes().len());
            driver.mirror = Some(Mirror {
                found: vec![format!(
                    "{} nodes, {} samples, trace {trace}, step {time_step:.3e}",
                    operator.degrees_of_freedom(),
                    operator.complementary_degrees_of_freedom(),
                )],
                active,
                state: reference,
                comparable: true,
                outcome: Outcome::default(),
            });
            driver.base = request.stats().completed_steps();
            state.e2e_steps.limit = Some(driver.base);
            state.wave_running = true;
            begin_strike(driver, state, request, 0)
        }
        EdgePhase::Strike {
            index,
            processed,
            queued,
        } => {
            let unmoved = request.stats().processed_event() == processed;
            let waiting =
                !state.coordinator.pending_pulses.is_empty() || request.live_event_pending();
            if unmoved && !waiting {
                let mirror = driver.mirror.as_mut()?;
                mirror
                    .found
                    .push(format!("the app dropped strike {index}: {}", state.message));
                return begin_strike(driver, state, request, index + 1);
            }
            if unmoved || waiting {
                driver.phase = EdgePhase::Strike {
                    index,
                    processed,
                    queued,
                };
                return None;
            }
            let rejection = request.stats().event_rejection();
            let strike = driver.cases.front()?.strikes[index];
            let mirror = driver.mirror.as_mut()?;
            mirror.outcome.strikes.push(rejection);
            let increment = match pulse_increment(
                &mirror.active,
                queued.0,
                queued.1,
                f64::from(strike.amplitude),
                f64::from(WIDTH),
            ) {
                Ok(increment) => increment,
                Err(error) => {
                    return Some(Err(format!("the driver's pulse did not form: {error}")));
                }
            };
            let mut trial = mirror.state.clone();
            let cpu = trial.pulse(&mirror.active, &increment);
            let said = match (rejection, &cpu) {
                (0, Ok(())) => {
                    mirror.state = trial;
                    format!("strike {index} taken")
                }
                (0, Err(error)) => {
                    mirror.comparable = false;
                    format!("strike {index} taken, which the reference REFUSED ({error})")
                }
                (code, Ok(())) => format!(
                    "strike {index} REFUSED by the device ({code}: {}), the reference took it",
                    canonical_failure_description(code)
                ),
                (code, Err(error)) => format!(
                    "strike {index} refused by both ({code}: {}; {error})",
                    canonical_failure_description(code)
                ),
            };
            mirror.found.push(said);
            begin_strike(driver, state, request, index + 1)
        }
        EdgePhase::Run => {
            let completed = request
                .stats()
                .completed_steps()
                .saturating_sub(driver.base);
            let failure = request.stats().failure();
            let steps = driver.cases.front()?.steps;
            if let Some(at) = driver.cases.front()?.latch
                && completed >= at
                && failure == 0
            {
                if let Err(error) =
                    request.inject_latch(assets, commands, CANONICAL_FAILURE_NON_FINITE)
                {
                    return Some(Err(format!("the driver's latch was not set: {error}")));
                }
                let processed = request.stats().processed_event();
                state.pulse_amplitude = 1.0;
                state.pulse_width = WIDTH;
                state.place_pulse(Point2::new(-0.3, -0.2));
                driver.phase = EdgePhase::Latched {
                    at: completed,
                    processed,
                    queued: false,
                    since: Instant::now(),
                };
                return None;
            }
            if let Some(at) = driver.cases.front()?.fault
                && completed >= at
                && failure == 0
            {
                if let Err(error) =
                    request.inject_failure(assets, commands, CANONICAL_FAILURE_NON_FINITE, 0)
                {
                    return Some(Err(format!("the driver's fault was not armed: {error}")));
                }
                driver.phase = EdgePhase::Faulted {
                    armed: completed,
                    last: request.stats().processed_event(),
                    sent: 0,
                    events: Vec::new(),
                    since: Instant::now(),
                    first: true,
                };
                return None;
            }
            if failure == 0 && completed < steps {
                driver.phase = EdgePhase::Run;
                return None;
            }
            let seen = display.full_readbacks;
            request.request_full_state_readback();
            driver.phase = EdgePhase::Snapshot {
                seen,
                completed,
                failure,
                asked: Instant::now(),
            };
            None
        }
        EdgePhase::Snapshot {
            seen,
            completed,
            failure,
            asked,
        } => {
            let endpoint = driver.base + completed;
            let arrived = display.full_readbacks > seen
                && display.generation == request.generation()
                && display.full_snapshot_completed_steps() == endpoint;
            if !arrived {
                if asked.elapsed().as_secs() >= WAIT_SECONDS {
                    let mirror = driver.mirror.as_mut()?;
                    mirror.found.push(format!(
                        "no snapshot of step {completed} came{}",
                        if failure != 0 {
                            format!(" after the device failed ({failure})")
                        } else {
                            String::new()
                        }
                    ));
                    return Some(Ok(()));
                }
                request.request_full_state_readback();
                driver.phase = EdgePhase::Snapshot {
                    seen,
                    completed,
                    failure,
                    asked,
                };
                return None;
            }
            let category = driver.cases.front()?.category;
            let mirror = driver.mirror.as_mut()?;
            read_against(mirror, state, display, completed, failure);
            if category == "tails" {
                mirror
                    .found
                    .push(format!("branches {}", request.step_branches().join(", ")));
            }
            match driver.cases.front()?.remesh {
                Some(edge) if failure == 0 => {
                    let generation = request.generation();
                    state.editor.document.presentation.mesh_edge = edge;
                    driver.phase = EdgePhase::Remesh {
                        generation,
                        asked: Instant::now(),
                    };
                    None
                }
                _ => Some(Ok(())),
            }
        }
        EdgePhase::Remesh { generation, asked } => {
            let failure = request.stats().failure();
            let landed = published(state, request, display, generation).is_some();
            let mirror = driver.mirror.as_mut()?;
            if failure != 0 {
                mirror.found.push(format!(
                    "the handoff FAILED ({failure}: {}); {}",
                    canonical_failure_description(failure),
                    state.amr_error.as_deref().unwrap_or(&state.message)
                ));
                return Some(Ok(()));
            }
            if landed {
                mirror.found.push(format!(
                    "the handoff landed, {} nodes",
                    display.primary_flux.len()
                ));
                return Some(Ok(()));
            }
            if asked.elapsed().as_secs() >= WAIT_SECONDS {
                mirror.found.push(format!(
                    "no handoff came: {}",
                    state.amr_error.as_deref().unwrap_or(&state.message)
                ));
                return Some(Ok(()));
            }
            driver.phase = EdgePhase::Remesh { generation, asked };
            None
        }
        EdgePhase::Latched {
            at,
            processed,
            mut queued,
            since,
        } => {
            queued |= request.live_event_pending();
            let known = request.stats().failure();
            let answered = request.stats().processed_event() != processed;
            let idle = state.coordinator.pending_pulses.is_empty() && !request.live_event_pending();
            if answered && known != 0 {
                let rejection = request.stats().event_rejection();
                let mirror = driver.mirror.as_mut()?;
                mirror.outcome.latched_pulse = Some(rejection);
                mirror.found.push(format!(
                    "latched past step {at}, the pulse behind it {}",
                    if rejection == 0 {
                        "TAKEN".to_owned()
                    } else {
                        format!("refused ({rejection})")
                    }
                ));
                let completed = request
                    .stats()
                    .completed_steps()
                    .saturating_sub(driver.base);
                let seen = display.full_readbacks;
                request.request_full_state_readback();
                driver.phase = EdgePhase::Snapshot {
                    seen,
                    completed,
                    failure: known,
                    asked: Instant::now(),
                };
                return None;
            }
            // The app learned of the latch before it sent the pulse: the
            // case did not put an event behind it, and says nothing.
            if known != 0 && idle && !answered && !queued {
                return Some(Err(format!(
                    "the driver's pulse did not reach the device before the host read the \
                     latch: {}",
                    state.message
                )));
            }
            if since.elapsed().as_secs() >= WAIT_SECONDS {
                return Some(Err(format!(
                    "the driver's pulse behind the latch was not answered (queued {queued})"
                )));
            }
            driver.phase = EdgePhase::Latched {
                at,
                processed,
                queued,
                since,
            };
            None
        }
        EdgePhase::Faulted {
            armed,
            mut last,
            mut sent,
            mut events,
            since,
            first,
        } => {
            let processed = request.stats().processed_event();
            let known = request.stats().failure();
            let completed = request
                .stats()
                .completed_steps()
                .saturating_sub(driver.base);
            if processed != last {
                let rejection = request.stats().event_rejection();
                events.push(format!(
                    "event {processed} {} at step {completed}{}",
                    if rejection == 0 {
                        "TAKEN".to_owned()
                    } else {
                        format!("refused ({rejection})")
                    },
                    if known != 0 {
                        ", the failure known"
                    } else {
                        ""
                    }
                ));
                last = processed;
            }
            let idle = state.coordinator.pending_pulses.is_empty() && !request.live_event_pending();
            if (known != 0 && idle) || since.elapsed().as_secs() >= WAIT_SECONDS {
                let mirror = driver.mirror.as_mut()?;
                mirror.comparable = false;
                mirror.found.push(format!(
                    "armed past step {armed}, {sent} pulses placed{}; {}",
                    if known == 0 {
                        ", the app NEVER SAW a failure"
                    } else {
                        ""
                    },
                    if events.is_empty() {
                        "no event processed after the arm".to_owned()
                    } else {
                        events.join("; ")
                    }
                ));
                let seen = display.full_readbacks;
                request.request_full_state_readback();
                driver.phase = EdgePhase::Snapshot {
                    seen,
                    completed,
                    failure: known,
                    asked: Instant::now(),
                };
                return None;
            }
            if !first && known == 0 && idle && sent < 40 {
                state.pulse_amplitude = 0.5;
                state.pulse_width = WIDTH;
                state.place_pulse(Point2::new(-0.3, -0.2));
                sent += 1;
            }
            driver.phase = EdgePhase::Faulted {
                armed,
                last,
                sent,
                events,
                since,
                first: false,
            };
            None
        }
        EdgePhase::Done => None,
    }
}

/// Queues the case's strike `index`, or starts the run once none is left.
fn begin_strike(
    driver: &mut EdgeDriver,
    state: &mut Playground,
    request: &mut CanonicalGpuRequest,
    index: usize,
) -> Option<Result<(), String>> {
    let case = driver.cases.front()?;
    let Some(strike) = case.strikes.get(index).copied() else {
        state.e2e_steps.limit = Some(driver.base + case.steps);
        state.wave_running = true;
        driver.phase = EdgePhase::Run;
        return None;
    };
    state.pulse_amplitude = strike.amplitude;
    state.pulse_width = WIDTH;
    let processed = request.stats().processed_event();
    let before = state.coordinator.pending_pulses.len();
    state.place_pulse(strike.at);
    if state.coordinator.pending_pulses.len() != before + 1 {
        driver.mirror.as_mut()?.found.push(format!(
            "the app did not queue strike {index}: {}",
            state.message
        ));
        return begin_strike(driver, state, request, index + 1);
    }
    let queued = *state.coordinator.pending_pulses.back()?;
    driver.phase = EdgePhase::Strike {
        index,
        processed,
        queued,
    };
    None
}

/// Steps the reference as far as the device went and reads the snapshot
/// against it: what each side refused, the relative errors, the device's
/// peak and its books.
fn read_against(
    mirror: &mut Mirror,
    state: &Playground,
    display: &CanonicalGpuDisplay,
    completed: u64,
    failure: u32,
) {
    let stencils = probe_stencils(&mirror.active);
    let keeping = stencils.as_ref().is_ok_and(|stencils| !stencils.is_empty());
    // The reference over the run's first half, where the probes' records
    // have surely come back by the snapshot.
    let mut kept = Vec::new();
    let mut keep = |reference: &State| {
        kept.push((
            reference.time(),
            reference.primary().to_vec(),
            reference.complementary().to_vec(),
        ))
    };
    if keeping {
        keep(&mirror.state);
    }
    let mut cpu_stopped = None;
    for step in 0..completed {
        if let Err(error) = mirror.state.step(&mirror.active) {
            cpu_stopped = Some(format!("the reference REFUSED step {step}: {error}"));
            break;
        }
        if keeping && step < completed / 2 {
            keep(&mirror.state);
        }
    }
    match &stencils {
        Ok(stencils) if keeping => mirror.found.push(read_probes(
            state,
            stencils,
            &kept,
            &mirror.active.canonical_operator,
            mirror.state.time_step(),
        )),
        Ok(_) => {}
        Err(error) => mirror.found.push(format!("probes not read: {error}")),
    }
    mirror.outcome.completed = completed;
    mirror.outcome.failure = failure;
    mirror.outcome.reference_stopped = cpu_stopped.is_some();
    if failure != 0 {
        let next = mirror.state.clone().step(&mirror.active);
        mirror.outcome.reference_went_on = next.is_ok();
        mirror.found.push(format!(
            "the device FAILED after {completed} steps ({failure}: {}), the reference {}",
            canonical_failure_description(failure),
            match next {
                Ok(()) => "took the next step".to_owned(),
                Err(error) => format!("refused it too ({error})"),
            }
        ));
    } else {
        mirror.found.push(format!("ran {completed} steps"));
    }
    if let Some(stopped) = cpu_stopped {
        mirror.found.push(stopped);
        mirror.comparable = false;
    }
    let peak = display
        .primary_flux
        .iter()
        .fold(0.0_f64, |peak, value| peak.max(f64::from(value.abs())));
    let finite = display.primary_flux.iter().all(|value| value.is_finite())
        && display
            .complementary_flux
            .iter()
            .all(|value| value.iter().all(|lane| lane.is_finite()));
    let expected_peak = mirror
        .state
        .primary()
        .iter()
        .fold(0.0_f64, |peak, value| peak.max(value.abs()));
    mirror.found.push(format!(
        "peak {peak:.3e} (reference {expected_peak:.3e}){}",
        if finite { "" } else { ", NON-FINITE" }
    ));
    mirror.outcome.peak = peak;
    mirror.outcome.reference_peak = expected_peak;
    mirror.outcome.finite = finite;
    if mirror.comparable && expected_peak == 0.0 {
        // Nothing to divide by: the device is to hold nothing as well.
        mirror.found.push(if peak == 0.0 {
            mirror.outcome.error = Some(0.0);
            "both zero".to_owned()
        } else {
            "the device holds a field where the reference holds none".to_owned()
        });
    } else if mirror.comparable {
        match state_errors(&mirror.state, display, None) {
            Ok((q, b, r)) => {
                mirror.outcome.error = Some(q.max(b).max(r));
                mirror
                    .found
                    .push(format!("Q {q:.2e}, b {b:.2e}, r {r:.2e}"))
            }
            Err(error) => mirror.found.push(format!("not compared: {error}")),
        }
    }
    let books_finite = display.accounting.iter().all(|value| value.is_finite());
    mirror.outcome.books_finite = books_finite;
    mirror.found.push(if books_finite {
        "books finite".to_owned()
    } else {
        format!("books NON-FINITE {:?}", display.accounting)
    });
}

/// Whether `outcome` did what `expect` asks of it, a case of `strikes`.
fn judge(expect: Expect, outcome: Option<&Outcome>, strikes: usize) -> Result<(), String> {
    let Some(outcome) = outcome else {
        return Err("the case never ran".into());
    };
    let taken = outcome.strikes.len() == strikes && outcome.strikes.iter().all(|code| *code == 0);
    let within = |tolerance: f64| outcome.error.is_some_and(|error| error <= tolerance);
    let off = || format!("off the reference by {:?}", outcome.error);
    match expect {
        Expect::Studied => Ok(()),
        Expect::Matches(tolerance) => {
            if !taken {
                Err(format!("the strikes were answered {:?}", outcome.strikes))
            } else if outcome.failure != 0 {
                Err(format!("the device failed with {}", outcome.failure))
            } else if outcome.reference_stopped {
                Err("the reference refused a step the device took".into())
            } else if !outcome.finite || !outcome.books_finite {
                Err("the field or the books are not finite".into())
            } else if !within(tolerance) {
                Err(off())
            } else {
                Ok(())
            }
        }
        Expect::BothRefuse(code) => {
            if outcome.failure != code {
                Err(format!("the device failed with {}", outcome.failure))
            } else if outcome.reference_went_on || outcome.reference_stopped {
                Err("the reference did not refuse the same step".into())
            } else {
                Ok(())
            }
        }
        Expect::DeviceFails(code, tolerance) => {
            if outcome.failure != code {
                Err(format!("the device failed with {}", outcome.failure))
            } else if !outcome.reference_went_on {
                Err("the reference refused the step too".into())
            } else if !outcome.books_finite {
                Err("the accepted books are not finite".into())
            } else if !within(tolerance) {
                Err(off())
            } else {
                Ok(())
            }
        }
        Expect::Bounded(factor) => {
            if outcome.failure != 0 {
                Err(format!("the device failed with {}", outcome.failure))
            } else if !outcome.finite || !outcome.books_finite {
                Err("the field or the books are not finite".into())
            } else if !(outcome.reference_peak > 0.0
                && outcome.peak <= factor * outcome.reference_peak)
            {
                Err(format!(
                    "a peak of {:.3e} against the reference's {:.3e}",
                    outcome.peak, outcome.reference_peak
                ))
            } else {
                Ok(())
            }
        }
        Expect::LatchedPulseRefused(tolerance) => {
            if outcome.failure == 0 || outcome.latched_pulse != Some(outcome.failure) {
                Err(format!(
                    "the pulse behind the latch ({}) was answered {:?}",
                    outcome.failure, outcome.latched_pulse
                ))
            } else if !within(tolerance) {
                Err(off())
            } else {
                Ok(())
            }
        }
    }
}

/// A probe as the reference reads it.
enum ProbeStencil {
    Point(Box<CanonicalPointStencil>),
    Line(Vec<CanonicalPointStencil>),
    Area(QuadraticAreaStencil),
}

/// The generation's probes, compiled as the device's recorders were.
fn probe_stencils(active: &PreparedTopology) -> Result<Vec<(ProbeId, ProbeStencil)>, String> {
    let operator = &active.canonical_operator;
    active
        .probes
        .iter()
        .map(|probe| {
            let TopologyProbeCompilation::Ready(stencil) = &probe.result else {
                return Err(format!("probe {:?} did not compile", probe.id));
            };
            let read = match stencil.as_ref().clone() {
                TopologyProbeStencil::Point(stencil) => ProbeStencil::Point(Box::new(
                    CanonicalPointStencil::from_quadratic(stencil, operator)
                        .map_err(|error| error.to_string())?,
                )),
                TopologyProbeStencil::Segment(stencils) => ProbeStencil::Line(
                    stencils
                        .into_iter()
                        .map(|stencil| CanonicalPointStencil::from_quadratic_own(stencil, operator))
                        .collect::<Result<_, _>>()
                        .map_err(|error| error.to_string())?,
                ),
                TopologyProbeStencil::Area(stencil) => ProbeStencil::Area(stencil),
                TopologyProbeStencil::Boundary(_) => {
                    return Err(format!("probe {:?} is a boundary's", probe.id));
                }
            };
            Ok((probe.id, read))
        })
        .collect()
}

/// Every record the app holds of the run's first half against the
/// reference's reading at its step: the field at a point, at each point of
/// a line, and its mean over a disc, over the reference's peak.
fn read_probes(
    state: &Playground,
    stencils: &[(ProbeId, ProbeStencil)],
    kept: &[(f64, Vec<f64>, Vec<Point2>)],
    operator: &funfern_core::CanonicalWaveOperator,
    time_step: f64,
) -> String {
    let scale = kept
        .iter()
        .flat_map(|(_, primary, _)| primary.iter())
        .fold(0.0_f64, |peak, value| peak.max(value.abs()));
    let at = |time: f64| {
        kept.iter()
            .find(|(kept, _, _)| (kept - time).abs() < 0.5 * time_step)
    };
    let rate = vec![0.0; operator.degrees_of_freedom()];
    let mut worst = (0.0_f64, String::new());
    let mut records = 0_usize;
    let mut silent = Vec::new();
    let mut unread = Vec::new();
    let mut grow = |error: f64, what: String| {
        // A NaN is the worst there is, and stays.
        if !worst.0.is_nan() && (error.is_nan() || error > worst.0) {
            worst = (error, what);
        }
    };
    for (id, stencil) in stencils {
        let mut seen = 0_usize;
        match stencil {
            ProbeStencil::Point(stencil) => {
                for record in state
                    .probe_traces
                    .get(id)
                    .map(|trace| &trace.samples)
                    .into_iter()
                    .flatten()
                {
                    let Some((_, primary, complementary)) = at(record.time) else {
                        continue;
                    };
                    seen += 1;
                    match stencil.sample(primary, &rate, complementary) {
                        Ok(read) => grow(
                            (record.displacement - read.primary).abs() / scale,
                            format!("point {} at {:.3} s", id.0, record.time),
                        ),
                        Err(error) => unread.push(format!("point {}: {error}", id.0)),
                    }
                }
            }
            ProbeStencil::Line(stencils) => {
                for record in state
                    .curve_probe_traces
                    .get(id)
                    .map(|trace| &trace.records)
                    .into_iter()
                    .flatten()
                {
                    let Some((_, primary, complementary)) = at(record.time) else {
                        continue;
                    };
                    seen += 1;
                    if record.displacement.len() != stencils.len() {
                        unread.push(format!(
                            "line {} holds {} points where it has {}",
                            id.0,
                            record.displacement.len(),
                            stencils.len()
                        ));
                        continue;
                    }
                    for (index, (value, stencil)) in
                        record.displacement.iter().zip(stencils).enumerate()
                    {
                        match stencil.sample(primary, &rate, complementary) {
                            Ok(read) => grow(
                                (f64::from(*value) - read.primary).abs() / scale,
                                format!("line {} point {index} at {:.3} s", id.0, record.time),
                            ),
                            Err(error) => unread.push(format!("line {}: {error}", id.0)),
                        }
                    }
                }
            }
            ProbeStencil::Area(stencil) => {
                for record in state
                    .area_probe_traces
                    .get(id)
                    .map(|trace| &trace.records)
                    .into_iter()
                    .flatten()
                {
                    let Some((_, primary, complementary)) = at(record.time) else {
                        continue;
                    };
                    seen += 1;
                    match sample_canonical_area(stencil, operator, primary, complementary) {
                        Ok(read) => grow(
                            (record.mean_displacement - read.mean_primary).abs() / scale,
                            format!("disc {} at {:.3} s", id.0, record.time),
                        ),
                        Err(error) => unread.push(format!("disc {}: {error}", id.0)),
                    }
                }
            }
        }
        records += seen;
        if seen == 0 {
            silent.push(id.0.to_string());
        }
    }
    let mut said = format!(
        "{} probes, {records} records read, worst {:.2e} ({})",
        stencils.len(),
        worst.0,
        worst.1
    );
    if !silent.is_empty() {
        said.push_str(&format!("; NOTHING RECORDED by {}", silent.join(", ")));
    }
    if !unread.is_empty() {
        unread.dedup();
        said.push_str(&format!("; UNREAD {}", unread.join(", ")));
    }
    said
}
