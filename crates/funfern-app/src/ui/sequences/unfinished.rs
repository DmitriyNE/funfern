//! The Materials panel's sequences with unfinished edits among them: text
//! typed into a field and not yet committed, and a colour being dragged in a
//! picker, each overtaken before it is let go.
//!
//! Typed text commits when its field lets go, and a picked colour when the
//! pointer does; until then each is held for the material it was begun on.
//! Opening another material, Undo and Redo move what the panel shows from
//! under it. What was begun belongs to the document it was begun in: it
//! lands there when nothing has moved, and goes when something has, never
//! landing on what came in its place. A Library row and a toolbar button act
//! on the click, which is the release, and the field lets go of that same
//! click, so whether its press came in the same frame or an earlier one
//! changes nothing.
//!
//! Its seeds are kept apart from the other machines', in a file of their own
//! named after this one.

use super::*;

/// Text typed and let go by a click, and colours picked with the history
/// stepped under the drag.
fn unfinished(step: u32) -> BoxedStrategy<Action> {
    let leave = prop_oneof![
        3 => any::<u8>().prop_map(Leave::Library),
        3 => Just(Leave::History(History::Undo)),
        2 => Just(Leave::History(History::Redo)),
        2 => Just(Leave::Elsewhere),
    ];
    let during = prop_oneof![2 => Just(History::Undo), 1 => Just(History::Redo)];
    prop_oneof![
        8 => (any::<u8>(), texts(step), leave, any::<bool>()).prop_map(
            |(field, text, leave, split)| Action::Draft {
                field,
                text,
                leave,
                split,
            }
        ),
        3 => (any::<u8>(), during).prop_map(|(nth, during)| Action::HeldPick { nth, during }),
    ]
    .boxed()
}

struct Unfinished;

impl ReferenceStateMachine for Unfinished {
    type State = Steps;
    type Transition = Action;

    fn init_state() -> BoxedStrategy<Steps> {
        Just(Steps { taken: 0 }).boxed()
    }

    /// About one unfinished edit in three actions, with the panel's own
    /// actions between to open materials, fold their rows and build the
    /// history it steps through; and parameters added more often than the
    /// panel's own sequences add them, since a name field is a parameter's.
    fn transitions(state: &Steps) -> BoxedStrategy<Action> {
        prop_oneof![
            20 => panel_actions(state.taken),
            11 => unfinished(state.taken),
            3 => Just(Action::Press(Button::AddParameter)),
        ]
        .boxed()
    }

    fn apply(state: Steps, _: &Action) -> Steps {
        Steps {
            taken: state.taken + 1,
        }
    }
}

impl StateMachineTest for Unfinished {
    type SystemUnderTest = Panel;
    type Reference = Self;

    /// The panel's start, with the open material given a parameter and its
    /// row unfolded, so a parameter's name field is there from the first
    /// action.
    fn init_test(_: &Steps) -> Panel {
        [
            Action::Fold(Header::Parameters),
            Action::Press(Button::AddParameter),
            Action::Press(Button::Apply),
        ]
        .into_iter()
        .fold(Panel::start(), Panel::step)
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
    fn unfinished_edits_land_on_their_own_document_or_nowhere(sequential 1..40 => Unfinished);
}
