//! The runtime's generation protocol as a small state machine.
//!
//! The app's runtime prepares a candidate topology, packs it for the device,
//! uploads it and publishes it once the device has taken it, while the user
//! edits, resets, replaces the scene, fires pulses and throws Switches, and
//! the device answers in an order of its own. This crate holds that protocol
//! and nothing else: tokens are small numbers, a generation is a counter, and
//! what the app computes from geometry - whether a preparation finished this
//! frame, how a ready candidate is carried, whether a pulse lands in a region
//! - is an input of the step that needs it.
//!
//! The decisions the protocol makes are functions of what the host has seen,
//! and the app's runtime makes them by calling the same functions the model
//! does (see [`decisions`]).
//!
//! [`Protocol::step`] takes one [`Step`] the way the app does: a host frame
//! runs the blocks of `refresh_runtime` in its order, and the device's and
//! the user's steps change what those blocks read. [`Protocol::check`] holds
//! the properties every reachable state keeps, [`Protocol::at_rest`] says
//! whether nothing is left to happen, and [`Protocol::settle`] runs the fair
//! suffix that should bring any state to rest.
//!
//! Two engines read it. Under `cargo kani` the proofs explore every sequence
//! of steps and inputs to a bounded depth. Under the app's tests it runs in
//! lockstep with the real runtime, taking the inputs the real one met, and
//! the two states are compared after every step; that is what makes a proof
//! about this machine a statement about the app's.

pub mod decisions;

use decisions::{RequestSeen, Settlement, UploadSeen, request_due, settle_upload, standing};

/// A candidate, named by the order of its request.
pub type Token = u8;

/// The most requests one run of the machine names; a step that would request
/// past it requests nothing. The app's sequences of 120 steps make fewer.
pub const TOKENS: usize = 128;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(kani, derive(kani::Arbitrary))]
pub enum Acceptance {
    Pending,
    Valid,
    Invalid,
}

/// What a preparation in flight comes to in a frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(kani, derive(kani::Arbitrary))]
pub enum Prepared {
    NotYet,
    Done,
    Failed,
}

/// How a ready candidate is carried to the device, as the app decides it from
/// what changed and whether the step did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(kani, derive(kani::Arbitrary))]
pub enum Route {
    /// Nothing the solver runs changed: commit without the device.
    InPlace,
    /// Only the sources changed: a live patch on the running generation.
    /// `built` is whether the patch could be built for it, `packable`
    /// whether the candidate carries what a whole generation needs.
    Patch { built: bool, packable: bool },
    /// A whole generation: packed, then installed or handed off.
    Pack,
}

/// The device's live event in flight.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    Pulse,
    Switch,
    Patch(Token),
}

/// What happens to the frame's inputs that the app computes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(kani, derive(kani::Arbitrary))]
pub struct Frame {
    /// The frame's share of validation finishes the draft's.
    pub validated: bool,
    /// What a preparation reads once validation has finished, which a valid
    /// draft moves.
    pub inputs: u8,
    /// What the preparation in flight comes to.
    pub prepared: Prepared,
    /// How a candidate ready for the device is carried.
    pub route: Route,
    /// A packed candidate installs rather than hands off, its medium having
    /// started or stopped being driven.
    pub install: bool,
    /// The upload begins rather than being refused.
    pub begun: bool,
    /// The waiting pulse is inside a region of the running topology.
    pub pulse: bool,
    /// The running generation has the Switch being thrown.
    pub switch: bool,
    /// The steps the frame asks for leave the device behind.
    pub behind: bool,
    /// The speed no longer asks for the running step, past the hysteresis,
    /// so the running generation is requested again.
    pub retime: bool,
}

/// One step of the interleaving.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(kani, derive(kani::Arbitrary))]
pub enum Step {
    Frame(Frame),
    /// The device's readbacks for a frame; `caught_up` is whether every step
    /// asked for has completed.
    Readbacks {
        caught_up: bool,
    },
    /// The device settles a pending handoff.
    Handoff {
        admit: bool,
    },
    /// The device settles a pending live event.
    Settle {
        take: bool,
    },
    /// The running generation fails at its last accepted step.
    Fault,
    /// The draft's validation finishes, and what a preparation reads then.
    Validate {
        inputs: u8,
    },
    /// An edit: what a preparation reads after it, and what the draft's
    /// validation will come to.
    Edit {
        inputs: u8,
        valid: bool,
    },
    /// The mesh edge set, named by value: equal names, equal edges.
    Edge {
        edge: u8,
    },
    /// Another scene opened in this one's place: what a preparation reads of
    /// it, its mesh edge, and what its draft's validation will come to.
    Replace {
        inputs: u8,
        edge: u8,
        valid: bool,
    },
    Reset,
    Run(bool),
    /// One step, as Step does while paused.
    StepOnce,
    /// A pulse placed in the scene.
    Pulse,
    /// Switch's hotkey; `queued` is whether it queued a press, one per
    /// material, so not again for a material already waiting.
    Switch {
        queued: bool,
    },
}

/// The device's running generation, as the harness knows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Running {
    /// The host token whose topology the generation carries.
    pub token: Token,
    /// The token whose step the generation was packed for.
    pub step: Token,
    /// Its status reads ready.
    pub ready: bool,
    pub failed: bool,
    /// Every step asked for has completed.
    pub caught_up: bool,
}

/// An upload in flight: what it carries and the generation it publishes as.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Upload {
    pub token: Token,
    pub generation: u8,
}

/// The host, the device and the editor, as far as the protocol reads them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Protocol {
    // The editor.
    pub revision: u8,
    /// What a preparation reads, the accepted scene and what lies outside the
    /// draft, named by content: equal names, equal content.
    pub inputs: u8,
    pub acceptance: Acceptance,
    /// What the pending validation comes to.
    pub validates: bool,

    // The host.
    /// The revision and inputs of each token's request.
    pub requests: [(u8, u8); TOKENS],
    /// Whether each token's request started from zero.
    pub fresh_tokens: [bool; TOKENS],
    pub next: Token,
    /// The latest request, and the revision and inputs it was made at.
    pub requested: Option<Token>,
    pub requested_revision: Option<u8>,
    pub requested_inputs: Option<u8>,
    pub preparing: Option<Token>,
    pub ready: Option<Token>,
    pub packed: Option<Token>,
    pub uploading: Option<Upload>,
    pub source_commit: Option<Token>,
    pub active: Option<Token>,
    pub last_error: Option<Token>,
    /// The token whose step the host paces.
    pub uploaded: Option<Token>,
    pub reset: bool,
    pub drop: bool,
    pub fresh: bool,
    pub pulses: u8,
    pub switches: u8,
    /// A device failure the host has paused for.
    pub fault: bool,
    pub running: bool,
    pub stepping: bool,
    /// The mesh edge, named by value, and the one the latest request was
    /// made at.
    pub edge: u8,
    pub requested_edge: Option<u8>,

    // The device.
    pub generation: u8,
    pub device: Option<Running>,
    pub handoff: Option<Token>,
    /// The last handoff was refused.
    pub refused: bool,
    pub event: Option<Event>,
    /// How the device settled the source patch in flight, until the host
    /// reads it.
    pub patched: Option<bool>,
    /// The generation the display shows.
    pub display: u8,
}

impl Default for Protocol {
    /// A scene just opened, as at launch: nothing runs, nothing is asked for,
    /// and the draft is being validated.
    fn default() -> Self {
        Self {
            revision: 1,
            inputs: 0,
            acceptance: Acceptance::Pending,
            validates: true,
            requests: [(0, 0); TOKENS],
            fresh_tokens: [false; TOKENS],
            next: 0,
            requested: None,
            requested_revision: None,
            requested_inputs: None,
            preparing: None,
            ready: None,
            packed: None,
            uploading: None,
            source_commit: None,
            active: None,
            last_error: None,
            uploaded: None,
            reset: false,
            drop: true,
            fresh: true,
            pulses: 0,
            switches: 0,
            fault: false,
            running: true,
            stepping: false,
            edge: 0,
            requested_edge: None,
            generation: 0,
            device: None,
            handoff: None,
            refused: false,
            event: None,
            patched: None,
            display: 0,
        }
    }
}

impl Protocol {
    pub fn step(&mut self, step: Step) {
        match step {
            Step::Frame(frame) => self.frame(frame),
            Step::Readbacks { caught_up } => {
                if let Some(device) = &mut self.device {
                    if !device.failed {
                        device.ready = true;
                        device.caught_up = caught_up;
                    }
                    self.display = self.generation;
                }
            }
            Step::Handoff { admit } => {
                let Some(target) = self.handoff.take() else {
                    return;
                };
                if admit {
                    // The target takes over the source's count of steps
                    // asked for and done, and a clean status.
                    let caught_up = self.device.is_none_or(|device| device.caught_up);
                    self.generation = self.generation.wrapping_add(1).max(1);
                    self.device = Some(Running {
                        token: target,
                        step: target,
                        ready: true,
                        failed: false,
                        caught_up,
                    });
                    self.display = self.generation;
                    self.refused = false;
                } else {
                    self.refused = true;
                }
            }
            Step::Settle { take } => {
                if let Some(Event::Patch(_)) = self.event {
                    self.patched = Some(take);
                }
                self.event = None;
            }
            Step::Fault => {
                if let Some(device) = &mut self.device {
                    device.failed = true;
                    device.ready = false;
                }
            }
            Step::Validate { inputs } => self.validate(inputs),
            Step::Edit { inputs, valid } => {
                self.revision = self.revision.wrapping_add(1);
                self.acceptance = Acceptance::Pending;
                self.validates = valid;
                self.inputs = inputs;
            }
            Step::Edge { edge } => self.edge = edge,
            Step::Replace {
                inputs,
                edge,
                valid,
            } => {
                self.revision = self.revision.wrapping_add(1);
                self.inputs = inputs;
                self.edge = edge;
                self.acceptance = Acceptance::Pending;
                self.validates = valid;
                self.requested_revision = None;
                self.fresh = true;
                self.drop = true;
            }
            Step::Reset => self.reset = true,
            Step::Run(on) => self.running = on,
            Step::StepOnce => self.stepping = true,
            Step::Pulse => {
                if self.active.is_some() {
                    self.pulses = self.pulses.saturating_add(1);
                }
            }
            Step::Switch { queued } => {
                if queued {
                    self.switches = self.switches.saturating_add(1);
                }
            }
        }
    }

    fn validate(&mut self, inputs: u8) {
        if self.acceptance != Acceptance::Pending {
            return;
        }
        if self.validates {
            self.acceptance = Acceptance::Valid;
            self.inputs = inputs;
        } else {
            self.acceptance = Acceptance::Invalid;
        }
    }

    /// The device's failure as the host reads it: none without a generation,
    /// whose counters go with it.
    pub fn failed(&self) -> bool {
        self.device.is_some_and(|device| device.failed)
    }

    fn revision_of(&self, token: Token) -> u8 {
        self.requests[token as usize % TOKENS].0
    }

    fn standing(&self) -> Option<u8> {
        standing(
            self.acceptance,
            self.revision,
            self.requested_revision,
            || self.requested_inputs == Some(self.inputs),
        )
    }

    fn behind(&self) -> bool {
        self.device
            .is_some_and(|device| !device.caught_up && !device.failed)
    }

    /// One host frame: validation's share, then the blocks of
    /// `refresh_runtime` in its order.
    fn frame(&mut self, frame: Frame) {
        if frame.validated {
            self.validate(frame.inputs);
        }
        // A replaced scene's generation goes, and all that was for it.
        if std::mem::take(&mut self.drop) {
            self.device = None;
            self.handoff = None;
            self.refused = false;
            self.event = None;
            self.patched = None;
            self.active = None;
            self.preparing = None;
            self.ready = None;
            self.requested = None;
            self.last_error = None;
            self.uploading = None;
            self.source_commit = None;
            self.packed = None;
            self.uploaded = None;
            self.fault = false;
            self.pulses = 0;
            self.switches = 0;
        }
        // A source patch the device has settled commits, or is refused.
        if let Some(token) = self.source_commit
            && let Some(taken) = self.patched.take()
        {
            self.source_commit = None;
            if taken {
                self.commit_in_place(token);
            } else {
                self.reject(token);
            }
        }
        // A failure with nothing uploading pauses the run; Run or Step
        // clears it, on a device that has a generation to clear it on.
        if self.failed() {
            if self.uploading.is_none() {
                if !self.fault {
                    self.fault = true;
                    self.running = false;
                    self.stepping = false;
                } else if (self.running || self.stepping)
                    && let Some(device) = &mut self.device
                {
                    device.failed = false;
                    device.ready = false;
                    device.caught_up = true;
                    self.fault = false;
                }
            }
        } else {
            self.fault = false;
        }
        if self.uploading.is_none() && self.source_commit.is_none() && self.packed.is_none() {
            // `retime_for_speed`: a running step the speed no longer asks for
            // is requested again.
            if self.uploaded.is_some()
                && self.preparing.is_none()
                && self.ready.is_none()
                && self.active.is_some()
                && frame.retime
            {
                self.requested_revision = None;
            }
            self.request();
        }
        // Preparation's share.
        if let Some(token) = self.preparing {
            match frame.prepared {
                Prepared::NotYet => {}
                Prepared::Done => {
                    self.preparing = None;
                    self.ready = Some(token);
                }
                Prepared::Failed => {
                    self.preparing = None;
                    self.last_error = Some(token);
                }
            }
        }
        if self.packed.is_some() && self.packed != self.ready {
            self.packed = None;
        }
        // A ready candidate is carried.
        if self.uploading.is_none()
            && self.source_commit.is_none()
            && self.packed.is_none()
            && let Some(token) = self.ready
        {
            // Without a step running nothing can stay unchanged, and so
            // nothing is carried without a whole generation.
            let route = if self.uploaded.is_some() {
                frame.route
            } else {
                Route::Pack
            };
            match route {
                Route::InPlace => self.commit_in_place(token),
                Route::Patch { built, packable } => {
                    if self.event.is_none() {
                        let free = self
                            .device
                            .is_some_and(|device| !device.failed && self.handoff.is_none());
                        if built && free {
                            self.event = Some(Event::Patch(token));
                            self.patched = None;
                            self.source_commit = Some(token);
                        } else if self.handoff.is_some() {
                            // Busy: a later frame tries again.
                        } else if packable {
                            self.packed = Some(token);
                        } else {
                            self.reject(token);
                        }
                    }
                }
                Route::Pack => self.packed = Some(token),
            }
        }
        // A packed candidate begins its upload once the device has caught up.
        if self.uploading.is_none()
            && !self.behind()
            && let Some(token) = self.packed
        {
            self.packed = None;
            if self.ready != Some(token) {
                return;
            }
            if let Some(device) = &mut self.device
                && device.failed
            {
                device.failed = false;
                device.ready = false;
                device.caught_up = true;
                self.fault = false;
            }
            let install = self.fresh_tokens[token as usize % TOKENS]
                || self.active.is_none()
                || frame.install;
            if !frame.begun {
                self.reject(token);
            } else if install {
                self.generation = self.generation.wrapping_add(1).max(1);
                self.device = Some(Running {
                    token,
                    step: token,
                    ready: false,
                    failed: false,
                    caught_up: true,
                });
                self.handoff = None;
                self.refused = false;
                self.event = None;
                self.patched = None;
                self.uploaded = Some(token);
                self.uploading = Some(Upload {
                    token,
                    generation: self.generation,
                });
            } else {
                self.handoff = Some(token);
                self.refused = false;
                self.uploading = Some(Upload {
                    token,
                    generation: self.generation.wrapping_add(1).max(1),
                });
            }
        }
        // An upload settles.
        if let Some(upload) = self.uploading {
            let seen = UploadSeen {
                runs_candidate: self.generation == upload.generation,
                failed: self.failed(),
                refused: self.refused,
                ready: self.device.is_some_and(|device| device.ready),
                shown: self.display == upload.generation,
                handoff_pending: self.handoff.is_some(),
            };
            match settle_upload(seen) {
                Settlement::Withdraw => {
                    // One the device has already refused has nothing to
                    // withdraw.
                    if self.handoff.take().is_some() {
                        self.refused = false;
                    }
                    self.reject(upload.token);
                    self.uploading = None;
                }
                Settlement::Refused => {
                    self.reject(upload.token);
                    self.uploading = None;
                }
                Settlement::Publish => {
                    self.uploading = None;
                    self.uploaded = Some(upload.token);
                    if self.requested == Some(upload.token) && self.ready == Some(upload.token) {
                        self.ready = None;
                        self.active = Some(upload.token);
                    }
                }
                Settlement::Wait => {}
            }
        }
        // Reset installs the active topology from zero.
        if self.reset
            && self.uploading.is_none()
            && self.source_commit.is_none()
            && let Some(active) = self.active
        {
            self.generation = self.generation.wrapping_add(1).max(1);
            self.device = Some(Running {
                token: active,
                step: active,
                ready: false,
                failed: false,
                caught_up: true,
            });
            self.handoff = None;
            self.refused = false;
            self.event = None;
            self.patched = None;
            self.reset = false;
            self.uploaded = Some(active);
            self.pulses = 0;
            self.switches = 0;
        }
        if self.active.is_some() {
            // A waiting pulse, then a waiting Switch, each taken by a device
            // free for it and dropped by one that cannot take it at all.
            if self.uploading.is_none() && self.source_commit.is_none() && self.pulses > 0 {
                match self.queue(frame.pulse, Event::Pulse) {
                    Queued::Taken | Queued::Refused => self.pulses -= 1,
                    Queued::Busy => {}
                }
            }
            if self.uploading.is_none() && self.source_commit.is_none() && self.switches > 0 {
                match self.queue(frame.switch, Event::Switch) {
                    Queued::Taken | Queued::Refused => self.switches -= 1,
                    Queued::Busy => {}
                }
            }
            // Steps, unless a packed candidate or a fresh upload holds them.
            let fresh_upload = self
                .uploading
                .is_some_and(|upload| self.fresh_tokens[upload.token as usize % TOKENS]);
            let withheld = (self.uploading.is_none() && self.packed.is_some()) || fresh_upload;
            if !withheld
                && (self.running || std::mem::take(&mut self.stepping))
                && let Some(device) = &mut self.device
                && frame.behind
                && !device.failed
            {
                device.caught_up = false;
            }
        }
    }

    /// `request_runtime`: a request for the document as it stands, unless one
    /// stands already.
    fn request(&mut self) {
        let due = request_due(RequestSeen {
            acceptance: self.acceptance,
            standing: self.standing(),
            requested: self.requested_revision,
            same_edge: self.requested_edge == Some(self.edge),
            remesh: false,
            in_flight: self.preparing.is_some() || self.ready.is_some(),
            active: self.active.map(|token| self.revision_of(token)),
            failed: self.last_error.map(|token| self.revision_of(token)),
        });
        if !due || self.next as usize >= TOKENS {
            return;
        }
        let token = self.next;
        self.next += 1;
        self.requests[token as usize] = (self.revision, self.inputs);
        self.fresh_tokens[token as usize] = self.active.is_none() || self.reset || self.fresh;
        self.requested = Some(token);
        self.preparing = Some(token);
        self.ready = None;
        self.last_error = None;
        self.requested_revision = Some(self.revision);
        self.requested_inputs = Some(self.inputs);
        self.reset = false;
        self.fresh = false;
        self.requested_edge = Some(self.edge);
    }

    fn commit_in_place(&mut self, token: Token) {
        if self.requested == Some(token) && self.ready == Some(token) {
            self.ready = None;
            self.active = Some(token);
            if let Some(device) = &mut self.device {
                device.token = token;
            }
        }
    }

    fn reject(&mut self, token: Token) {
        if self.ready == Some(token) {
            self.ready = None;
            self.last_error = Some(token);
        }
    }

    fn queue(&mut self, built: bool, event: Event) -> Queued {
        if !built {
            return Queued::Refused;
        }
        let Some(device) = self.device else {
            return Queued::Refused;
        };
        if self.event.is_some() || self.handoff.is_some() {
            Queued::Busy
        } else if device.failed {
            Queued::Refused
        } else {
            self.event = Some(event);
            Queued::Taken
        }
    }

    /// What every reachable state keeps.
    pub fn check(&self) {
        // A replacement waits for its frame to drop what ran.
        if !self.drop
            && self.uploading.is_none()
            && let (Some(device), Some(active)) = (self.device, self.active)
        {
            assert_eq!(
                device.token, active,
                "the host's active topology is not the one the device runs"
            );
            assert_eq!(
                self.uploaded,
                Some(device.step),
                "the host paces a step the device does not run"
            );
        }
        if let Some(target) = self.handoff {
            let upload = self.uploading.expect("a handoff no upload waits for");
            assert_eq!(upload.token, target, "a handoff of another candidate");
            assert_eq!(
                upload.generation,
                self.generation.wrapping_add(1).max(1),
                "a handoff publishing as another generation than the host awaits"
            );
        }
        assert!(
            self.event.is_none() || self.device.is_some(),
            "a live event with nothing running"
        );
        assert!(
            self.packed.is_none() || self.uploading.is_none(),
            "a candidate packed while another uploads"
        );
        if let Some(token) = self.source_commit {
            assert!(self.uploading.is_none(), "a source commit during an upload");
            assert!(
                self.event == Some(Event::Patch(token)) || self.patched.is_some(),
                "a source commit with no patch for it"
            );
        }
    }

    /// Whether nothing is left to happen: nothing waits, and the device runs
    /// the request for the document as it stands, or an error names it.
    pub fn at_rest(&self) -> bool {
        // A Reset or a Switch pressed while nothing runs has nothing to act
        // on, and waits harmlessly for something to.
        let held = self.active.is_some() && (self.reset || self.switches > 0);
        let waiting = self.drop
            || held
            || self.uploading.is_some()
            || self.packed.is_some()
            || self.source_commit.is_some()
            || self.preparing.is_some()
            || self.ready.is_some()
            || self.pulses > 0
            || self.event.is_some()
            || self.handoff.is_some()
            || self.acceptance == Acceptance::Pending;
        if waiting {
            return false;
        }
        let standing = self.standing();
        if standing.is_some()
            && self
                .last_error
                .is_some_and(|token| Some(self.revision_of(token)) == standing)
        {
            return true;
        }
        let Some(active) = self.active else {
            return false;
        };
        let current = if self.acceptance == Acceptance::Valid {
            Some(self.revision_of(active)) == standing
        } else {
            self.requests[active as usize % TOKENS].1 == self.inputs
        };
        current
            && self.requested_edge == Some(self.edge)
            && self.device.is_some_and(|device| {
                device.token == active && device.ready && Some(device.step) == self.uploaded
            })
    }

    /// The fair suffix: validation finishes, every frame prepares and carries
    /// what it can, the device completes and admits what it is given, and a
    /// paused run is resumed, as its message asks. Answers whether it came to
    /// rest within `rounds`.
    pub fn settle(&mut self, rounds: usize, route: Route) -> bool {
        let fair = Frame {
            validated: true,
            inputs: self.inputs,
            prepared: Prepared::Done,
            route,
            install: false,
            begun: true,
            pulse: true,
            switch: true,
            behind: false,
            retime: false,
        };
        for _ in 0..rounds {
            if self.fault || self.failed() {
                self.running = true;
            }
            for step in [
                Step::Validate {
                    inputs: self.inputs,
                },
                Step::Frame(fair),
                Step::Readbacks { caught_up: true },
                Step::Handoff { admit: true },
                Step::Settle { take: true },
                Step::Frame(fair),
            ] {
                self.step(step);
                self.check();
            }
            if self.at_rest() {
                return true;
            }
        }
        false
    }
}

enum Queued {
    Taken,
    Busy,
    Refused,
}

#[cfg(kani)]
mod proofs;

#[cfg(test)]
mod tests {
    use super::*;

    /// A scene opened comes to rest, as does one whose draft never validates.
    #[test]
    fn a_scene_opened_comes_to_rest() {
        for valid in [true, false] {
            let mut protocol = Protocol {
                validates: valid,
                ..Protocol::default()
            };
            assert!(protocol.settle(8, Route::Pack), "{protocol:#?}");
            assert!(protocol.active.is_some());
        }
    }

    /// An edit, a Reset and a fault on the way all come to rest.
    #[test]
    fn edits_resets_and_faults_come_to_rest() {
        let mut protocol = Protocol::default();
        assert!(protocol.settle(8, Route::Pack));
        for step in [
            Step::Edit {
                inputs: 1,
                valid: true,
            },
            Step::Reset,
            Step::Fault,
            Step::Pulse,
        ] {
            protocol.step(step);
            protocol.check();
        }
        assert!(protocol.settle(8, Route::Pack), "{protocol:#?}");
    }
}
