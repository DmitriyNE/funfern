//! AMR-04: the user acting while an adaptation is on its way to the device.

use super::*;
use std::panic::{AssertUnwindSafe, catch_unwind};

/// Where an adaptation is on its way to the device.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stage {
    /// Its estimate finished and not yet read.
    Estimated,
    /// Its transaction started and not yet advanced.
    Adapting,
    /// Its transaction finished on a worker, the mesh not yet asked for.
    Held,
    /// The adapted mesh preparing.
    Preparing,
    /// Prepared and packed, the device not yet caught up.
    Packed,
    /// Uploading, its handoff pending on the device.
    Uploading,
    /// The device admitted the handoff, and the host has not yet published.
    Admitted,
    /// The device refused the handoff.
    Refused,
    /// Published, not yet stepped.
    Published,
}

const STAGES: [Stage; 9] = [
    Stage::Estimated,
    Stage::Adapting,
    Stage::Held,
    Stage::Preparing,
    Stage::Packed,
    Stage::Uploading,
    Stage::Admitted,
    Stage::Refused,
    Stage::Published,
];

/// Where the session strikes the field it estimates.
const STRUCK: Point2 = Point2::new(-0.4, 0.1);
const STRUCK_AGAIN: Point2 = Point2::new(0.3, -0.4);

/// A session at rest whose adaptation has reached `stage`.
fn reach(session: &mut Session, stage: Stage) -> Result<(), String> {
    session.quiesce();
    session.estimate_struck(STRUCK);
    if stage == Stage::Estimated {
        return Ok(());
    }
    session.step(&Act::Frame(Some(0)));
    if session.state.amr_adaptation_job.is_none() {
        return Err(format!(
            "the estimate started no transaction: {}",
            session.state.amr_status
        ));
    }
    if stage == Stage::Adapting {
        return Ok(());
    }
    if stage == Stage::Held {
        let mut job = session.state.amr_adaptation_job.take().unwrap();
        let result = loop {
            if let Some(result) = job.advance(1 << 16) {
                break result;
            }
        };
        session.state.amr_adaptation_completed = Some(result);
        return Ok(());
    }
    session.step(&Act::Frame(None));
    if !(session.state.runtime.requested_adaptation() && session.state.preparation_in_progress()) {
        return Err(format!(
            "the transaction asked for no mesh: {}",
            session.state.amr_status
        ));
    }
    if stage == Stage::Preparing {
        return Ok(());
    }
    session.step(&Act::Frame(None));
    if session.state.coordinator.gpu_upload_preparation.is_none() {
        return Err("the adapted mesh was not packed".into());
    }
    if stage == Stage::Packed {
        return Ok(());
    }
    session.step(&Act::Readbacks(u64::MAX));
    session.step(&Act::Frame(None));
    if session.state.coordinator.uploading.is_none()
        || session.request.handoff_outcome() != CanonicalGpuHandoffOutcome::Pending
    {
        return Err("the adapted mesh's handoff did not begin".into());
    }
    match stage {
        Stage::Uploading => Ok(()),
        Stage::Refused => {
            session.step(&Act::Handoff(false));
            Ok(())
        }
        _ => {
            session.step(&Act::Handoff(true));
            if stage == Stage::Admitted {
                return Ok(());
            }
            session.step(&Act::Frame(Some(0)));
            if !session
                .state
                .runtime
                .active()
                .is_some_and(|active| active.adapted)
            {
                return Err("the adapted mesh did not publish".into());
            }
            Ok(())
        }
    }
}

/// What the user does.
struct Action {
    name: &'static str,
    apply: fn(&mut Session),
}

fn steps(session: &mut Session, acts: &[Act]) {
    for act in acts {
        session.step(act);
    }
}

const ACTIONS: [Action; 14] = [
    Action {
        name: "nothing",
        apply: |_| {},
    },
    Action {
        name: "material",
        apply: |session| session.step(&Act::Material { nth: 0, kind: 0 }),
    },
    Action {
        name: "nudge",
        apply: |session| session.step(&Act::Nudge(0)),
    },
    Action {
        name: "invalid",
        apply: |session| session.step(&Act::Nudge(3)),
    },
    Action {
        name: "edge",
        apply: |session| session.step(&Act::Edge(0)),
    },
    Action {
        name: "remesh",
        apply: |session| session.step(&Act::Remesh),
    },
    Action {
        name: "source",
        apply: |session| {
            session.step(&Act::Source {
                cell: 3,
                on: true,
                high: true,
            })
        },
    },
    Action {
        name: "reset",
        apply: |session| session.step(&Act::Reset),
    },
    Action {
        name: "undo",
        apply: |session| {
            steps(
                session,
                &[Act::Material { nth: 0, kind: 0 }, Act::Validate, Act::Undo],
            )
        },
    },
    Action {
        name: "open",
        apply: |session| session.step(&Act::Open(true)),
    },
    Action {
        name: "pulse",
        apply: |session| steps(session, &[Act::Pulse(4), Act::Switch]),
    },
    Action {
        name: "pause",
        apply: |session| {
            steps(
                session,
                &[
                    Act::Run(false),
                    Act::Frame(None),
                    Act::Frame(None),
                    Act::Run(true),
                ],
            )
        },
    },
    Action {
        name: "speed",
        apply: |session| session.step(&Act::Speed(0)),
    },
    Action {
        name: "off",
        apply: |session| {
            session
                .state
                .editor
                .document
                .presentation
                .adaptation
                .enabled = false;
            session.step(&Act::Frame(None));
            session
                .state
                .editor
                .document
                .presentation
                .adaptation
                .enabled = true;
        },
    },
];

/// One case: the adaptation brought to `stage`, the user's `action`, the
/// session brought to rest, and adaptation asked once more, which must come
/// round and change the mesh.
fn case(stage: Stage, action: &Action) -> Result<(), String> {
    let mut session = Session::adapting();
    reach(&mut session, stage)?;
    (action.apply)(&mut session);
    session.quiesce();
    let mesh = session.state.runtime.active().unwrap().mesh.mesh_revision;
    session.estimate_struck(STRUCK_AGAIN);
    session.quiesce();
    if session.state.runtime.active().unwrap().mesh.mesh_revision == mesh {
        return Err(format!(
            "adaptation did not come round again: {}",
            session.state.amr_status
        ));
    }
    Ok(())
}

/// AMR-04: each of the user's actions at each stage of an adaptation's way
/// to the device. The session's checks hold at every step, among them that
/// an adapted mesh publishes only over the topology it was adapted from;
/// the session comes to rest running the editor's accepted scene, with
/// adaptation's history of the mesh that runs and no error; and adaptation
/// comes round again.
#[test]
fn every_action_at_every_stage_of_an_adaptation_comes_to_rest() {
    let failures = STAGES
        .iter()
        .flat_map(|stage| ACTIONS.iter().map(move |action| (*stage, action)))
        .filter_map(|(stage, action)| {
            catch_unwind(AssertUnwindSafe(|| case(stage, action)))
                .unwrap_or_else(|panic| {
                    Err(panic
                        .downcast_ref::<String>()
                        .cloned()
                        .or_else(|| panic.downcast_ref::<&str>().map(|text| text.to_string()))
                        .unwrap_or_default())
                })
                .err()
                .map(|error| format!("{stage:?} then {}: {error}", action.name))
        })
        .collect::<Vec<_>>();
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// The runtime's interleavings as `the_host_and_the_device_agree_on_what_runs`
/// draws them, from a scene at rest with adaptation on, and estimates of a
/// field struck in a cell among the steps: the protocol model takes
/// adaptation's requests as the runtime meets them.
struct Adapting;

impl ReferenceStateMachine for Adapting {
    type State = ();
    type Transition = Act;

    fn init_state() -> BoxedStrategy<()> {
        Just(()).boxed()
    }

    fn transitions(_: &()) -> BoxedStrategy<Act> {
        prop_oneof![
            6 => acts(),
            2 => any::<u8>().prop_map(Act::Adapt),
        ]
        .boxed()
    }

    fn apply(state: (), _: &Act) {
        state
    }
}

impl StateMachineTest for Adapting {
    type SystemUnderTest = Session;
    type Reference = Self;

    fn init_test(_: &()) -> Session {
        let mut session = Session::adapting();
        session.quiesce();
        session
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
    fn the_host_and_the_device_agree_while_adaptation_asks(sequential 1..120 => Adapting);
}

/// Found by the runtime sequences with adaptation among them: the device
/// refused an edit's handoff, which left the scene before it running while
/// the editor had accepted the edit. An adaptation of what ran asked for the
/// editor's scene on the running mesh, and the runtime refused it as
/// "Adapted mesh does not match the active topology", lit in the panel at
/// every cycle until an edit ran. It adapts what runs now, and the edit's
/// failure stands, not asked for again.
#[test]
fn an_adaptation_after_a_refused_edit_adapts_what_runs() {
    let mut session = Session::adapting();
    session.quiesce();
    let running = session.state.runtime.active().unwrap().clone();
    steps(
        &mut session,
        &[
            Act::Material { nth: 0, kind: 0 },
            Act::Validate,
            Act::Frame(None),
            Act::Readbacks(u64::MAX),
            Act::Frame(None),
        ],
    );
    assert_eq!(
        session.request.handoff_outcome(),
        CanonicalGpuHandoffOutcome::Pending,
        "the edit hands off"
    );
    steps(&mut session, &[Act::Handoff(false), Act::Frame(None)]);
    let edit = Some(session.state.editor.revision);
    let failed = |session: &Session| {
        session
            .state
            .runtime
            .last_error()
            .map(|error| error.token.document_revision)
    };
    assert_eq!(failed(&session), edit, "the edit failed");

    session.estimate_struck(STRUCK);
    session.quiesce();
    assert_eq!(session.state.amr_error, None);
    assert_eq!(failed(&session), edit, "the edit's failure stands");
    assert!(
        !session.state.request_is_due(),
        "and is not asked for again"
    );
    let adapted = session.state.runtime.active().unwrap();
    assert!(
        adapted.adapted && adapted.mesh.mesh_revision != running.mesh.mesh_revision,
        "{}",
        session.state.amr_status
    );
    assert_eq!(adapted.bundle.authored, running.bundle.authored);
    assert_eq!(
        adapted.bundle.token.document_revision,
        running.bundle.token.document_revision
    );
}

/// AMR-04's study: while the draft was invalid, an adapted mesh published
/// under the draft's revision, at which nothing had been requested, and the
/// next frame prepared the same scene on the same mesh again to account for
/// it, dropping the estimate that had come meanwhile. It publishes under the
/// revision it runs.
#[test]
fn an_adaptation_while_the_draft_is_invalid_asks_for_nothing_after_it() {
    let mut session = Session::adapting();
    session.quiesce();
    session.step(&Act::Nudge(3));
    session.quiesce();
    assert!(matches!(
        session.state.editor.acceptance,
        TopologyAcceptance::Invalid(_)
    ));
    let running = session.state.runtime.active().unwrap().bundle.token;
    session.estimate_struck(STRUCK);
    for act in [
        Act::Frame(None),
        Act::Readbacks(u64::MAX),
        Act::Handoff(true),
    ]
    .iter()
    .cycle()
    .take(12)
    {
        session.step(act);
        if session.state.runtime.active().unwrap().adapted {
            break;
        }
    }
    let adapted = session.state.runtime.active().unwrap();
    assert!(adapted.adapted, "{}", session.state.amr_status);
    assert_eq!(
        adapted.bundle.token.document_revision,
        running.document_revision
    );
    assert!(!session.state.request_is_due());
    // An estimate now is read, not dropped by a commit of the same scene.
    session.estimate_struck(STRUCK_AGAIN);
    session.step(&Act::Frame(None));
    assert!(
        session.state.adaptation_in_flight(),
        "{}",
        session.state.amr_status
    );
}

/// AMR-04's study: an adapted mesh an edit's request replaced before it was
/// published left its history waiting for a handoff that never came. Only its
/// own mesh, which never runs, could take it, so it did nothing; it goes with
/// the request now.
#[test]
fn an_adapted_mesh_an_edit_replaced_leaves_no_history() {
    let mut session = Session::adapting();
    reach(&mut session, Stage::Preparing).unwrap();
    assert!(session.state.amr.pending.is_some());
    session.step(&Act::Source {
        cell: 3,
        on: true,
        high: true,
    });
    session.quiesce();
    assert!(session.state.amr.pending.is_none());
}
