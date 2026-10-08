//! The decisions the runtime makes by the protocol, as functions of what it
//! has seen. The app's runtime calls these, and so does the model, so what
//! the proofs explore is the code that decides in the app. Each answers what
//! to do and does none of it: the runtime acts on the device and the host,
//! the model on its own state. What the runtime reads to give them their
//! inputs - a display that shows a generation, a device that reads ready - is
//! its own, and its tests hold it to the model.

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
