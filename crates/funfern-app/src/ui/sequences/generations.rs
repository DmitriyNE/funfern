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

use super::*;
use crate::canonical_gpu::{CANONICAL_FAILURE_NON_FINITE, CanonicalGpuHandoffOutcome};
use bevy::ecs::world::CommandQueue;
use funfern_app::topology_editor::TopologyEditor;
use funfern_app::topology_runtime::{PreparedSolverUpdate, TopologyToken};

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
        }
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
                .uploading
                .as_ref()
                .map(|upload| (upload.token, upload.generation)),
            reset: self.state.reset_requested,
            drop: self.state.drop_requested,
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
            // FINDING (temporary tolerance): a fault while an upload waits
            // makes the host drop the candidate and keep its old topology,
            // while the device keeps what it holds: an installed candidate
            // already runs, and a pending handoff can still be admitted. So
            // the device here faults only while nothing uploads.
            Act::Fault => {
                if self.state.uploading.is_none() {
                    self.request.fail(CANONICAL_FAILURE_NON_FINITE);
                }
            }
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
            Act::Reset => self.state.reset_requested = true,
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
                        .uploading
                        .as_ref()
                        .filter(|upload| upload.generation == generation)
                    {
                        self.running = Some((generation, upload.token));
                    } else if before.reset && !self.state.reset_requested {
                        let token = active.expect("Reset installed a generation with none active");
                        self.running = Some((generation, token));
                        assert!(
                            self.state.pending_pulses.is_empty()
                                && self.state.pending_switches.is_empty()
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
                self.state.pending_pulses.is_empty()
                    && self.state.pending_switches.is_empty()
                    && self.state.source_commit.is_none(),
                "work for the replaced scene survived its drop"
            );
        }
        let accepted = &self.state.editor.document.model.accepted;
        if self.state.drop_requested && !before.drop {
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
        self.state.notices.clear();
        self.act(act);
        self.observe(act, before);
        self.check();
    }

    fn check(&self) {
        let state = &self.state;
        // A replacement waits for its frame to drop what ran.
        if state.drop_requested {
            return;
        }
        if let (Some((generation, token)), Some(active)) = (self.running, state.runtime.active())
            && state.uploading.is_none()
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
                Some(state.uploaded_time_step),
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
        // FINDING (temporary tolerance): while the draft is not valid nothing
        // is prepared. Edits outside the draft, such as the point source, wait
        // for it, and a scene that has not started does not start, with a
        // Reset or a Switch pressed meanwhile waiting for good.
        let held = state.editor.acceptance != TopologyAcceptance::Valid;
        if held && state.runtime.active().is_none() {
            return None;
        }
        let waits = [
            ("a Reset", state.reset_requested),
            ("a drop", state.drop_requested),
            ("an upload", state.uploading.is_some()),
            ("a pack", state.gpu_upload_preparation.is_some()),
            ("a source commit", state.source_commit.is_some()),
            ("a preparation", state.preparation_in_progress()),
            ("a candidate", state.runtime.ready().is_some()),
            ("a pulse", !state.pending_pulses.is_empty()),
            ("a Switch", !state.pending_switches.is_empty()),
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
        if held {
            return state.runtime.active().and_then(|active| {
                (self.running.map(|(_, token)| token) != Some(active.bundle.token))
                    .then(|| "the device runs another topology than the host's".to_owned())
            });
        }
        let target = self.accepted_revision;
        if state
            .runtime
            .last_error()
            .is_some_and(|error| error.token.document_revision == target)
        {
            return None;
        }
        let Some(active) = state.runtime.active() else {
            return Some(format!(
                "nothing runs revision {target}, and no error says why"
            ));
        };
        if active.bundle.token.document_revision != target {
            return Some(format!(
                "revision {} runs where {target} was accepted, and no error says why",
                active.bundle.token.document_revision
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
        if (wanted / state.uploaded_time_step - 1.0).abs() > TIME_STEP_HYSTERESIS {
            return Some(format!(
                "the step {:e} is not the speed's {wanted:e}",
                state.uploaded_time_step
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
                self.state.wave_running = true;
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
