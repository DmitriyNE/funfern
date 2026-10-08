//! The Materials panel's sequences with the document's other edits among
//! them, saved and opened again between any two: by Save and Open, by a
//! shared link, and by the browser's autosave.
//!
//! The file format's contract is that a document comes back exactly as it
//! was saved, and the persistence tests hold it for documents built by hand.
//! Here the documents are the ones a session reaches: drafts left invalid
//! over an accepted scene, curves deleted and undone, ids handed out after a
//! reopen. Each reopen goes through the handler that route has in the app,
//! and checks that the document comes back equal, that validation brings it
//! where the session would have come, and that the history starts afresh.
//! Every action besides is held to the panel's own post-conditions, and
//! after every one the document must still save, since the autosave writes
//! it within a second.
//!
//! Its seeds are kept apart from the panel machine's, in a file of their own
//! named after this one.

use super::*;

/// The document's other edits: drawing, dividing, baffles, dragging control
/// points, deleting and detaching curves, probes, the source and the domain.
fn geometry() -> BoxedStrategy<Action> {
    prop_oneof![
        3 => (any::<bool>(), any::<u8>(), any::<u8>())
            .prop_map(|(hole, cell, size)| Action::Draw { hole, cell, size }),
        2 => any::<u8>().prop_map(|at| Action::Divide { at }),
        2 => (any::<u8>(), any::<u8>()).prop_map(|(cell, turn)| Action::Baffle { cell, turn }),
        4 => (any::<u8>(), any::<u8>(), any::<u8>())
            .prop_map(|(curve, control, step)| Action::Nudge { curve, control, step }),
        2 => (any::<u8>(), any::<u8>()).prop_map(|(curve, keep)| Action::Remove { curve, keep }),
        2 => (any::<u8>(), any::<u8>()).prop_map(|(curve, end)| Action::Detach { curve, end }),
        1 => any::<u8>().prop_map(|cell| Action::Probe { cell }),
        1 => (any::<u8>(), any::<bool>()).prop_map(|(cell, on)| Action::Source { cell, on }),
        1 => any::<u8>().prop_map(Action::Domain),
    ]
    .boxed()
}

struct Reopening;

impl ReferenceStateMachine for Reopening {
    type State = Steps;
    type Transition = Action;

    fn init_state() -> BoxedStrategy<Steps> {
        Just(Steps { taken: 0 }).boxed()
    }

    /// About one reopen in nine actions, so a sequence of forty saves and
    /// opens four or five times, with the edits between of every kind.
    fn transitions(state: &Steps) -> BoxedStrategy<Action> {
        prop_oneof![
            20 => panel_actions(state.taken),
            10 => geometry(),
            2 => any::<u8>().prop_map(Action::Toggle),
            4 => prop_oneof![Just(Route::File), Just(Route::Link), Just(Route::Autosave)]
                .prop_map(Action::Reopen),
        ]
        .boxed()
    }

    fn apply(state: Steps, _: &Action) -> Steps {
        Steps {
            taken: state.taken + 1,
        }
    }
}

impl StateMachineTest for Reopening {
    type SystemUnderTest = Panel;
    type Reference = Self;

    fn init_test(_: &Steps) -> Panel {
        Panel::start()
    }

    fn apply(panel: Panel, _: &Steps, action: Action) -> Panel {
        panel.step(action)
    }

    fn check_invariants(panel: &Panel, _: &Steps) {
        panel.check();
    }
}

prop_state_machine! {
    #![proptest_config(ProptestConfig {
        cases: 64,
        ..ProptestConfig::default()
    })]
    #[test]
    fn documents_survive_reopening_between_any_edits(sequential 1..40 => Reopening);
}
