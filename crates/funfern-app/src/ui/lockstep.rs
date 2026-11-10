//! The protocol model (`funfern-protocol`) run beside the real runtime.
//!
//! Each step of the runtime is given to the model with the inputs the
//! runtime met: a frame with what the runtime's decision log
//! ([`ProtocolNote`]) says it decided, a user's action with what it changed
//! of the document, and the device's steps with what its copies told the
//! host. After each the two are compared field by field, in the runtime's own
//! tokens. Where they part, either the runtime has left the protocol the
//! proofs are about or the model does not describe it; either way the proofs
//! say nothing until it is settled.
//!
//! The runtime machines (`sequences::generations`) run it on a test device,
//! and the `interleaved` fixture (RT-01) beside the app on a real one.

use super::*;
use crate::canonical_gpu::{CanonicalGpuDisplay, CanonicalGpuHandoffOutcome, CanonicalGpuRequest};
use crate::topology_editor::TopologyAcceptance;
use crate::topology_runtime::TopologyToken;
use funfern_protocol as model;
use std::panic::{AssertUnwindSafe, catch_unwind};

/// What the model is told an action changed, read before it.
pub(super) struct Was {
    revision: u64,
    acceptance: TopologyAcceptance,
    mesh_edge: f64,
    pulses: usize,
    switches: usize,
    history: (usize, usize),
    undo_replaces: bool,
    redo_replaces: bool,
}

impl Was {
    pub(super) fn of(state: &Playground) -> Self {
        let document = &state.editor.document;
        Was {
            revision: state.editor.revision,
            acceptance: state.editor.acceptance,
            mesh_edge: document.presentation.mesh_edge,
            pulses: state.coordinator.pending_pulses.len(),
            switches: state.coordinator.pending_switches.len(),
            history: state.editor.history_len(),
            undo_replaces: state.editor.undo_replaces_scene(),
            redo_replaces: state.editor.redo_replaces_scene(),
        }
    }

    /// Whether the draft was being validated.
    pub(super) fn pending(&self) -> bool {
        self.acceptance == TopologyAcceptance::Pending
    }
}

/// A user's action, as the model knows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Told {
    /// The draft's validation finishes.
    Validate,
    Reset,
    Remesh,
    Run(bool),
    Step,
    Pulse,
    Switch,
    /// The mesh edge set.
    Edge,
    /// The simulation speed set.
    Speed,
    /// An edit of the document: a material, a curve, the source.
    Edit,
    /// Another scene opened in this one's place.
    Open,
    Undo,
    Redo,
}

/// The device's running generation and the host token it carries, as what
/// runs the model has watched the device come to run.
pub(super) type Running = Option<(u64, TopologyToken)>;

/// The state the model and the runtime are compared on, with tokens as the
/// runtime knows them.
#[derive(Debug, PartialEq)]
pub(super) struct Projection {
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

/// The model beside the runtime, and the names it knows the runtime's
/// tokens, inputs and mesh edges by.
pub(super) struct Lockstep {
    pub(super) model: model::Protocol,
    /// The real token each of the model's requests produced, by the model's
    /// token. Two requests that change neither the document nor the mesh
    /// produce one real token, and the runtime cannot tell them apart.
    tokens: Vec<TopologyToken>,
    /// What a preparation reads, each content met so far, by the name the
    /// model knows it as; the same for mesh edges.
    inputs: Vec<PreparedInputs>,
    edges: Vec<f64>,
}

impl Lockstep {
    /// The model of a scene just opened in `state`, told the scene's inputs
    /// by name, as at launch.
    pub(super) fn opened(state: &Playground) -> Self {
        let mut lockstep = Lockstep {
            model: model::Protocol::default(),
            tokens: Vec::new(),
            inputs: Vec::new(),
            edges: Vec::new(),
        };
        lockstep.model.inputs = lockstep.inputs_name(state);
        lockstep.model.edge = lockstep.edge_name(state);
        lockstep
    }

    /// How many requests the model has named: past [`model::TOKENS`] it
    /// names none.
    pub(super) fn requests(&self) -> usize {
        self.tokens.len()
    }

    /// The model's name for the mesh edge now.
    fn edge_name(&mut self, state: &Playground) -> u8 {
        let edge = state.editor.document.presentation.mesh_edge;
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
    fn inputs_name(&mut self, state: &Playground) -> u8 {
        let document = &state.editor.document;
        let index = match self.inputs.iter().position(|known| known.read(document)) {
            Some(index) => index,
            None => {
                self.inputs.push(PreparedInputs::of(document));
                self.inputs.len() - 1
            }
        };
        index as u8
    }

    /// The real token the model's `token` names.
    fn real(&self, token: model::Token) -> TopologyToken {
        self.tokens[token as usize]
    }

    /// The model's step for a host frame, with the inputs the runtime met:
    /// what it logged, and what the frame's validation came to since `was`.
    pub(super) fn frame(
        &mut self,
        state: &Playground,
        request: &CanonicalGpuRequest,
        was: &Was,
        log: &[ProtocolNote],
    ) -> model::Step {
        let inputs = self.inputs_name(state);
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
            behind: !request.caught_up(),
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

    /// The model's step for a user's action, with what it changed since
    /// `was`; none where it changed nothing the model knows.
    pub(super) fn told(&mut self, act: Told, state: &Playground, was: &Was) -> Option<model::Step> {
        let inputs = self.inputs_name(state);
        Some(match act {
            Told::Validate => model::Step::Validate { inputs },
            Told::Reset => model::Step::Reset,
            Told::Remesh => model::Step::Remesh,
            Told::Run(on) => model::Step::Run(on),
            Told::Step => model::Step::StepOnce,
            Told::Pulse => {
                if state.coordinator.pending_pulses.len() <= was.pulses {
                    return None;
                }
                model::Step::Pulse
            }
            Told::Switch => model::Step::Switch {
                queued: state.coordinator.pending_switches.len() > was.switches,
            },
            Told::Edge => {
                if state.editor.document.presentation.mesh_edge == was.mesh_edge {
                    return None;
                }
                let edge = self.edge_name(state);
                model::Step::Edge { edge }
            }
            // The speed reaches the model through the frames that retime,
            // and asks for a retime again.
            Told::Speed => model::Step::Speed,
            Told::Edit | Told::Open | Told::Undo | Told::Redo => {
                // A scene opened replaces this one, and so does a step of the
                // history that crosses a replacement.
                let moved = state.editor.history_len() != was.history;
                let replaced = match act {
                    Told::Open => true,
                    Told::Undo => moved && was.undo_replaces,
                    Told::Redo => moved && was.redo_replaces,
                    _ => false,
                };
                let valid = state.editor.document.model.draft.compile(0).is_ok();
                if replaced {
                    let edge = self.edge_name(state);
                    model::Step::Replace {
                        inputs,
                        edge,
                        valid,
                    }
                } else if state.editor.revision != was.revision {
                    model::Step::Edit { inputs, valid }
                } else {
                    return None;
                }
            }
        })
    }

    /// Steps the model by `steps`, the runtime having taken them and logged
    /// `log`, and holds the two to one state. A frame may request twice: an
    /// edit's candidate committed in place, and then adaptation's.
    pub(super) fn conform(
        &mut self,
        steps: &[model::Step],
        log: &[ProtocolNote],
        state: &Playground,
        request: &CanonicalGpuRequest,
        display: &CanonicalGpuDisplay,
        running: Running,
    ) -> Result<(), String> {
        let next = self.model.next;
        for step in steps {
            let model = &mut self.model;
            catch_unwind(AssertUnwindSafe(|| {
                model.step(*step);
                model.check();
            }))
            .map_err(|panic| {
                let reason = panic
                    .downcast_ref::<String>()
                    .cloned()
                    .or_else(|| {
                        panic
                            .downcast_ref::<&str>()
                            .map(|reason| reason.to_string())
                    })
                    .unwrap_or_default();
                format!("the protocol model broke its own rule at {step:?}: {reason}")
            })?;
        }
        let requested = log
            .iter()
            .filter_map(|note| match *note {
                ProtocolNote::Requested(real) | ProtocolNote::Adapted(real) => Some(real),
                _ => None,
            })
            .collect::<Vec<_>>();
        if usize::from(self.model.next - next) != requested.len() {
            return Err(format!(
                "the runtime requested {requested:?} and the model {} times",
                self.model.next - next
            ));
        }
        self.tokens.extend(requested);
        let real = Self::projection(state, request, display, running);
        let modelled = self.modelled();
        if real != modelled {
            return Err(format!(
                "the runtime and the protocol model part:\n  runtime {real:?}\n  model   {modelled:?}"
            ));
        }
        Ok(())
    }

    /// The runtime in the model's terms.
    pub(super) fn projection(
        state: &Playground,
        request: &CanonicalGpuRequest,
        display: &CanonicalGpuDisplay,
        running: Running,
    ) -> Projection {
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
            generation: request.generation() as u8,
            installed: request.running_node_count().is_some(),
            carries: running
                .filter(|_| request.running_node_count().is_some())
                .map(|(_, token)| token),
            ready_status: request.ready(),
            failed: request.failed(),
            behind: !request.caught_up(),
            handoff: request.handoff_outcome() == CanonicalGpuHandoffOutcome::Pending,
            refused: matches!(
                request.handoff_outcome(),
                CanonicalGpuHandoffOutcome::Rejected(_)
            ),
            event: request.live_event_pending(),
            display: display.generation as u8,
        }
    }

    /// The model's state in the same terms.
    pub(super) fn modelled(&self) -> Projection {
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
}

/// What the host had read of the device when a step ended, for the next to
/// say what the device's copies told it since: the model's device steps.
#[derive(Clone, Copy, Debug)]
pub(super) struct Heard {
    copies: u64,
    controls: u64,
    handoff: bool,
    event: bool,
    failed: bool,
}

impl Heard {
    pub(super) fn now(request: &CanonicalGpuRequest, display: &CanonicalGpuDisplay) -> Self {
        Heard {
            copies: display.copies,
            controls: display.controls,
            handoff: request.handoff_outcome() == CanonicalGpuHandoffOutcome::Pending,
            event: request.live_event_pending(),
            failed: request.failed(),
        }
    }

    /// The device's steps since: a control or state copy of the running
    /// generation read, a handoff or a live event settled, a failure read. `caught_up`
    /// also reads a failed run as caught up, which the model is told on its
    /// own, after it.
    pub(super) fn since(
        self,
        request: &CanonicalGpuRequest,
        display: &CanonicalGpuDisplay,
    ) -> Vec<model::Step> {
        let mut steps = Vec::new();
        if display.copies != self.copies {
            steps.push(model::Step::Readbacks {
                caught_up: request.stats().completed_steps() >= request.requested_steps(),
                control: display.controls != self.controls,
            });
        }
        if self.handoff {
            match request.handoff_outcome() {
                CanonicalGpuHandoffOutcome::Accepted => {
                    steps.push(model::Step::Handoff { admit: true })
                }
                CanonicalGpuHandoffOutcome::Rejected(_) => {
                    steps.push(model::Step::Handoff { admit: false })
                }
                _ => {}
            }
        }
        if self.event && !request.live_event_pending() {
            steps.push(model::Step::Settle {
                take: request.stats().event_rejection() == 0,
            });
        }
        if !self.failed && request.failed() {
            steps.push(model::Step::Fault);
        }
        steps
    }
}
