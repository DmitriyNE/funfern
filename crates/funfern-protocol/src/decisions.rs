//! The decisions the runtime makes by the protocol, as functions of what it
//! has seen. The app's runtime calls these, and so does the model, so what
//! the proofs explore is the code that decides in the app. Each answers what
//! to do and does none of it: the runtime acts on the device and the host,
//! the model on its own state. What the runtime reads to give them their
//! inputs - a display that shows a generation, a device that reads ready - is
//! its own, and its tests hold it to the model.

use crate::Acceptance;

/// What the host sees of the upload in flight, in a frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UploadSeen {
    /// The device already runs the candidate: an install replaces the
    /// running generation at once, and an admitted handoff has made the
    /// target the running one.
    pub runs_candidate: bool,
    /// The running generation has failed.
    pub failed: bool,
    /// The device refused the handoff.
    pub refused: bool,
    /// The running generation's status reads ready.
    pub ready: bool,
    /// The display shows the upload's generation.
    pub shown: bool,
    /// The handoff still waits for the device.
    pub handoff_pending: bool,
}

/// What becomes of the upload in flight.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Settlement {
    /// The generation the candidate hands off from failed before the device
    /// admitted the handoff: the handoff goes, on the device too, and the
    /// candidate with it. Left there it could still be admitted, and the
    /// device ran a candidate the host had dropped. The running generation
    /// stays, paused by its failure.
    Withdraw,
    /// The device refused the handoff, and kept the running generation: the
    /// candidate goes.
    Refused,
    /// The device has published the candidate. A failure of a candidate the
    /// device already runs is its own, and the generation it replaced is
    /// gone, so the host publishes what the device runs and the failure
    /// pauses it as any does. Kept back, the host went on pacing and painting
    /// a topology the device no longer had.
    Publish,
    /// Not yet.
    Wait,
}

pub fn settle_upload(seen: UploadSeen) -> Settlement {
    if seen.failed && !seen.runs_candidate {
        Settlement::Withdraw
    } else if seen.refused {
        Settlement::Refused
    } else if (seen.ready || (seen.failed && seen.runs_candidate))
        && seen.shown
        && !seen.handoff_pending
    {
        Settlement::Publish
    } else {
        Settlement::Wait
    }
}

/// The revision a request for the document as it stands was made at, if one
/// stands. A valid draft is the accepted scene, and a request at its revision
/// is for it. An invalid draft's edits move the revision and nothing a
/// preparation reads, so while the draft is invalid the latest request still
/// stands if what it read, `reads_current`, has not changed; otherwise every
/// edit of the invalid draft would prepare the same scene again. None stands
/// while the draft is being validated, which may yet change the accepted
/// scene.
pub fn standing<R: Copy>(
    acceptance: Acceptance,
    revision: R,
    requested: Option<R>,
    reads_current: impl FnOnce() -> bool,
) -> Option<R> {
    match acceptance {
        Acceptance::Pending => None,
        Acceptance::Valid => Some(revision),
        Acceptance::Invalid => requested.filter(|_| reads_current()),
    }
}

/// What a request for the document as it stands meets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RequestSeen<R> {
    pub acceptance: Acceptance,
    /// The revision a request for the document as it stands was made at; see
    /// [`standing`].
    pub standing: Option<R>,
    /// The revision the latest request was made at.
    pub requested: Option<R>,
    /// The latest request was made at the mesh edge set now.
    pub same_edge: bool,
    /// A rebuild was asked for though nothing changed.
    pub remesh: bool,
    /// A candidate is being prepared, or waits to be carried.
    pub in_flight: bool,
    /// The revision the active topology was requested at.
    pub active: Option<R>,
    /// The revision whose preparation failed last.
    pub failed: Option<R>,
}

/// Whether the document as it stands takes a new request. Not while its
/// draft is being validated, which may yet change the accepted scene. An
/// invalid draft holds nothing back: it leaves the accepted scene alone, and
/// what a preparation reads, that scene and what lies outside the draft, is
/// prepared without it. Waiting for a valid draft, a scene made invalid
/// before its first validation never started, and a source switched off
/// meanwhile ran on.
///
/// Nor is one due while the request standing for it is accounted for: a
/// preparation in flight for it, the active topology, or a failure to prepare
/// it. Without that last a revision that cannot be prepared is retried every
/// frame forever: the whole preparation runs again, the phase label churns,
/// and the error it is reporting is replaced before it can be read. The next
/// edit, a different mesh edge or a rebuild asked for moves on; Reset installs
/// the active topology afresh and asks for nothing.
pub fn request_due<R: Copy + PartialEq>(seen: RequestSeen<R>) -> bool {
    if seen.acceptance == Acceptance::Pending {
        return false;
    }
    let accounted = seen.standing.is_some()
        && seen.requested == seen.standing
        && seen.same_edge
        && (seen.in_flight || seen.active == seen.standing || seen.failed == seen.standing);
    seen.remesh || !accounted
}

/// What the host sees of a failure of the running generation, in a frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FaultSeen {
    /// The running generation has failed.
    pub failed: bool,
    /// An upload is in flight; its settlement answers for a failure.
    pub uploading: bool,
    /// The host has paused for this failure already.
    pub latched: bool,
    /// Run or Step asks for steps.
    pub resuming: bool,
}

/// What the host does about a failure of the running generation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Supervision {
    /// There is none, and nothing is paused for one.
    Clear,
    /// The run pauses at the last accepted step, and says why. Nothing is
    /// clipped or reset.
    Pause,
    /// Run or Step on a paused failure clears it and retries from the last
    /// accepted step.
    Resume,
    /// Not now: the run stays paused, or the upload in flight settles it.
    Hold,
}

pub fn supervise_fault(seen: FaultSeen) -> Supervision {
    if !seen.failed {
        Supervision::Clear
    } else if seen.uploading {
        Supervision::Hold
    } else if !seen.latched {
        Supervision::Pause
    } else if seen.resuming {
        Supervision::Resume
    } else {
        Supervision::Hold
    }
}

/// What to do with a live event the running generation would not take.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LiveEventFallback {
    /// The generation is busy. The event waits, or the candidate it carries
    /// stays ready, and a later frame tries the same again.
    Retry,
    /// The generation cannot take this patch at all. The edit goes through a
    /// whole prepared generation instead, which is what it did before the
    /// patch existed.
    Pack,
    /// The generation cannot take it, and nothing else carries it: a pulse or
    /// a Switch press goes, and so does an edit not prepared in a form that
    /// can be packed. Nothing can carry it, so say so rather than fail further
    /// on with a reason that names neither cause.
    Refuse,
}

/// A refused live patch is a reason to take the slow path, never a reason to
/// lose the edit.
///
/// Source moves and weight edits were routed onto the live-patch fast path in
/// "Avoid full handoffs for sources and measurements"; the day after, temporal
/// material events arrived and gated every patch kind whose composition with a
/// driven medium had not been tested. A driven scene therefore took the fast
/// path and was then refused, and the refusal rejected the prepared candidate -
/// so moving a source on a pumped medium reported a failed preparation and
/// dropped the edit. Neither change is wrong on its own.
///
/// Packing is the fallback because it is the path these edits took before the
/// patch existed - but only for a candidate prepared with the handoff maps a
/// pack needs. A source-only preparation deliberately builds none of them, on
/// the promise that a live patch will carry the edit; where that promise cannot
/// be kept, the preparation now makes a whole generation instead, so the last
/// case should not arise. It is kept truthful rather than trusted, because what
/// it replaced failed later on with a reason that named neither the refusal nor
/// the missing maps.
pub fn live_event_fallback(busy: bool, packable: bool) -> LiveEventFallback {
    if busy {
        LiveEventFallback::Retry
    } else if packable {
        LiveEventFallback::Pack
    } else {
        LiveEventFallback::Refuse
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The reported regression: moving a continuous source on a pumped medium
    /// reported "this event has not passed its Stage 7 temporal composition
    /// gate" as a failed preparation, and the edit was lost. A refusal packs
    /// an edit prepared to be packed and says so of one that was not; only a
    /// generation merely busy is tried again, since packing would throw away
    /// a fast path about to be available.
    #[test]
    fn a_refused_source_patch_packs_instead_of_losing_the_edit() {
        assert_eq!(live_event_fallback(false, true), LiveEventFallback::Pack);
        assert_eq!(live_event_fallback(false, false), LiveEventFallback::Refuse);
        for packable in [true, false] {
            assert_eq!(
                live_event_fallback(true, packable),
                LiveEventFallback::Retry
            );
        }
    }
}
