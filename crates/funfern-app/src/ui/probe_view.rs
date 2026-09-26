//! Per-probe presentation state: what each readout is showing, over what time
//! span, and the sample rings behind it.
use super::{GOLD, RED, SELECT, TEAL, primary_field_label, transverse_field_magnitude_label};
use crate::wave_gpu::{AreaProbeRecord, CurveProbeRecord, FarFieldRecord, PointProbeRecord};
use bevy_egui::egui::Color32;
use funfern_core::*;
use std::collections::VecDeque;

#[derive(Clone, Debug)]
pub(super) struct ProbeTrace {
    pub(super) samples: VecDeque<PointProbeRecord>,
    pub(super) last_time: f64,
}

use funfern_app::document::ProbeReadout;
pub(super) use funfern_app::document::{LineProbeQuantity, LineProbeRepresentation};

/// What the readout draws for each line-probe quantity: its name in this skin,
/// its colour, and whether the skin has it.
pub(super) trait LineProbeQuantityView {
    fn label_for(self, physics: PhysicsModel) -> &'static str;
    fn color(self) -> Color32;
    fn applies(self, physics: PhysicsModel) -> bool;
}

impl LineProbeQuantityView for LineProbeQuantity {
    fn label_for(self, physics: PhysicsModel) -> &'static str {
        match self {
            Self::Field => primary_field_label(physics),
            Self::Transverse => transverse_field_magnitude_label(physics),
            Self::Flux => match physics {
                PhysicsModel::Mechanical => "Normal energy flux",
                PhysicsModel::Electromagnetic { .. } => "Normal Poynting flux",
            },
            // Named plainly rather than with angle brackets: egui's default
            // font has no glyph for those and drew them as tofu.
            Self::MeanFlux => "Average flux",
            Self::Energy => "Energy density",
            Self::MeanEnergy => "Average energy density",
        }
    }

    fn color(self) -> Color32 {
        match self {
            Self::Field => SELECT,
            Self::Transverse => Color32::from_rgb(188, 139, 255),
            Self::Flux => TEAL,
            Self::MeanFlux => RED,
            Self::Energy => GOLD,
            Self::MeanEnergy => Color32::from_rgb(240, 150, 90),
        }
    }

    fn applies(self, _physics: PhysicsModel) -> bool {
        true
    }
}

/// Per-readout presentation: where the shared time window sits, and the
/// document's `ProbeReadout` for which traces are drawn over how long. Every
/// trace in a readout pans and zooms together.
#[derive(Clone, Debug)]
pub(super) struct ProbeViewState {
    pub(super) live: bool,
    pub(super) end_time: f64,
    pub(super) readout: ProbeReadout,
}

impl ProbeViewState {
    pub(super) fn new(readout: ProbeReadout) -> Self {
        Self {
            live: true,
            end_time: 0.0,
            readout,
        }
    }
}

/// What a readout's window changed of its document readout, if anything. The
/// window shows `shown`, which is `stored` with its span and mean window
/// clamped to the history recorded so far; those clamps are the display's, so
/// a span or window the user left alone keeps its stored value rather than
/// being shortened for good by a look early in a run.
pub(super) fn edited_readout(
    stored: ProbeReadout,
    shown: ProbeReadout,
    after: ProbeReadout,
) -> Option<ProbeReadout> {
    if after == shown {
        return None;
    }
    let mut edited = after;
    if after.span == shown.span {
        edited.span = stored.span;
    }
    if after.mean_window == shown.mean_window {
        edited.mean_window = stored.mean_window;
    }
    Some(edited)
}

pub(super) struct CurveTrace {
    pub(super) records: VecDeque<CurveProbeRecord>,
    pub(super) last_time: f64,
}
impl Default for CurveTrace {
    fn default() -> Self {
        Self {
            records: VecDeque::new(),
            last_time: -1.0,
        }
    }
}

pub(super) struct AreaTrace {
    pub(super) records: VecDeque<AreaProbeRecord>,
    pub(super) last_time: f64,
}
impl Default for AreaTrace {
    fn default() -> Self {
        Self {
            records: VecDeque::new(),
            last_time: -1.0,
        }
    }
}

pub(super) struct FarFieldTrace {
    pub(super) records: VecDeque<FarFieldRecord>,
    pub(super) last_time: f64,
}
impl Default for FarFieldTrace {
    fn default() -> Self {
        Self {
            records: VecDeque::new(),
            last_time: -1.0,
        }
    }
}
impl Default for ProbeTrace {
    fn default() -> Self {
        Self {
            samples: VecDeque::new(),
            last_time: -1.0,
        }
    }
}
