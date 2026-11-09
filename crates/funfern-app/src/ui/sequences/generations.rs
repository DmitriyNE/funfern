//! The runtime under generated interleavings of host frames, device
//! completions and user actions.
//!
//! The host prepares a candidate, packs it, uploads it and publishes it once
//! the device has taken it, while edits, Reset, Remesh, scene replacements,
//! pulses and Switch presses arrive, and the device finishes things in an
//! order of its own. The defects of this area each needed one particular
//! order: a waiting pulse sent into the run Reset had just cleared, an upload
//! waiting for a generation that never came, a rejected handoff that kept the
//! candidate's step. Here the order is drawn. A frame lends preparation some
//! work units, the device delivers readbacks, settles a handoff or a live
//! event either way, or faults, and the user acts in between.
//!
//! The device is `canonical_gpu`'s test device: it writes what the shaders
//! write into the buffers the readbacks map, each frame takes copies of them,
//! and the copies arrive through the production observers and settle
//! systems, on the entity they were taken for. A copy can arrive frames after
//! it was taken, after a Reset, a replacement, a handoff or a cleared failure
//! has made it another generation's (RT-03); the device can settle a handoff,
//! a live event or a failure whose news waits in a copy. Preparation advances
//! by the units each frame is granted, on the frame or on a worker the
//! sequence plays, and packing happens on the frame or, where the sequence
//! plays the worker, when it says, so the order is the sequence's alone. The
//! solve for the stationary flux a handoff leaves answers when the sequence
//! says too, since the device serves no full snapshot to start it.
//!
//! The harness keeps what it has watched the device come to run: the
//! generation and the host token it carries, from the upload or Reset that
//! installed it or the handoff that published it. After every step:
//! - while nothing uploads, the host's active topology is the one the device
//!   runs, and the host paces the step the device was packed for;
//! - a topology is only ever prepared from an accepted scene the editor has
//!   held since the scene was last replaced;
//! - a commit with no new generation is one that needs none;
//! - a Reset or a replacement leaves nothing queued for the run it ended.
//!
//! And at the end, with preparation and the device fair, the runtime comes to
//! rest running the editor's accepted revision, or with an error naming it.
//!
//! Beside the runtime runs the protocol model (`funfern-protocol`), which
//! Kani checks for every sequence of steps to a bounded depth. Each step is
//! given to it with the inputs the runtime met, from the runtime's
//! `protocol_log` and from what an edit changed, and after each the two are
//! compared field by field in the runtime's own tokens. The device's steps
//! are what its copies told the host: a readback of the running generation,
//! a handoff or a live event settled, a failure read. A copy of another
//! generation is no step, and the comparison then holds that it changed
//! nothing. Where they part, either the runtime has left the protocol the
//! proofs are about or the model does not describe it; either way the proofs
//! say nothing until it is settled.

use super::*;
use crate::canonical_gpu::CanonicalGpuHandoffOutcome;
use crate::canonical_gpu::test_device::{self, Arrival, Kind, TestDevice};
use crate::topology_editor::TopologyEditor;
use crate::topology_runtime::{PreparedSolverUpdate, TopologyPreparationJob, TopologyToken};
use bevy::ecs::world::CommandQueue;
use funfern_protocol as model;
use std::sync::mpsc;

/// One step of the interleaving.
#[derive(Clone, Debug)]
enum Act {
    /// A host frame lending preparation that many slices; `None`, all it
    /// needs.
    Frame(Option<usize>),
    /// The device completes up to that many more of the requested steps,
    /// and every copy in flight arrives, then a fresh one of each readback.
    Readbacks(u64),
    /// The device settles a pending handoff, admitting the target or not,
    /// and its copies arrive as [`Act::Readbacks`]' do.
    Handoff(bool),
    /// The device settles a pending live event, taking it or refusing it,
    /// and its copies arrive.
    Event(bool),
    /// The run fails at its last accepted step, and its copies arrive.
    Fault,
    /// The oldest frame's copies arrive, the `nth` first.
    Deliver(u8),
    /// [`Act::Handoff`], the receipt left in the transfer buffer for a copy.
    Admit(bool),
    /// [`Act::Event`], the result left in the control word for a copy.
    Process(bool),
    /// [`Act::Fault`], the failure left in the status buffer for a copy.
    Latch,
    /// The played preparation worker works that many slices; `None`, all
    /// its job needs.
    Work(Option<usize>),
    /// The oldest pack held back is made and sent to the frame waiting for
    /// it, if one still does; every one, if `all`.
    Pack(bool),
    /// The solve for the stationary flux the last handoff left answers: a
    /// part found to take away, or none.
    Clean(bool),
    /// The draft's validation finishes.
    Validate,
    /// Edit the `nth` material the `kind`th of four ways.
    Material {
        nth: u8,
        kind: u8,
    },
    /// Drag the first curve's first control point by the `nth` offset.
    Nudge(u8),
    /// Set the mesh edge to the `nth` of three.
    Edge(u8),
    /// Press Remesh: rebuild at the mesh edge set, though nothing changed.
    Remesh,
    /// Set the simulation speed to the `nth` of three.
    Speed(u8),
    /// Move the point source to the `cell`th cell, on or off, at one of two
    /// frequencies.
    Source {
        cell: u8,
        on: bool,
        high: bool,
    },
    Reset,
    Run(bool),
    Step,
    /// Place a pulse in the `cell`th cell.
    Pulse(u8),
    /// Press Switch's hotkey: the open material's Switch, or the scene's
    /// only one. The button itself waits for the medium to run.
    Switch,
    /// Open one of the two fixture scenes, as an example opens.
    Open(bool),
    Undo,
    Redo,
    /// An estimate of a field struck in the `cell`th cell, finished, as a
    /// full readback of the running generation would have led to.
    Adapt(u8),
}

fn acts() -> BoxedStrategy<Act> {
    let settle = || prop_oneof![3 => Just(true), 1 => Just(false)];
    prop_oneof![
        12 => prop_oneof![Just(Some(0)), Just(Some(1)), Just(Some(3)), Just(None)]
            .prop_map(Act::Frame),
        6 => prop_oneof![Just(0), Just(1), Just(16), Just(u64::MAX)].prop_map(Act::Readbacks),
        3 => settle().prop_map(Act::Handoff),
        3 => settle().prop_map(Act::Event),
        1 => Just(Act::Fault),
        6 => any::<u8>().prop_map(Act::Deliver),
        2 => settle().prop_map(Act::Admit),
        2 => settle().prop_map(Act::Process),
        1 => Just(Act::Latch),
        2 => prop_oneof![Just(Some(1)), Just(Some(3)), Just(None)].prop_map(Act::Work),
        3 => any::<bool>().prop_map(Act::Pack),
        2 => prop_oneof![3 => Just(true), 1 => Just(false)].prop_map(Act::Clean),
        3 => Just(Act::Validate),
        3 => (any::<u8>(), any::<u8>()).prop_map(|(nth, kind)| Act::Material { nth, kind }),
        2 => any::<u8>().prop_map(Act::Nudge),
        1 => any::<u8>().prop_map(Act::Edge),
        1 => Just(Act::Remesh),
        1 => any::<u8>().prop_map(Act::Speed),
        2 => (any::<u8>(), any::<bool>(), any::<bool>())
            .prop_map(|(cell, on, high)| Act::Source { cell, on, high }),
        2 => Just(Act::Reset),
        1 => any::<bool>().prop_map(Act::Run),
        1 => Just(Act::Step),
        3 => any::<u8>().prop_map(Act::Pulse),
        2 => Just(Act::Switch),
        1 => any::<bool>().prop_map(Act::Open),
        1 => Just(Act::Undo),
        1 => Just(Act::Redo),
    ]
    .boxed()
}

/// [`fixture`], adapting if `adaptive`.
fn scene(other: bool, adaptive: bool) -> TopologyDocument {
    let mut document = fixture(other);
    document.presentation.adaptation.enabled = adaptive;
    document
}

/// A small scene to run: the unit square on a coarse mesh, a subdomain of a
/// switchable material, the point source on, adaptation off. `other` moves
/// the subdomain and coarsens the mesh, for a replacement that differs.
fn fixture(other: bool) -> TopologyDocument {
    fixture_of(other, true)
}

/// [`fixture`], its subdomain's medium switchable or left linear: a fixed,
/// lossless generation, onto which a handoff's carry leaves a stationary
/// flux to take away.
fn fixture_of(other: bool, switchable: bool) -> TopologyDocument {
    let mut editor = TopologyEditor::default();
    let subdomain = editor.add_material().unwrap();
    if switchable {
        let material = editor
            .document
            .model
            .draft
            .material(subdomain)
            .unwrap()
            .clone();
        let switch = law_presets()
            .iter()
            .find(|preset| preset.name == "Switchable medium" && preset.row == LawPresetRow::Mass)
            .unwrap();
        editor
            .update_material(apply_law_preset(switch, &material).unwrap())
            .unwrap();
    }
    let centre = if other {
        Point2::new(0.35, -0.25)
    } else {
        Point2::new(-0.4, 0.1)
    };
    editor
        .create_closed_curve(
            PeriodicCubicSpline::rounded(centre, 0.25),
            ClosedCurvePurpose::Subdomain {
                material: subdomain,
            },
        )
        .unwrap();
    let mut source = editor.document.model.source;
    source.enabled = true;
    source.position = Point2::new(0.3, 0.4);
    editor.set_point_source(source).unwrap();
    settle(&mut editor);
    let mut document = editor.document.clone();
    document.presentation.mesh_edge = if other { 0.3 } else { 0.25 };
    document.presentation.adaptation.enabled = false;
    document
}

/// Whether `active` was prepared from what a preparation reads of `document`:
/// its accepted scene, source, probes and far field. The source's region is
/// left out, as the preparation resolves it.
fn runs_document(active: &PreparedTopology, document: &TopologyDocument) -> bool {
    let model = &document.model;
    *active.bundle.authored == model.accepted
        && active.point_source
            == PointSource {
                region: active.point_source.region,
                ..model.source
            }
        && *active.probe_definitions == model.probes[..]
        && active.far_field_settings == model.far_field
}

/// The host and the device under test, and what the harness has watched.
struct Session {
    state: Playground,
    world: World,
    request: CanonicalGpuRequest,
    display: CanonicalGpuDisplay,
    recorders: WaveGpuRequest,
    vector: VectorOverlayDisplay,
    /// The generation the device runs and the host token it carries.
    running: Option<(u64, TopologyToken)>,
    /// The accepted scenes the editor has held since the scene was last
    /// replaced: all a topology may be prepared from.
    scenes: Vec<TopologyScene>,
    /// The revision whose draft was last validated: what the runtime is to
    /// come to run.
    accepted_revision: u64,
    /// The protocol model, stepped beside the runtime with the inputs it
    /// met, and the model's token for each real request.
    model: model::Protocol,
    /// The real token each of the model's requests produced, by the model's
    /// token. Two requests that change neither the document nor the mesh
    /// produce one real token, and the runtime cannot tell them apart.
    tokens: Vec<TopologyToken>,
    /// What a preparation reads, each content met so far, by the name the
    /// model knows it as; the same for mesh edges.
    inputs: Vec<PreparedInputs>,
    edges: Vec<f64>,
    /// Whether the scenes adapt, and the frames run adaptation after the
    /// runtime as the app's do.
    adaptive: bool,
    /// The field the display paints, which adaptation reads beside the
    /// device's.
    wave: WaveDisplay,
    /// Each adapted request, and the topology active when it was made: the
    /// generation it may publish over.
    adapted_from: Vec<(TopologyToken, Option<TopologyToken>)>,
    /// The device, and the copies its readbacks have in flight.
    device: TestDevice,
    /// What the copies that arrived in this step told the host, as the
    /// model's steps.
    told: Vec<model::Step>,
    /// The preparation worker, where the sequence plays it.
    worker: Option<Worker>,
    /// What the host had read when the last step ended.
    read: Read,
}

/// What the host had read: the display's generation and its clock's step,
/// and the running generation's steps and processed event.
#[derive(Default)]
struct Read {
    display: u64,
    clock: Option<(u64, u64)>,
    counted: Option<(u64, u64, u32)>,
}

/// The preparation worker, played on the sequence's word: it takes the jobs
/// the host submits and sends what the host reads, as
/// `run_preparation_worker` does, working only the slices it is lent.
struct Worker {
    jobs: mpsc::Receiver<TopologyPreparationJob>,
    events: mpsc::Sender<BackgroundPreparationEvent>,
    job: Option<TopologyPreparationJob>,
}

impl Worker {
    /// The host's end of the worker's channels, and the worker's.
    fn connect() -> (BackgroundPreparationWorker, Self) {
        let (job_sender, jobs) = mpsc::channel();
        let (events, event_receiver) = mpsc::channel();
        let host = BackgroundPreparationWorker {
            sender: job_sender,
            receiver: Mutex::new(event_receiver),
            token: None,
            phase: None,
            detail: None,
            timing: None,
        };
        (
            host,
            Self {
                jobs,
                events,
                job: None,
            },
        )
    }

    /// Works `slices` of the newest job, taking any newer one between them
    /// and reporting after each, as the worker does between its quanta.
    fn work(&mut self, slices: usize) {
        for _ in 0..slices {
            while let Ok(newer) = self.jobs.try_recv() {
                self.job = Some(newer);
            }
            let Some(job) = &mut self.job else {
                return;
            };
            let token = job.token();
            let event = match job.advance(4096) {
                Some(result) => {
                    self.job = None;
                    BackgroundPreparationEvent::Finished {
                        token,
                        result: Box::new(result),
                    }
                }
                None => BackgroundPreparationEvent::Progress {
                    token,
                    phase: job.phase(),
                    detail: job.detail(),
                    timing: job.timing(),
                },
            };
            let _ = self.events.send(event);
        }
    }
}

/// What the model is told an edit changed, read before it.
struct Was {
    revision: u64,
    acceptance: TopologyAcceptance,
    mesh_edge: f64,
    pulses: usize,
    switches: usize,
    history: (usize, usize),
    undo_replaces: bool,
    redo_replaces: bool,
}

/// The state the model and the runtime are compared on, with tokens as the
/// runtime knows them.
#[derive(Debug, PartialEq)]
struct Projection {
    active: Option<TopologyToken>,
    uploading: Option<(TopologyToken, u8)>,
    source_commit: Option<TopologyToken>,
    packed: Option<TopologyToken>,
    pack_back: bool,
    ready: Option<TopologyToken>,
    preparing: bool,
    last_error: Option<TopologyToken>,
    request_stands: bool,
    reset: bool,
    remesh: bool,
    drop: bool,
    pulses: usize,
    switches: usize,
    fault: bool,
    cleanup: Option<(u8, bool)>,
    running: bool,
    stepping: bool,
    generation: u8,
    installed: bool,
    carries: Option<TopologyToken>,
    ready_status: bool,
    failed: bool,
    behind: bool,
    handoff: bool,
    refused: bool,
    event: bool,
    display: u8,
}

/// What a step is judged against, read before it.
struct Before {
    generation: u64,
    active: Option<TopologyToken>,
    uploading: Option<(TopologyToken, u64)>,
    reset: bool,
    drop: bool,
}

impl Session {
    /// The fixture opened as at launch, with preparation and packing on the
    /// sequence's word and the device fresh.
    fn start() -> Self {
        Self::opening(false)
    }

    /// [`Self::start`] with adaptation on, its jobs on the frame and on the
    /// sequence's word too.
    fn adapting() -> Self {
        Self::opening(true)
    }

    /// [`Self::start`] with preparation on a worker the sequence plays, and
    /// each pack made when it says, as the app's packing worker answers
    /// frames later. Its scene opens fixed, so that its handoffs leave a
    /// stationary flux to take away until an edit makes the medium
    /// switchable.
    fn played() -> Self {
        let mut session = Self::opening_with(fixture_of(false, false), false);
        let (host, worker) = Worker::connect();
        session.state.background_preparation = Some(host);
        session.state.preparation_grant = None;
        session.state.held_packs = Some(Vec::new());
        session.worker = Some(worker);
        session
    }

    fn opening(adaptive: bool) -> Self {
        Self::opening_with(scene(false, adaptive), adaptive)
    }

    fn opening_with(document: TopologyDocument, adaptive: bool) -> Self {
        let mut state = Playground {
            background_preparation: None,
            background_amr: None,
            preparation_grant: Some(usize::MAX),
            amr_grant: Some(usize::MAX),
            pack_inline: true,
            protocol_log: ProtocolLog::kept(),
            ..Playground::default()
        };
        state.set_document(document, false, true).unwrap();
        let mut world = World::new();
        world.init_resource::<Assets<ShaderBuffer>>();
        test_device::connect(&mut world);
        let scenes = vec![state.editor.document.model.accepted.clone()];
        Self {
            state,
            world,
            request: CanonicalGpuRequest::default(),
            display: CanonicalGpuDisplay::default(),
            recorders: WaveGpuRequest::default(),
            vector: VectorOverlayDisplay::default(),
            running: None,
            scenes,
            accepted_revision: 0,
            model: model::Protocol::default(),
            tokens: Vec::new(),
            inputs: Vec::new(),
            edges: Vec::new(),
            adaptive,
            wave: WaveDisplay::default(),
            adapted_from: Vec::new(),
            device: TestDevice::default(),
            told: Vec::new(),
            worker: None,
            read: Read::default(),
        }
        .named()
    }

    /// Runs `f` on the world with the request, the display and the recorders
    /// in it, where the observers and settle systems look for them.
    fn in_world<R>(&mut self, f: impl FnOnce(&mut World, &mut TestDevice) -> R) -> R {
        self.world
            .insert_resource(std::mem::take(&mut self.request));
        self.world
            .insert_resource(std::mem::take(&mut self.display));
        self.world
            .insert_resource(std::mem::take(&mut self.recorders));
        let result = f(&mut self.world, &mut self.device);
        self.request = self.world.remove_resource().unwrap();
        self.display = self.world.remove_resource().unwrap();
        self.recorders = self.world.remove_resource().unwrap();
        result
    }

    /// The copies `deliver` brings arrive, and the settle systems run after
    /// them: what they told the host is kept as the model's steps.
    fn arrive(&mut self, deliver: impl FnOnce(&mut World, &mut TestDevice) -> Vec<Arrival>) {
        let failed = self.request.failed();
        let handoff = self.request.handoff_outcome() == CanonicalGpuHandoffOutcome::Pending;
        let event = self.request.live_event_pending();
        let arrivals = self.in_world(|world, device| {
            let arrivals = deliver(world, device);
            TestDevice::settle(world);
            arrivals
        });
        // A readback of the running generation; one of another changes
        // nothing. Whether every step asked for has completed is the count's
        // to say: `caught_up` also reads a failed run as caught up, which
        // the model is told on its own, after this.
        if arrivals
            .iter()
            .any(|arrival| arrival.current && matches!(arrival.kind, Kind::Control | Kind::State))
        {
            self.told.push(model::Step::Readbacks {
                caught_up: self.request.stats().completed_steps() >= self.request.requested_steps(),
            });
        }
        if handoff {
            match self.request.handoff_outcome() {
                CanonicalGpuHandoffOutcome::Accepted => {
                    self.told.push(model::Step::Handoff { admit: true })
                }
                CanonicalGpuHandoffOutcome::Rejected(_) => {
                    self.told.push(model::Step::Handoff { admit: false })
                }
                _ => {}
            }
        }
        if event && !self.request.live_event_pending() {
            self.told.push(model::Step::Settle {
                take: self.request.stats().event_rejection() == 0,
            });
        }
        if !failed && self.request.failed() {
            self.told.push(model::Step::Fault);
        }
    }

    /// The device does what `does` does, and then every copy in flight
    /// arrives, and a frame's fresh copies after them.
    fn answer(&mut self, does: impl FnOnce(&mut TestDevice, &World)) {
        self.arrive(|world, device| device.answer(world, does));
    }

    /// The model told the scene's inputs by name, as at launch.
    fn named(mut self) -> Self {
        self.model.inputs = self.inputs_name();
        self.model.edge = self.edge_name();
        self
    }

    /// The model's name for the mesh edge now.
    fn edge_name(&mut self) -> u8 {
        let edge = self.state.editor.document.presentation.mesh_edge;
        let index = match self.edges.iter().position(|known| *known == edge) {
            Some(index) => index,
            None => {
                self.edges.push(edge);
                self.edges.len() - 1
            }
        };
        index as u8
    }

    /// The model's name for what a preparation reads of the document now.
    fn inputs_name(&mut self) -> u8 {
        let document = &self.state.editor.document;
        let index = match self.inputs.iter().position(|known| known.read(document)) {
            Some(index) => index,
            None => {
                self.inputs.push(PreparedInputs::of(document));
                self.inputs.len() - 1
            }
        };
        index as u8
    }

    fn was(&self) -> Was {
        let document = &self.state.editor.document;
        Was {
            revision: self.state.editor.revision,
            acceptance: self.state.editor.acceptance,
            mesh_edge: document.presentation.mesh_edge,
            pulses: self.state.coordinator.pending_pulses.len(),
            switches: self.state.coordinator.pending_switches.len(),
            history: self.state.editor.history_len(),
            undo_replaces: self.state.editor.undo_replaces_scene(),
            redo_replaces: self.state.editor.redo_replaces_scene(),
        }
    }

    /// The real token the model's `token` names.
    fn real(&self, token: model::Token) -> TopologyToken {
        self.tokens[token as usize]
    }

    /// Whether the draft as it stands would validate.
    fn draft_valid(&self) -> bool {
        self.state.editor.document.model.draft.compile(0).is_ok()
    }

    /// The model's step for `act`, with the inputs the runtime met.
    fn model_step(&mut self, act: &Act, was: &Was, log: &[ProtocolNote]) -> Option<model::Step> {
        let inputs = self.inputs_name();
        let state = &self.state;
        Some(match act {
            Act::Frame(_) => {
                let carry = log.iter().find_map(|note| match *note {
                    ProtocolNote::Carry { same_step, update } => Some((same_step, update)),
                    _ => None,
                });
                let patch = log.iter().find_map(|note| match *note {
                    ProtocolNote::Patch { built, packable } => Some((built, packable)),
                    _ => None,
                });
                let begin = log.iter().find_map(|note| match *note {
                    ProtocolNote::Begin { install, ok } => Some((install, ok)),
                    _ => None,
                });
                model::Step::Frame(model::Frame {
                    validated: was.acceptance == TopologyAcceptance::Pending
                        && state.editor.acceptance != TopologyAcceptance::Pending,
                    inputs,
                    prepared: log
                        .iter()
                        .find_map(|note| match *note {
                            ProtocolNote::Prepared(true) => Some(model::Prepared::Done),
                            ProtocolNote::Prepared(false) => Some(model::Prepared::Failed),
                            _ => None,
                        })
                        .unwrap_or(model::Prepared::NotYet),
                    carried: model::Carried {
                        update: carry.map_or(model::decisions::Update::Full, |(_, update)| update),
                        same_step: carry.is_some_and(|(same_step, _)| same_step),
                        built: patch.is_none_or(|(built, _)| built),
                        packable: patch.is_none_or(|(_, packable)| packable),
                    },
                    packed: log.contains(&ProtocolNote::Packed),
                    install: begin.is_some_and(|(install, _)| install),
                    begun: begin.is_none_or(|(_, ok)| ok),
                    pulse: log
                        .iter()
                        .find_map(|note| match *note {
                            ProtocolNote::Pulse(built) => Some(built),
                            _ => None,
                        })
                        .unwrap_or(true),
                    switch: log
                        .iter()
                        .find_map(|note| match *note {
                            ProtocolNote::Switch(built) => Some(built),
                            _ => None,
                        })
                        .unwrap_or(true),
                    behind: !self.request.caught_up(),
                    retime: log.contains(&ProtocolNote::Retimed),
                    adapted: log
                        .iter()
                        .any(|note| matches!(note, ProtocolNote::Adapted(_))),
                    cleans: log.contains(&ProtocolNote::CleanupDue),
                    cleanup: log
                        .iter()
                        .find_map(|note| match *note {
                            ProtocolNote::CleanupSolved(true) => Some(model::Solve::Found),
                            ProtocolNote::CleanupSolved(false) => Some(model::Solve::Nothing),
                            _ => None,
                        })
                        .unwrap_or(model::Solve::NotYet),
                })
            }
            // The device's steps are what its copies told the host, kept as
            // they arrived; the worker's are the frames that read them.
            Act::Readbacks(_)
            | Act::Handoff(_)
            | Act::Event(_)
            | Act::Fault
            | Act::Deliver(_)
            | Act::Admit(_)
            | Act::Process(_)
            | Act::Latch
            | Act::Work(_)
            | Act::Pack(_)
            | Act::Clean(_)
            | Act::Adapt(_) => return None,
            Act::Validate => model::Step::Validate { inputs },
            Act::Reset => model::Step::Reset,
            Act::Remesh => model::Step::Remesh,
            Act::Run(on) => model::Step::Run(*on),
            Act::Step => model::Step::StepOnce,
            Act::Pulse(_) => {
                if state.coordinator.pending_pulses.len() <= was.pulses {
                    return None;
                }
                model::Step::Pulse
            }
            Act::Switch => model::Step::Switch {
                queued: state.coordinator.pending_switches.len() > was.switches,
            },
            Act::Edge(_) => {
                if state.editor.document.presentation.mesh_edge == was.mesh_edge {
                    return None;
                }
                let edge = self.edge_name();
                model::Step::Edge { edge }
            }
            // The speed reaches the model through the frames that retime,
            // and asks for a retime again.
            Act::Speed(_) => model::Step::Speed,
            Act::Material { .. }
            | Act::Nudge(_)
            | Act::Source { .. }
            | Act::Open(_)
            | Act::Undo
            | Act::Redo => {
                // A scene opened replaces this one, and so does a step of the
                // history that crosses a replacement.
                let moved = state.editor.history_len() != was.history;
                let replaced = match act {
                    Act::Open(_) => true,
                    Act::Undo => moved && was.undo_replaces,
                    Act::Redo => moved && was.redo_replaces,
                    _ => false,
                };
                if replaced {
                    let edge = self.edge_name();
                    model::Step::Replace {
                        inputs,
                        edge,
                        valid: self.draft_valid(),
                    }
                } else if state.editor.revision != was.revision {
                    model::Step::Edit {
                        inputs,
                        valid: self.draft_valid(),
                    }
                } else {
                    return None;
                }
            }
        })
    }

    /// The runtime in the model's terms.
    fn projection(&self) -> Projection {
        let state = &self.state;
        Projection {
            active: state.runtime.active().map(|active| active.bundle.token),
            uploading: state
                .coordinator
                .uploading
                .as_ref()
                .map(|upload| (upload.token, upload.generation as u8)),
            source_commit: state
                .coordinator
                .source_commit
                .as_ref()
                .map(|commit| commit.token),
            packed: state
                .coordinator
                .gpu_upload_preparation
                .as_ref()
                .map(|pack| pack.token),
            pack_back: state
                .coordinator
                .gpu_upload_preparation
                .as_ref()
                .is_some_and(|pack| pack.result.is_some()),
            ready: state.runtime.ready().map(|ready| ready.bundle.token),
            // On the frame or on the worker; a job the worker holds that a
            // newer request replaced is no longer the runtime's.
            preparing: state.runtime.preparing_timing().is_some()
                || state.background_preparation.as_ref().is_some_and(|worker| {
                    worker.token.is_some() && worker.token == state.runtime.requested()
                }),
            last_error: state.runtime.last_error().map(|error| error.token),
            request_stands: state
                .coordinator
                .requested_inputs
                .as_ref()
                .is_some_and(|inputs| inputs.read(&state.editor.document)),
            reset: state.coordinator.reset_requested,
            remesh: state.coordinator.remesh_requested,
            drop: state.coordinator.drop_requested,
            pulses: state.coordinator.pending_pulses.len(),
            switches: state.coordinator.pending_switches.len(),
            fault: state.coordinator.solver_fault.is_some(),
            // Once sent, its event is the device's, and the protocol has
            // done with it.
            cleanup: state.flux_cleanup.as_ref().and_then(|cleanup| {
                match cleanup.stage {
                    FluxCleanupStage::Due | FluxCleanupStage::Solving => Some(false),
                    FluxCleanupStage::Found(_) => Some(true),
                    FluxCleanupStage::Sent(_) | FluxCleanupStage::Landed(_) => None,
                }
                .map(|found| (cleanup.generation as u8, found))
            }),
            running: state.wave_running,
            stepping: state.wave_step,
            generation: self.request.generation() as u8,
            installed: self.request.running_node_count().is_some(),
            carries: self
                .running
                .filter(|_| self.request.running_node_count().is_some())
                .map(|(_, token)| token),
            ready_status: self.request.ready(),
            failed: self.request.failed(),
            behind: !self.request.caught_up(),
            handoff: self.request.handoff_outcome() == CanonicalGpuHandoffOutcome::Pending,
            refused: matches!(
                self.request.handoff_outcome(),
                CanonicalGpuHandoffOutcome::Rejected(_)
            ),
            event: self.request.live_event_pending(),
            display: self.display.generation as u8,
        }
    }

    /// The model's state in the same terms.
    fn modelled(&self) -> Projection {
        let model = &self.model;
        Projection {
            active: model.active.map(|token| self.real(token)),
            uploading: model
                .uploading
                .map(|upload| (self.real(upload.token), upload.generation)),
            source_commit: model.source_commit.map(|token| self.real(token)),
            packed: model.packed.map(|token| self.real(token)),
            pack_back: model.pack_back,
            ready: model.ready.map(|token| self.real(token)),
            preparing: model.preparing.is_some(),
            last_error: model.last_error.map(|token| self.real(token)),
            request_stands: model.requested_inputs == Some(model.inputs),
            reset: model.reset,
            remesh: model.remesh,
            drop: model.drop,
            pulses: model.pulses as usize,
            switches: model.switches as usize,
            fault: model.fault,
            cleanup: model
                .cleanup
                .map(|cleanup| (cleanup.generation, cleanup.found)),
            running: model.running,
            stepping: model.stepping,
            generation: model.generation,
            installed: model.device.is_some(),
            carries: model.device.map(|device| self.real(device.token)),
            ready_status: model
                .device
                .is_some_and(|device| device.ready && !device.failed),
            failed: model.failed(),
            behind: model
                .device
                .is_some_and(|device| !device.caught_up && !device.failed),
            handoff: model.handoff.is_some(),
            refused: model.refused,
            event: model.event.is_some(),
            display: model.display,
        }
    }

    /// Steps the model as the runtime stepped and holds the two to one
    /// state: where they part, either the runtime left the protocol the
    /// proofs are about or the model does not describe it.
    fn conform(&mut self, act: &Act, was: Was) {
        let log = self.state.protocol_log.take();
        let next = self.model.next;
        let told = std::mem::take(&mut self.told);
        let own = self.model_step(act, &was, &log);
        for step in told.into_iter().chain(own) {
            self.model.step(step);
            self.model.check();
        }
        // A frame may request twice: an edit's candidate committed in place,
        // and then adaptation's.
        let requested = log
            .iter()
            .filter_map(|note| match *note {
                ProtocolNote::Requested(real) | ProtocolNote::Adapted(real) => Some(real),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            usize::from(self.model.next - next),
            requested.len(),
            "after {act:?} the runtime requested {requested:?} and the model {} times",
            self.model.next - next
        );
        self.tokens.extend(requested);
        assert_eq!(
            self.projection(),
            self.modelled(),
            "after {act:?} the runtime and the protocol model part"
        );
    }

    fn before(&self) -> Before {
        Before {
            generation: self.request.generation(),
            active: self
                .state
                .runtime
                .active()
                .map(|active| active.bundle.token),
            uploading: self
                .state
                .coordinator
                .uploading
                .as_ref()
                .map(|upload| (upload.token, upload.generation)),
            reset: self.state.coordinator.reset_requested,
            drop: self.state.coordinator.drop_requested,
        }
    }

    /// One host frame as the app's: validation's share, then the runtime,
    /// and in an adaptive session the display and adaptation; the commands
    /// the frame gave are applied, and the device takes the frame's copies.
    /// A played worker works its share beside the frame, before the host
    /// reads it.
    fn frame(&mut self, grant: Option<usize>) {
        let Session {
            state,
            world,
            request,
            display,
            recorders,
            vector,
            adaptive,
            wave,
            worker,
            ..
        } = self;
        match worker {
            Some(worker) => worker.work(grant.unwrap_or(usize::MAX)),
            None => state.preparation_grant = Some(grant.unwrap_or(usize::MAX)),
        }
        state.amr_grant = Some(grant.unwrap_or(usize::MAX));
        state.editor.validate_frame(12000);
        let mut queue = world.resource_scope(|world, mut assets: Mut<Assets<ShaderBuffer>>| {
            let mut queue = CommandQueue::default();
            let mut commands = Commands::new(&mut queue, world);
            state.refresh_runtime(
                request,
                display,
                recorders,
                vector,
                &mut assets,
                &mut commands,
                1.0 / 60.0,
            );
            queue
        });
        queue.apply(world);
        if *adaptive {
            state.display_fell_back |= refresh_canonical_wave_display(
                state.runtime.active(),
                display,
                request,
                state.display_material_runtime.as_ref(),
                wave,
            );
        }
        state.refresh_flux_cleanup(request, display);
        if *adaptive {
            state.refresh_amr(request, display, wave);
        }
        self.in_world(|world, device| device.take(world));
    }

    /// Hands the session a finished estimate of the running generation, as a
    /// full readback of it would have led to: a field struck by a narrow
    /// pulse at `centre` and stepped twice on the CPU, estimated by the
    /// production job. The device serves no full snapshot here, as the host
    /// asks for one by wall time, so the session never estimates on its own.
    fn estimate_struck(&mut self, centre: Point2) {
        let state = &mut self.state;
        let Some(active) = state.runtime.active().cloned() else {
            return;
        };
        // The step the generation runs at its speed, which its drives hold
        // stable; what the host last uploaded may be another generation's.
        let dt = paced_time_step(
            active.recommended_time_step(),
            state.editor.document.presentation.simulation_speed,
        );
        let increment = active
            .canonical_operator
            .node_points()
            .iter()
            .map(|point| {
                let offset = *point - centre;
                0.1 * (-offset.dot(offset) / 0.01).exp()
            })
            .collect::<Vec<_>>();
        let forcing = &active.canonical_forcing;
        let snapshots = match &active.canonical_temporal_operator {
            Some(temporal) => (|| {
                let mut run = CanonicalTemporalWaveState::zero(temporal, dt).ok()?;
                run.apply_primary_pulse(temporal, forcing, &increment)
                    .ok()?;
                run.step_with_forcing(temporal, forcing).ok()?;
                let previous = run.clone();
                run.step_with_forcing(temporal, forcing).ok()?;
                IndicatorSnapshots::of_temporal_run(&active, temporal, &run, &previous).ok()
            })(),
            None => (|| {
                let operator = &active.canonical_operator;
                let mut run = CanonicalWaveState::zero(operator, dt).ok()?;
                run.apply_primary_pulse(operator, forcing, &increment)
                    .ok()?;
                run.step_with_forcing(operator, forcing).ok()?;
                let previous = run.clone();
                run.step_with_forcing(operator, forcing).ok()?;
                IndicatorSnapshots::of_fixed_run(&active, &run, &previous).ok()
            })(),
        };
        let Some(snapshots) = snapshots else {
            return;
        };
        let mut job = state
            .amr
            .estimate(&active, snapshots, &state.editor.document.presentation);
        let result = loop {
            if let Some(result) = job.advance(1 << 16) {
                break result;
            }
        };
        state.amr_indicator_completed = Some(result);
        state.amr_indicator_source = Some(AmrIndicatorSource {
            topology: active.bundle.token,
            gpu_generation: self.request.generation(),
            accepted_step: self.wave.snapshot_completed_steps,
        });
    }

    /// The solve for the stationary flux the last handoff left answers, as
    /// a full readback of its generation would have let it: a part found to
    /// take away, or none. The device serves no full snapshot here, so the
    /// solve never starts by itself.
    fn solve_cleanup(&mut self, found: bool) {
        let state = &mut self.state;
        let (Some(cleanup), Some(active)) = (&mut state.flux_cleanup, state.runtime.active())
        else {
            return;
        };
        if !matches!(
            cleanup.stage,
            FluxCleanupStage::Due | FluxCleanupStage::Solving
        ) {
            return;
        }
        let samples = active.canonical_operator.complementary_degrees_of_freedom();
        cleanup.stage = FluxCleanupStage::Solving;
        state.flux_cleanup_job = None;
        state.flux_cleanup_completed = Some(Ok(CanonicalStationaryFlux {
            stationary: vec![Point2::new(1.0e-3, 0.0); samples],
            energy: if found { 1.0e-3 } else { 0.0 },
            flux_energy: 1.0,
            iterations: 0,
        }));
    }

    fn act(&mut self, act: &Act) {
        let domain = self.state.editor.document.model.draft.geometry.domain;
        match act {
            Act::Frame(grant) => self.frame(*grant),
            Act::Readbacks(steps) => self.answer(|device, world| device.advance(world, *steps)),
            // The device admits a handoff by its handles, not by the host's
            // outcome: guarded on the outcome, a withdrawal that set it and
            // left the handles behind was never offered for admission, and
            // "nothing was admitted" could not fail (VER-44 of the 2026-10-20
            // review).
            Act::Handoff(admit) => self.answer(|device, world| device.admit(world, *admit)),
            Act::Event(take) => self.answer(|device, world| device.process(world, *take)),
            Act::Fault => self.answer(|device, world| device.latch(world)),
            Act::Deliver(nth) => {
                self.arrive(|world, device| device.deliver_oldest(world, *nth as usize))
            }
            Act::Admit(admit) => self.in_world(|world, device| device.admit(world, *admit)),
            Act::Process(take) => self.in_world(|world, device| device.process(world, *take)),
            Act::Latch => self.in_world(|world, device| device.latch(world)),
            Act::Work(slices) => {
                if let Some(worker) = &mut self.worker {
                    worker.work(slices.unwrap_or(usize::MAX));
                }
            }
            Act::Pack(all) => {
                if let Some(held) = &mut self.state.held_packs {
                    let packs = if *all { held.len() } else { held.len().min(1) };
                    for pack in held.drain(..packs) {
                        pack.release();
                    }
                }
            }
            Act::Clean(found) => self.solve_cleanup(*found),
            Act::Validate => settle(&mut self.state.editor),
            Act::Material { nth, kind } => {
                let materials = &self.state.editor.document.model.draft.materials;
                let mut material = materials[*nth as usize % materials.len()].clone();
                let preset = |name: &str| {
                    law_presets()
                        .iter()
                        .find(|preset| preset.name == name && preset.row == LawPresetRow::Mass)
                        .unwrap()
                };
                let edited = match kind % 4 {
                    0 => {
                        let dense = material.mass_density == ScalarField::constant(1.0);
                        material.mass_density =
                            ScalarField::constant(if dense { 1.5 } else { 1.0 });
                        Ok(material)
                    }
                    1 => apply_law_preset(preset("Switchable medium"), &material),
                    2 => apply_law_preset(preset("Parametric pump"), &material),
                    _ => {
                        material.mass_law = CoefficientLaw::linear();
                        material.stiffness_law = CoefficientLaw::linear();
                        Ok(material)
                    }
                };
                if let Ok(edited) = edited {
                    let _ = self.state.editor.update_material(edited);
                }
            }
            Act::Nudge(nth) => {
                let draft = &self.state.editor.document.model.draft;
                let Some(curve) = draft.geometry.curves.first() else {
                    return;
                };
                let controls = match &curve.spline {
                    CurveSpline::Closed(spline) => spline.controls(),
                    CurveSpline::Open(spline) => spline.controls(),
                };
                let (dx, dy) =
                    [(0.05, 0.0), (-0.05, 0.03), (0.4, 0.0), (0.0, 0.9)][*nth as usize % 4];
                let point = Point2::new(controls[0].x + dx, controls[0].y + dy);
                let handle = TopologyHandle::Control {
                    curve: curve.id,
                    control: 0,
                };
                if let Ok(update) = plan_handle_drag(&draft.geometry, handle, point) {
                    let _ = self.state.editor.apply_transform_updates(&[update]);
                }
            }
            Act::Edge(nth) => {
                self.state.editor.document.presentation.mesh_edge =
                    [0.2, 0.25, 0.3][*nth as usize % 3];
            }
            Act::Speed(nth) => {
                self.state.editor.document.presentation.simulation_speed =
                    [0.25, 1.0, 2.0][*nth as usize % 3];
            }
            Act::Source { cell, on, high } => {
                let mut source = self.state.editor.document.model.source;
                source.position = cell_point(domain, *cell);
                source.enabled = *on;
                source.signal = TimeSignal::harmonic(0.0, 1.0, if *high { 3.5 } else { 2.5 }, 0.0);
                let _ = self.state.editor.set_point_source(source);
            }
            Act::Reset => self.state.coordinator.reset_requested = true,
            Act::Remesh => self.state.coordinator.remesh_requested = true,
            Act::Run(on) => self.state.wave_running = *on,
            Act::Step => self.state.wave_step = true,
            Act::Pulse(cell) => self.state.place_pulse(cell_point(domain, *cell)),
            Act::Switch => self.state.request_material_switch(),
            Act::Open(other) => {
                let _ = self
                    .state
                    .set_document(scene(*other, self.adaptive), true, true);
            }
            Act::Undo => self.state.undo(),
            Act::Redo => self.state.redo(),
            Act::Adapt(cell) => self.estimate_struck(cell_point(domain, *cell)),
        }
    }

    /// Follows what the step did to the device and to the host's active
    /// topology, and holds each change to what may cause it.
    fn observe(&mut self, act: &Act, before: Before) {
        let generation = self.request.generation();
        let active = self
            .state
            .runtime
            .active()
            .map(|active| active.bundle.token);
        if self.request.running_node_count().is_none() {
            self.running = None;
        } else if generation != before.generation {
            match act {
                // Published as its receipt arrives, in whichever step brings
                // it.
                _ if !matches!(act, Act::Frame(_))
                    && self.request.handoff_outcome() == CanonicalGpuHandoffOutcome::Accepted =>
                {
                    let (token, awaited) = before
                        .uploading
                        .expect("the device published a handoff no upload waits for");
                    assert_eq!(
                        awaited, generation,
                        "a handoff published as another generation than the host awaits"
                    );
                    self.running = Some((generation, token));
                }
                Act::Frame(_) => {
                    if let Some(upload) = self
                        .state
                        .coordinator
                        .uploading
                        .as_ref()
                        .filter(|upload| upload.generation == generation)
                    {
                        self.running = Some((generation, upload.token));
                    } else if before.reset && !self.state.coordinator.reset_requested {
                        let token = active.expect("Reset installed a generation with none active");
                        self.running = Some((generation, token));
                        assert!(
                            self.state.coordinator.pending_pulses.is_empty()
                                && self.state.coordinator.pending_switches.is_empty()
                                && !self.request.live_event_pending(),
                            "Reset left a pulse or a Switch for the run it cleared"
                        );
                    } else {
                        panic!(
                            "the device installed generation {generation} that no upload or Reset accounts for"
                        );
                    }
                }
                _ => panic!("{act:?} moved the device to generation {generation}"),
            }
        } else if let Some(token) = active.filter(|token| Some(*token) != before.active) {
            if before
                .uploading
                .is_some_and(|(uploaded, _)| uploaded == token)
            {
                assert_eq!(
                    self.running.map(|(_, running)| running),
                    Some(token),
                    "the host published an upload the device does not run"
                );
            } else {
                let update = self.state.runtime.active().unwrap().solver_update;
                assert!(
                    matches!(
                        update,
                        PreparedSolverUpdate::MeasurementsOnly
                            | PreparedSolverUpdate::SourceWeightsOnly
                            | PreparedSolverUpdate::SourceDrivesOnly
                    ),
                    "a {update:?} candidate was committed with no new generation"
                );
                if let Some((running, _)) = self.running {
                    self.running = Some((running, token));
                }
            }
        }
        if before.drop && matches!(act, Act::Frame(_)) {
            assert!(
                self.state.coordinator.pending_pulses.is_empty()
                    && self.state.coordinator.pending_switches.is_empty()
                    && self.state.coordinator.source_commit.is_none(),
                "work for the replaced scene survived its drop"
            );
        }
        let accepted = &self.state.editor.document.model.accepted;
        if self.state.coordinator.drop_requested && !before.drop {
            self.scenes = vec![accepted.clone()];
        } else if !self.scenes.contains(accepted) {
            self.scenes.push(accepted.clone());
        }
        if self.state.editor.acceptance == TopologyAcceptance::Valid {
            self.accepted_revision = self.state.editor.revision;
        }
    }

    fn step(&mut self, act: &Act) {
        let before_adaptation = self
            .state
            .runtime
            .active()
            .map(|active| active.bundle.token);
        let before = self.before();
        let was = self.was();
        self.state.notices.clear();
        self.state.protocol_log.take();
        self.act(act);
        self.observe(act, before);
        self.check();
        self.check_reads(act);
        if self.adaptive {
            self.observe_adaptation(act, &before_adaptation);
        }
        self.conform(act, was);
    }

    /// Follows adaptation's requests, and holds each adapted publication to
    /// the topology it was requested over: an adaptation publishes over the
    /// generation it was adapted from, never over one that replaced it.
    /// `before` was active before the step.
    fn observe_adaptation(&mut self, act: &Act, before: &Option<TopologyToken>) {
        let runtime = &self.state.runtime;
        let active = runtime.active();
        if runtime.requested_adaptation()
            && let Some(requested) = runtime.requested()
            && !self
                .adapted_from
                .iter()
                .any(|(token, _)| *token == requested)
        {
            self.adapted_from
                .push((requested, active.map(|active| active.bundle.token)));
        }
        if let Some(active) = active.filter(|active| Some(active.bundle.token) != *before)
            && active.adapted
        {
            let over = self
                .adapted_from
                .iter()
                .find(|(token, _)| *token == active.bundle.token)
                .map(|(_, over)| *over)
                .unwrap_or_else(|| {
                    panic!("after {act:?} an adapted mesh no request made published")
                });
            assert_eq!(
                over, *before,
                "after {act:?} an adapted mesh published over another topology than it was adapted from"
            );
        }
    }

    /// RT-03: a copy arriving late tells the host nothing older than it
    /// knows and nothing the device has not done. The display never goes back
    /// a generation, nor its clock within one; the running generation's steps
    /// and processed events never go back and never pass the device's; and a
    /// failure the host reads is one the device latched in that generation.
    fn check_reads(&mut self, act: &Act) {
        let display = self.display.generation;
        assert!(
            display >= self.read.display,
            "after {act:?} the display went back to generation {display} from {}",
            self.read.display
        );
        self.read.display = display;
        if let Some(clock) = self.display.clock {
            if let Some((generation, steps)) = self.read.clock
                && generation == display
            {
                assert!(
                    clock.accepted_steps >= steps,
                    "after {act:?} the display's clock went back from step {steps} to {}",
                    clock.accepted_steps
                );
            }
            self.read.clock = Some((display, clock.accepted_steps));
        }
        // A cleared generation's counters are fresh, and stand for none.
        if self.request.running_node_count().is_none() {
            self.read.counted = None;
            return;
        }
        let stats = self.request.stats();
        let counted = (
            self.request.generation(),
            stats.completed_steps(),
            stats.processed_event(),
        );
        if let Some((generation, completed, processed)) = self.read.counted
            && generation == counted.0
        {
            assert!(
                counted.1 >= completed,
                "after {act:?} the host's count went back from {completed} steps to {}",
                counted.1
            );
            assert!(
                counted.2 >= processed,
                "after {act:?} the host's processed event went back from {processed} to {}",
                counted.2
            );
        }
        if let Some((completed, processed)) = self.device.done(&self.request) {
            assert!(
                counted.1 <= completed && counted.2 <= processed,
                "after {act:?} the host reads {} steps and event {} where the device has done \
                 {completed} and {processed}",
                counted.1,
                counted.2
            );
        }
        assert!(
            !self.request.failed() || self.device.failed(&self.request),
            "after {act:?} the host reads a failure the running generation never latched"
        );
        self.read.counted = Some(counted);
    }

    fn check(&self) {
        let state = &self.state;
        // A replacement waits for its frame to drop what ran.
        if state.coordinator.drop_requested {
            return;
        }
        if let (Some((generation, token)), Some(active)) = (self.running, state.runtime.active())
            && state.coordinator.uploading.is_none()
            && self.request.generation() == generation
        {
            assert_eq!(
                active.bundle.token, token,
                "the host's active topology is not the one the device runs"
            );
            assert_eq!(
                self.request.running_node_count(),
                Some(active.canonical_operator.degrees_of_freedom()),
                "the device runs another mesh than the host's"
            );
            assert_eq!(
                self.request.running_time_step(),
                Some(state.coordinator.uploaded_time_step),
                "the host paces a step the device does not run"
            );
        }
        for prepared in state
            .runtime
            .active()
            .map(|active| &active.bundle)
            .into_iter()
            .chain(state.runtime.ready().map(|ready| &ready.bundle))
        {
            assert!(
                self.scenes.contains(&prepared.authored),
                "a topology was prepared from a scene this one never held"
            );
        }
    }

    /// Why the runtime is not at rest, if it is not: running the accepted
    /// revision at the speed's step on a ready device, or with an error
    /// naming that revision, and nothing waiting.
    fn unsettled(&self) -> Option<String> {
        let state = &self.state;
        let waits = [
            ("a Reset", state.coordinator.reset_requested),
            ("a drop", state.coordinator.drop_requested),
            ("an upload", state.coordinator.uploading.is_some()),
            ("a pack", state.coordinator.gpu_upload_preparation.is_some()),
            (
                "a handoff's stationary flux",
                state.flux_cleanup.as_ref().is_some_and(|cleanup| {
                    !matches!(
                        cleanup.stage,
                        FluxCleanupStage::Sent(_) | FluxCleanupStage::Landed(_)
                    )
                }),
            ),
            ("a source commit", state.coordinator.source_commit.is_some()),
            ("a preparation", state.preparation_in_progress()),
            ("a candidate", state.runtime.ready().is_some()),
            ("a pulse", !state.coordinator.pending_pulses.is_empty()),
            ("a Switch", !state.coordinator.pending_switches.is_empty()),
            ("a live event", self.request.live_event_pending()),
            (
                "a handoff",
                self.request.handoff_outcome() == CanonicalGpuHandoffOutcome::Pending,
            ),
            (
                "validation",
                state.editor.acceptance == TopologyAcceptance::Pending,
            ),
        ];
        if let Some((what, _)) = waits.iter().find(|(_, waiting)| *waiting) {
            return Some(format!("{what} still waits"));
        }
        // A Remesh pressed, a mesh edge set or an edit made while an upload
        // was in flight is taken up by the next frame.
        if state.request_is_due() {
            return Some("a request is due".into());
        }
        // A pause holds adaptation where it is, by design.
        let adapting = self.adaptive
            && state.wave_running
            && state.editor.document.presentation.adaptation.enabled;
        if adapting && state.adaptation_in_flight() {
            return Some("an adaptation still waits".into());
        }
        if adapting && (state.estimate_in_flight() || state.amr_indicator_completed.is_some()) {
            return Some("an estimate still waits".into());
        }
        // What adaptation remembers is of the mesh that runs, error or not.
        if self.adaptive
            && let Some(active) = state.runtime.active()
        {
            let mesh = active.mesh.mesh_revision;
            if let Some(history) = state.amr.state.as_ref()
                && history.mesh_revision != mesh
            {
                return Some(format!(
                    "adaptation's history is of mesh {} where {mesh} runs",
                    history.mesh_revision
                ));
            }
            if let Some(pending) = state.amr.pending.as_ref() {
                return Some(format!(
                    "the history of adapted mesh {} waits for a handoff nothing will make",
                    pending.mesh_revision
                ));
            }
            if let Some(error) = &state.amr_error {
                return Some(format!("adaptation reports {error:?}"));
            }
        }

        // An invalid draft leaves the accepted scene alone, and the runtime
        // comes to rest on it and on what lies outside the draft as they
        // stand; a valid one, on its own revision. Either way an error naming
        // the request that read them will do instead.
        let held = state.editor.acceptance != TopologyAcceptance::Valid;
        let refused = if held {
            state
                .coordinator
                .requested_inputs
                .as_ref()
                .is_some_and(|inputs| inputs.read(&state.editor.document))
                && state.runtime.last_error().is_some_and(|error| {
                    Some(error.token.document_revision) == state.coordinator.requested_revision
                })
        } else {
            state
                .runtime
                .last_error()
                .is_some_and(|error| error.token.document_revision == self.accepted_revision)
        };
        if refused {
            return None;
        }
        let Some(active) = state.runtime.active() else {
            return Some("nothing runs, and no error says why".into());
        };
        if held {
            if !runs_document(active, &state.editor.document) {
                return Some(
                    "the run is not the accepted scene with the source, probes and far field \
                     as they stand, and no error says why"
                        .into(),
                );
            }
        } else if active.bundle.token.document_revision != self.accepted_revision {
            return Some(format!(
                "revision {} runs where {} was accepted, and no error says why",
                active.bundle.token.document_revision, self.accepted_revision
            ));
        }
        if self.running.map(|(_, token)| token) != Some(active.bundle.token) {
            return Some("the device runs another topology than the host's".into());
        }
        if !self.request.ready() {
            return Some("the device is not ready".into());
        }
        let wanted = paced_time_step(
            active.recommended_time_step(),
            state.editor.document.presentation.simulation_speed,
        );
        if (wanted / state.coordinator.uploaded_time_step - 1.0).abs() > TIME_STEP_HYSTERESIS {
            return Some(format!(
                "the step {:e} is not the speed's {wanted:e}",
                state.coordinator.uploaded_time_step
            ));
        }
        None
    }

    /// Preparation and the device fair from here: each round validates, lends
    /// preparation all it needs, makes every pack held back, finds the
    /// stationary flux a handoff left, completes every step asked for, takes
    /// whatever the device is offered, and presses Run on a failed run, as
    /// its message asks.
    fn quiesce(&mut self) {
        let round = [
            Act::Validate,
            Act::Pack(true),
            Act::Clean(true),
            Act::Frame(None),
            Act::Pack(true),
            Act::Frame(None),
            Act::Readbacks(u64::MAX),
            Act::Handoff(true),
            Act::Event(true),
            Act::Frame(None),
        ];
        for _ in 0..32 {
            if self.request.failed() {
                self.step(&Act::Run(true));
            }
            for act in &round {
                self.step(act);
            }
            if self.unsettled().is_none() {
                return;
            }
        }
        panic!(
            "the runtime did not come to rest: {}",
            self.unsettled().unwrap()
        );
    }
}

/// The reference state: nothing, since every step is legal in every state.
struct Generations;

impl ReferenceStateMachine for Generations {
    type State = ();
    type Transition = Act;

    fn init_state() -> BoxedStrategy<()> {
        Just(()).boxed()
    }

    fn transitions(_: &()) -> BoxedStrategy<Act> {
        acts()
    }

    fn apply(state: (), _: &Act) {
        state
    }
}

impl StateMachineTest for Generations {
    type SystemUnderTest = Session;
    type Reference = Self;

    fn init_test(_: &()) -> Session {
        Session::start()
    }

    fn apply(mut session: Session, _: &(), act: Act) -> Session {
        session.step(&act);
        session
    }

    fn teardown(mut session: Session, _: ()) {
        session.quiesce();
    }
}

prop_state_machine! {
    #![proptest_config(ProptestConfig {
        cases: 64,
        ..ProptestConfig::default()
    })]
    #[test]
    fn the_host_and_the_device_agree_on_what_runs(sequential 1..120 => Generations);
}

/// RT-03: the same interleavings with preparation on a worker the sequence
/// plays, whose result can come for a request a newer one has replaced.
struct Played;

impl StateMachineTest for Played {
    type SystemUnderTest = Session;
    type Reference = Generations;

    fn init_test(_: &()) -> Session {
        Session::played()
    }

    fn apply(mut session: Session, _: &(), act: Act) -> Session {
        session.step(&act);
        session
    }

    fn teardown(mut session: Session, _: ()) {
        session.quiesce();
    }
}

prop_state_machine! {
    #![proptest_config(ProptestConfig {
        cases: 64,
        ..ProptestConfig::default()
    })]
    #[test]
    fn late_preparations_agree_on_what_runs(sequential 1..120 => Played);
}

/// Found by the runtime sequences: a fault on a candidate just installed made
/// the host reject it and keep its old topology, which the install had already
/// replaced on the device, and the host paced the candidate's step for a
/// topology that no longer ran. The host publishes what the device runs now,
/// and the fault pauses it as any fault does; Run retries from there.
#[test]
fn a_fault_on_an_installed_candidate_publishes_what_the_device_runs() {
    let mut session = Session::start();
    session.quiesce();
    // An edit with Reset pressed starts afresh, so it installs.
    for act in [
        Act::Material { nth: 0, kind: 0 },
        Act::Reset,
        Act::Validate,
        Act::Frame(None),
        Act::Readbacks(u64::MAX),
        Act::Frame(None),
    ] {
        session.step(&act);
    }
    let upload = session
        .state
        .coordinator
        .uploading
        .as_ref()
        .expect("an install uploads");
    assert_eq!(upload.generation, session.request.generation(), "installed");
    let candidate = upload.token;
    session.step(&Act::Fault);
    session.step(&Act::Readbacks(0));
    session.step(&Act::Frame(Some(0)));
    assert!(session.state.coordinator.uploading.is_none());
    assert_eq!(
        session
            .state
            .runtime
            .active()
            .map(|active| active.bundle.token),
        Some(candidate),
        "the host runs what the device runs"
    );
    session.step(&Act::Frame(Some(0)));
    assert!(!session.state.wave_running, "the fault pauses the run");
    assert!(
        session
            .state
            .notices
            .iter()
            .any(|notice| notice.title == "Simulation paused")
    );
    session.step(&Act::Run(true));
    session.quiesce();
}

/// Found by the runtime sequences: a fault on the running generation while a
/// handoff waited made the host drop the candidate, but the handoff stayed on
/// the device and could still be admitted, which left the device on 1,519
/// nodes at a step the host had never paced while the host kept 1,579. The
/// host withdraws the handoff now, and the running generation stays, paused.
#[test]
fn a_fault_while_a_handoff_waits_withdraws_it() {
    let mut session = Session::start();
    session.quiesce();
    for act in [
        Act::Nudge(2),
        Act::Validate,
        Act::Frame(None),
        Act::Readbacks(u64::MAX),
        Act::Frame(None),
    ] {
        session.step(&act);
    }
    assert_eq!(
        session.request.handoff_outcome(),
        CanonicalGpuHandoffOutcome::Pending
    );
    let running = session.request.generation();
    let active = session.state.runtime.active().unwrap().bundle.token;
    session.step(&Act::Fault);
    session.step(&Act::Frame(Some(0)));
    assert!(session.state.coordinator.uploading.is_none());
    assert_ne!(
        session.request.handoff_outcome(),
        CanonicalGpuHandoffOutcome::Pending,
        "the handoff was withdrawn"
    );
    session.step(&Act::Handoff(true));
    assert_eq!(
        session.request.generation(),
        running,
        "nothing was admitted"
    );
    assert_eq!(session.state.runtime.active().unwrap().bundle.token, active);
    session.step(&Act::Frame(Some(0)));
    assert!(!session.state.wave_running, "the fault pauses the run");
    session.step(&Act::Run(true));
    session.quiesce();
}

/// RT-9 of the 2026-10-20 review: Reset while an edit's pack waited for the
/// device kept the pack, made against the run Reset discarded, and its
/// handoff carried that run's clock origin into the reset one, so every
/// drive, pulse, gate and Switch carried through it stood that far off.
/// Reset drops the pack, and the candidate is packed again against the reset
/// run.
#[test]
fn reset_drops_a_pack_made_against_the_run_it_clears() {
    let mut session = Session::start();
    session.quiesce();
    for act in [Act::Nudge(2), Act::Validate, Act::Frame(None)] {
        session.step(&act);
    }
    assert!(
        session.state.coordinator.gpu_upload_preparation.is_some(),
        "the pack waits for the device"
    );
    session.step(&Act::Reset);
    session.step(&Act::Frame(Some(0)));
    assert!(
        !session.state.coordinator.reset_requested,
        "Reset installed"
    );
    assert!(
        session.state.coordinator.gpu_upload_preparation.is_none(),
        "the pack made against the cleared run is dropped"
    );
    session.quiesce();
}

/// RT-10 of the 2026-10-20 review: the display cadence read a frame that
/// backpressure clamped to no step as a quiet one, though the queue behind
/// it was full; on an overloaded scene those frames filled its window with
/// the solver's slowness. A frame counts what it asked for.
#[test]
fn a_frame_backpressure_clamps_is_not_quiet() {
    let mut session = Session::start();
    session.quiesce();
    session.state.e2e_steps.batch = Some(40);
    // Nothing read back: the queue fills, and the third frame admits none.
    for _ in 0..3 {
        session.step(&Act::Frame(None));
    }
    assert_eq!(
        session.request.requested_steps() - session.request.stats().retired_steps(),
        MAX_STEPS_PER_FRAME,
        "the queue is full"
    );
    assert_eq!(session.state.display_cadence.pending_batch(), 40);
}

/// RT-11 of the 2026-10-20 review: a retimed candidate refused leaves the
/// step where it was while the speed still asks for another, and the speed
/// ceiling asked again: every frame prepared, packed and was refused, and
/// each error replaced the last before it could be read. A retime is asked
/// once from a step, until the speed is met.
#[test]
fn a_refused_retime_is_not_asked_again() {
    let mut session = Session::start();
    session.quiesce();
    let before = session.model.next;
    session.step(&Act::Speed(0));
    for _ in 0..4 {
        for act in [
            Act::Frame(None),
            Act::Readbacks(u64::MAX),
            Act::Frame(None),
            Act::Handoff(false),
            Act::Frame(None),
        ] {
            session.step(&act);
        }
    }
    assert_eq!(
        session.model.next,
        before + 1,
        "the refused retime is asked once"
    );
    // The speed met again, at 1x, and left again: the retime is asked anew.
    session.step(&Act::Speed(1));
    session.quiesce();
    session.step(&Act::Speed(0));
    session.quiesce();
    assert!(
        session.model.next > before + 1,
        "a speed met and left asks again"
    );
}

/// Found by the runtime sequences: a scene whose draft an edit made invalid
/// before its first validation never ran, since nothing was prepared while
/// the draft was invalid, and neither did one opened with an invalid draft.
/// Its accepted scene is prepared without the draft now.
#[test]
fn a_scene_invalid_before_it_first_ran_runs_its_accepted_scene() {
    let mut session = Session::start();
    session.step(&Act::Nudge(3));
    session.quiesce();
    assert!(matches!(
        session.state.editor.acceptance,
        TopologyAcceptance::Invalid(_)
    ));
    let active = session
        .state
        .runtime
        .active()
        .expect("the accepted scene runs");
    assert!(runs_document(active, &session.state.editor.document));
}

/// Found by the runtime sequences: the point source switched off while the
/// draft was invalid ran on, as nothing was prepared until the draft was valid
/// again. What lies outside the draft reaches the run now, and further edits
/// of the invalid draft, which change nothing a preparation reads, prepare
/// nothing.
#[test]
fn an_invalid_draft_holds_back_nothing_outside_it() {
    let mut session = Session::start();
    session.quiesce();
    session.step(&Act::Nudge(3));
    session.step(&Act::Validate);
    session.step(&Act::Source {
        cell: 5,
        on: false,
        high: false,
    });
    session.quiesce();
    let active = session.state.runtime.active().unwrap();
    assert!(!active.point_source.enabled, "the source went off");
    let running = active.bundle.token;
    session.step(&Act::Nudge(3));
    session.step(&Act::Validate);
    session.step(&Act::Frame(None));
    assert!(
        !session.state.preparation_in_progress() && session.state.coordinator.uploading.is_none(),
        "an edit of the invalid draft prepared the same scene again"
    );
    assert_eq!(
        session.state.runtime.active().unwrap().bundle.token,
        running
    );
}

/// Found with the protocol model beside the runtime: the device's counters
/// outlived a dropped generation, so after a fault the scene opened in its
/// place read that failure as its own. Opened while the fault's upload was
/// still in flight, the new scene started paused; opened after the pause, it
/// raised a second "Simulation paused" notice. Run could not clear either
/// until the new scene installed.
#[test]
fn a_scene_opened_after_a_fault_takes_none_of_it() {
    // A fault while an install uploads, the scene replaced before the host
    // has paused for it.
    let mut session = Session::start();
    for act in [Act::Validate, Act::Frame(None)] {
        session.step(&act);
    }
    assert!(
        session.state.coordinator.uploading.is_some(),
        "an install uploads"
    );
    for act in [Act::Fault, Act::Open(false), Act::Frame(Some(0))] {
        session.step(&act);
    }
    assert!(!session.request.failed());
    assert!(session.state.wave_running, "the new scene runs");
    assert!(
        session.state.notices.is_empty(),
        "{:?}",
        session.state.notices.len()
    );
    session.quiesce();

    // A fault the host has paused for, the scene replaced after.
    let mut session = Session::start();
    session.quiesce();
    for act in [Act::Fault, Act::Frame(Some(0))] {
        session.step(&act);
    }
    assert!(
        session.state.coordinator.solver_fault.is_some(),
        "paused for the fault"
    );
    for act in [Act::Open(true), Act::Frame(Some(0))] {
        session.step(&act);
    }
    assert!(!session.request.failed());
    assert!(
        session.state.notices.is_empty(),
        "a second notice of the dropped generation's fault"
    );
    session.quiesce();
}

/// The running generation with an edit's handoff pending on the device, and
/// no copy in flight.
fn handing_off(session: &mut Session) {
    session.quiesce();
    for act in [
        Act::Nudge(2),
        Act::Validate,
        Act::Frame(None),
        Act::Readbacks(u64::MAX),
        Act::Frame(None),
        Act::Readbacks(0),
    ] {
        session.step(&act);
    }
    assert_eq!(
        session.request.handoff_outcome(),
        CanonicalGpuHandoffOutcome::Pending
    );
}

/// RT-03: the source's copies taken after the device admitted a handoff, and
/// arriving after the receipt published it. The display already shows the
/// target, from the receipt; a copy of the source that took it back would
/// paint the old mesh's field over the new one's nodes.
#[test]
fn a_sources_copies_after_its_handoff_change_nothing() {
    let mut session = Session::start();
    handing_off(&mut session);
    let source = session.request.generation();
    for act in [Act::Admit(true), Act::Frame(Some(0)), Act::Frame(Some(0))] {
        session.step(&act);
    }
    // The first frame's copies bring the receipt, and the target publishes.
    session.step(&Act::Deliver(0));
    let target = session.request.generation();
    assert_ne!(target, source, "the receipt published the target");
    assert_eq!(session.display.generation, target);
    let clock = session.display.clock.map(|clock| clock.accepted_steps);
    // The second frame's are the source's, late.
    session.step(&Act::Deliver(0));
    assert_eq!(session.display.generation, target);
    assert_eq!(
        session.display.clock.map(|clock| clock.accepted_steps),
        clock
    );
    session.quiesce();
}

/// RT-03: a receipt the device wrote for a handoff the host then withdrew,
/// another scene opened in its place, arriving after the old scene was
/// dropped. It admits nothing: nothing runs until the new scene installs.
#[test]
fn a_receipt_of_a_withdrawn_handoff_admits_nothing() {
    let mut session = Session::start();
    handing_off(&mut session);
    let generation = session.request.generation();
    for act in [
        Act::Admit(true),
        Act::Frame(Some(0)),
        Act::Open(true),
        Act::Frame(Some(0)),
    ] {
        session.step(&act);
    }
    assert_eq!(
        session.request.handoff_outcome(),
        CanonicalGpuHandoffOutcome::None,
        "the replacement withdrew the handoff"
    );
    assert_eq!(
        session.request.running_node_count(),
        None,
        "and dropped the run"
    );
    session.step(&Act::Deliver(0));
    assert_eq!(session.request.generation(), generation);
    assert_eq!(session.request.running_node_count(), None);
    assert_eq!(
        session.request.handoff_outcome(),
        CanonicalGpuHandoffOutcome::None
    );
    session.quiesce();
}

/// RT-03: a status copy taken while the run stood failed, arriving after Run
/// cleared the failure. The cleared status buffer was replaced; the copy of
/// the old one fails nothing.
#[test]
fn a_status_from_before_a_cleared_failure_fails_nothing() {
    let mut session = Session::start();
    session.quiesce();
    for act in [Act::Fault, Act::Frame(Some(0))] {
        session.step(&act);
    }
    assert!(session.request.failed());
    assert!(
        session.state.coordinator.solver_fault.is_some(),
        "paused for the fault"
    );
    // A frame's copies of the failed status, still on their way.
    for act in [Act::Frame(Some(0)), Act::Run(true), Act::Frame(Some(0))] {
        session.step(&act);
    }
    assert!(!session.request.failed(), "Run cleared the failure");
    session.step(&Act::Deliver(0));
    assert!(!session.request.failed());
    session.quiesce();
}

/// RT-03: the worker finishes a preparation after an edit has asked for
/// another, and the host reads it in the frame that asked. It is dropped, and
/// the edit's preparation runs.
#[test]
fn a_preparation_finished_for_a_replaced_request_is_dropped() {
    let mut session = Session::played();
    session.quiesce();
    for act in [
        Act::Material { nth: 0, kind: 0 },
        Act::Validate,
        Act::Frame(Some(0)),
        Act::Work(None),
        Act::Material { nth: 1, kind: 0 },
        Act::Validate,
    ] {
        session.step(&act);
    }
    let edit = session.state.editor.revision;
    session.step(&Act::Frame(Some(0)));
    assert!(
        session.state.runtime.ready().is_none(),
        "the replaced request's candidate was taken"
    );
    assert_eq!(
        session
            .state
            .runtime
            .requested()
            .map(|token| token.document_revision),
        Some(edit)
    );
    session.quiesce();
    assert_eq!(
        session
            .state
            .runtime
            .active()
            .map(|active| active.bundle.token.document_revision),
        Some(edit)
    );
}

mod adapting;
