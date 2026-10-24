//! The diagnostics log: what the transient status channels said before they
//! were overwritten.

/// How many entries the ring keeps.
pub(super) const EVENT_LOG_ENTRIES: usize = 200;

/// Which transient channel a log entry was caught from, which is also how much
/// it matters.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum EventSource {
    Status,
    Repair,
    Preparation,
    Adaptation,
    /// A browser background worker aborted.
    Worker,
    /// The painted field fell back to the linear `Q/M` because a field-law
    /// map could not be inverted (UI-19 of the 2026-10-20 review).
    Display,
}

impl EventSource {
    const fn tag(self) -> &'static str {
        match self {
            Self::Status => "status",
            Self::Repair => "repair",
            Self::Preparation => "preparation",
            Self::Adaptation => "adaptation",
            Self::Worker => "worker",
            Self::Display => "display",
        }
    }
    /// Whether an entry from here lights the status marker until the
    /// diagnostics are opened. A repair fallback explains a rebuild that
    /// succeeded, so it is worth keeping but not worth interrupting for.
    pub(super) const fn error(self) -> bool {
        matches!(
            self,
            Self::Preparation | Self::Adaptation | Self::Worker | Self::Display
        )
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct EventEntry {
    /// Seconds since the window system started, as egui counts them.
    pub(super) time: f64,
    pub(super) source: EventSource,
    pub(super) text: String,
    /// How many times in a row this same line arrived, so a channel that
    /// clears and returns every frame cannot flood the ring.
    pub(super) repeats: usize,
}

pub(super) fn event_line(entry: &EventEntry) -> String {
    // Rounded to the tenth shown before the minutes are split off: split
    // first, 59.96 s read "0:60.0" (UI-33 of the 2026-10-20 review).
    let tenths = (entry.time * 10.0).round().max(0.0) / 10.0;
    let minutes = (tenths / 60.0).floor();
    let seconds = tenths - minutes * 60.0;
    format!(
        "{minutes:.0}:{seconds:04.1} · {} · {}{}",
        entry.source.tag(),
        entry.text,
        if entry.repeats > 1 {
            format!(" ×{}", entry.repeats)
        } else {
            String::new()
        }
    )
}

#[cfg(test)]
mod tests {
    use super::super::*;
    use super::*;

    /// UI-33 of the 2026-10-20 review: the seconds were rounded to tenths
    /// after the minutes were split off, so a time just under a minute read
    /// "0:60.0". Rounded first, it reads "1:00.0".
    #[test]
    fn a_time_just_under_a_minute_reads_as_the_minute() {
        let line = |time: f64| {
            event_line(&EventEntry {
                time,
                source: EventSource::Adaptation,
                text: "Invalid adaptation source".into(),
                repeats: 1,
            })
        };
        assert_eq!(
            line(59.96),
            "1:00.0 · adaptation · Invalid adaptation source"
        );
        assert_eq!(
            line(59.94),
            "0:59.9 · adaptation · Invalid adaptation source"
        );
        assert_eq!(line(8.0), "0:08.0 · adaptation · Invalid adaptation source");
        assert_eq!(
            line(119.97),
            "2:00.0 · adaptation · Invalid adaptation source"
        );
    }

    /// The ring drops its oldest rather than growing without bound.
    #[test]
    fn the_log_is_bounded() {
        let mut state = Playground::default();
        for index in 0..EVENT_LOG_ENTRIES + 20 {
            state.message = format!("message {index}");
            state.record_events(index as f64);
        }
        assert_eq!(state.events.len(), EVENT_LOG_ENTRIES);
        assert_eq!(state.events[0].text, format!("message {}", 20));
        assert_eq!(
            state.events[EVENT_LOG_ENTRIES - 1].text,
            format!("message {}", EVENT_LOG_ENTRIES + 19)
        );
    }
}
