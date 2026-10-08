//! The runtime under generated interleavings of host frames, device
//! completions and user actions.
//!
//! The host prepares a candidate, packs it, uploads it and publishes it once
//! the device has taken it, while edits, Reset, scene replacements, pulses and
//! Switch presses arrive, and the device finishes things in an order of its
//! own. The defects of this area each needed one particular order: a waiting
//! pulse sent into the run Reset had just cleared, an upload waiting for a
//! generation that never came, a rejected handoff that kept the candidate's
//! step. Here the order is drawn. A frame lends preparation some work units,
//! the device delivers readbacks, settles a handoff or a live event either
//! way, or faults, and the user acts in between.
//!
//! The device is the request's test transitions (`deliver_readbacks`,
//! `accept_handoff`, `reject_handoff`, `process_live_event`,
//! `reject_live_event`, `fail`). Preparation advances by the units each frame
//! is granted and packing happens on the frame, so the order is the
//! sequence's alone.
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
//! compared field by field in the runtime's own tokens. Where they part,
//! either the runtime has left the protocol the proofs are about or the model
//! does not describe it; either way the proofs say nothing until it is
//! settled.

use super::*;
use crate::canonical_gpu::{CANONICAL_FAILURE_NON_FINITE, CanonicalGpuHandoffOutcome};
use bevy::ecs::world::CommandQueue;
use funfern_app::topology_editor::TopologyEditor;
use funfern_app::topology_runtime::{PreparedSolverUpdate, TopologyToken};
use funfern_protocol as model;

/// One step of the interleaving.
#[derive(Clone, Debug)]
enum Act {
    /// A host frame lending preparation that many slices; `None`, all it
    /// needs.
    Frame(Option<usize>),
    /// A frame's readbacks from the device, with up to that many more of the
    /// requested steps completed.
    Readbacks(u64),
    /// The device settles a pending handoff, admitting the target or not.
    Handoff(bool),
    /// The device settles a pending live event, taking it or refusing it.
    Event(bool),
    /// The run fails at its last accepted step.
    Fault,
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
        3 => Just(Act::Validate),
        3 => (any::<u8>(), any::<u8>()).prop_map(|(nth, kind)| Act::Material { nth, kind }),
        2 => any::<u8>().prop_map(Act::Nudge),
        1 => any::<u8>().prop_map(Act::Edge),
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

/// A small scene to run: the unit square on a coarse mesh, a subdomain of a
/// switchable material, the point source on, adaptation off. `other` moves
/// the subdomain and coarsens the mesh, for a replacement that differs.
fn fixture(other: bool) -> TopologyDocument {
    let mut editor = TopologyEditor::default();
    let switchable = editor.add_material().unwrap();
    let material = editor
        .document
        .model
        .draft
        .material(switchable)
        .unwrap()
        .clone();
    let switch = law_presets()
        .iter()
        .find(|preset| preset.name == "Switchable medium" && preset.row == LawPresetRow::Mass)
        .unwrap();
    editor
        .update_material(apply_law_preset(switch, &material).unwrap())
        .unwrap();
    let centre = if other {
        Point2::new(0.35, -0.25)
    } else {
        Point2::new(-0.4, 0.1)
    };
    editor
        .create_closed_curve(
            PeriodicCubicSpline::rounded(centre, 0.25),
            ClosedCurvePurpose::Subdomain {
                material: switchable,
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
    ready: Option<TopologyToken>,
    preparing: bool,
    last_error: Option<TopologyToken>,
    request_stands: bool,
    reset: bool,
    drop: bool,
    pulses: usize,
    switches: usize,
    fault: bool,
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
        let mut state = Playground {
            background_preparation: None,
            background_amr: None,
            preparation_grant: Some(usize::MAX),
            pack_inline: true,
            ..Playground::default()
        };
        state.set_document(fixture(false), false, true).unwrap();
        let mut world = World::new();
        world.init_resource::<Assets<ShaderBuffer>>();
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
        }
        .named()
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
                let route = log.iter().rev().find_map(|note| match *note {
                    ProtocolNote::InPlace => Some(model::Route::InPlace),
                    ProtocolNote::Patch { built, packable } => {
                        Some(model::Route::Patch { built, packable })
                    }
                    ProtocolNote::Pack => Some(model::Route::Pack),
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
                    route: route.unwrap_or(model::Route::Pack),
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
                })
            }
            Act::Readbacks(_) => model::Step::Readbacks {
                caught_up: self.request.caught_up(),
            },
            Act::Handoff(admit) => model::Step::Handoff { admit: *admit },
            Act::Event(take) => model::Step::Settle { take: *take },
            Act::Fault => model::Step::Fault,
            Act::Validate => model::Step::Validate { inputs },
            Act::Reset => model::Step::Reset,
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
            // The speed reaches the model through the frames that retime.
            Act::Speed(_) => return None,
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
            ready: state.runtime.ready().map(|ready| ready.bundle.token),
            preparing: state.runtime.preparing_timing().is_some(),
            last_error: state.runtime.last_error().map(|error| error.token),
            request_stands: state
                .coordinator
                .requested_inputs
                .as_ref()
                .is_some_and(|inputs| inputs.read(&state.editor.document)),
            reset: state.coordinator.reset_requested,
            drop: state.coordinator.drop_requested,
            pulses: state.coordinator.pending_pulses.len(),
            switches: state.coordinator.pending_switches.len(),
            fault: state.coordinator.solver_fault.is_some(),
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
            ready: model.ready.map(|token| self.real(token)),
            preparing: model.preparing.is_some(),
            last_error: model.last_error.map(|token| self.real(token)),
            request_stands: model.requested_inputs == Some(model.inputs),
            reset: model.reset,
            drop: model.drop,
            pulses: model.pulses as usize,
            switches: model.switches as usize,
            fault: model.fault,
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
        let log = std::mem::take(&mut self.state.protocol_log);
        let requested = self.model.requested;
        if let Some(step) = self.model_step(act, &was, &log) {
            self.model.step(step);
            self.model.check();
        }
        for note in &log {
            if let ProtocolNote::Requested(real) = *note {
                let token = self
                    .model
                    .requested
                    .filter(|token| Some(*token) != requested)
                    .unwrap_or_else(|| {
                        panic!("the runtime requested {real:?} and the model did not")
                    });
                assert_eq!(
                    token as usize,
                    self.tokens.len(),
                    "the model's tokens are in order"
                );
                self.tokens.push(real);
            }
        }
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

    /// One host frame as the app's: validation's share, then the runtime.
    fn frame(&mut self, grant: Option<usize>) {
        let Session {
            state,
            world,
            request,
            display,
            recorders,
            vector,
            ..
        } = self;
        state.preparation_grant = Some(grant.unwrap_or(usize::MAX));
        state.editor.validate_frame(12000);
        world.resource_scope(|world, mut assets: Mut<Assets<ShaderBuffer>>| {
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
        });
    }

    fn act(&mut self, act: &Act) {
        let domain = self.state.editor.document.model.draft.geometry.domain;
        match act {
            Act::Frame(grant) => self.frame(*grant),
            Act::Readbacks(steps) => self.request.deliver_readbacks(&mut self.display, *steps),
            Act::Handoff(accept) => {
                if self.request.handoff_outcome() != CanonicalGpuHandoffOutcome::Pending {
                    return;
                }
                if *accept {
                    let Session {
                        world,
                        request,
                        display,
                        ..
                    } = self;
                    world.resource_scope(|_, mut assets: Mut<Assets<ShaderBuffer>>| {
                        request.accept_handoff(&mut assets, display);
                    });
                } else {
                    self.request.reject_handoff(CANONICAL_FAILURE_NON_FINITE);
                }
            }
            Act::Event(accept) => {
                let Session { world, request, .. } = self;
                world.resource_scope(|_, mut assets: Mut<Assets<ShaderBuffer>>| {
                    if *accept {
                        request.process_live_event(&mut assets);
                    } else {
                        request.reject_live_event(&mut assets, CANONICAL_FAILURE_NON_FINITE);
                    }
                });
            }
            Act::Fault => self.request.fail(CANONICAL_FAILURE_NON_FINITE),
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
            Act::Run(on) => self.state.wave_running = *on,
            Act::Step => self.state.wave_step = true,
            Act::Pulse(cell) => self.state.place_pulse(cell_point(domain, *cell)),
            Act::Switch => self.state.request_material_switch(),
            Act::Open(other) => {
                let _ = self.state.set_document(fixture(*other), true, true);
            }
            Act::Undo => self.state.undo(),
            Act::Redo => self.state.redo(),
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
                Act::Handoff(true) => {
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
        let before = self.before();
        let was = self.was();
        self.state.notices.clear();
        self.state.protocol_log.clear();
        self.act(act);
        self.observe(act, before);
        self.check();
        self.conform(act, was);
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
    /// preparation all it needs, completes every step asked for, takes
    /// whatever the device is offered, and presses Run on a failed run, as
    /// its message asks.
    fn quiesce(&mut self) {
        let round = [
            Act::Validate,
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
