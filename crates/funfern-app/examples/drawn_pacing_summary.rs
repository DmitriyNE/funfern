//! Summarises a drawn pacing trace: how evenly the state each rendered frame
//! drew advanced. Record one by running the app with `FUNFERN_PACING_TRACE`
//! set to a path, then
//!
//! ```text
//! cargo run -p funfern-app --example drawn_pacing_summary -- TRACE [--skip=SECONDS]
//! ```
//!
//! `--skip` leaves out the first seconds of the trace (default 2), for the
//! warm-up and the scene loading.

use funfern_app::drawn_pacing::{parse_trace, summarize};

fn main() {
    let mut path = None;
    let mut skip = 2.0;
    for argument in std::env::args().skip(1) {
        if let Some(seconds) = argument.strip_prefix("--skip=") {
            skip = seconds.parse().expect("--skip takes seconds");
        } else {
            path = Some(argument);
        }
    }
    let path = path.expect("usage: drawn_pacing_summary TRACE [--skip=SECONDS]");
    let text = std::fs::read_to_string(&path).expect("cannot read the trace");
    let frames = parse_trace(&text).expect("cannot parse the trace");
    println!("{path}: {} frames", frames.len());
    println!("{}", summarize(&frames, skip));
}
