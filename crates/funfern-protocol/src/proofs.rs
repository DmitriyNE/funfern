//! The protocol's properties for every sequence of steps and inputs, to a
//! bounded depth, from a scene just opened. `cargo kani -p funfern-protocol`
//! checks both, in about half a minute and three minutes here.

use super::*;

/// Steps explored, every state on the way asked for the properties.
const DEPTH: usize = 9;
/// Steps explored before the fair suffix is asked to bring a state to rest.
const PREFIX: usize = 6;
/// The rounds it is given: one for each pulse or Switch the prefix can have
/// left waiting, since a round settles one live event, and three for an
/// upload to be prepared, begun and published.
const ROUNDS: usize = PREFIX + 3;

/// Every state reached keeps what `check` asks: the host's active topology is
/// the device's while nothing uploads, at the step the device runs; a
/// handoff is always one the host awaits; nothing is queued on a device that
/// is not there.
#[kani::proof]
#[kani::unwind(11)]
fn every_reachable_state_keeps_the_protocol() {
    let mut protocol = Protocol::default();
    for _ in 0..DEPTH {
        protocol.step(kani::any());
        protocol.check();
    }
}

/// From any reachable state, with preparation and the device fair from then
/// on, the runtime comes to rest: nothing waits, and the device runs the
/// request for the document as it stands, or an error names it.
#[kani::proof]
#[kani::unwind(11)]
fn the_fair_suffix_brings_any_state_to_rest() {
    let mut protocol = Protocol::default();
    for _ in 0..PREFIX {
        protocol.step(kani::any());
    }
    assert!(protocol.settle(ROUNDS, kani::any()));
}
