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

/// A quantity a line or boundary probe samples along its path. The GPU records
/// four of these every frame; `MeanFlux` is derived here from `Flux`. The
/// readout chooses which to draw.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum LineProbeQuantity {
    Field,
    Transverse,
    Flux,
    MeanFlux,
    Energy,
}

impl LineProbeQuantity {
    pub(super) const ALL: [Self; 5] = [
        Self::Field,
        Self::Transverse,
        Self::Flux,
        Self::MeanFlux,
        Self::Energy,
    ];

    pub(super) const fn label_for(self, physics: PhysicsModel) -> &'static str {
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
        }
    }

    pub(super) const fn color(self) -> Color32 {
        match self {
            Self::Field => SELECT,
            Self::Transverse => Color32::from_rgb(188, 139, 255),
            Self::Flux => TEAL,
            Self::MeanFlux => RED,
            Self::Energy => GOLD,
        }
    }

    pub(super) const fn offset(self) -> usize {
        match self {
            Self::Field => 0,
            Self::Transverse => 3,
            Self::Flux => 6,
            Self::MeanFlux => 9,
            Self::Energy => 12,
        }
    }

    pub(super) const fn applies(self, _physics: PhysicsModel) -> bool {
        true
    }

    /// Whether the row is drawn from the trailing mean of the recorded flux
    /// rather than from the record the GPU wrote.
    pub(super) const fn averaged(self) -> bool {
        matches!(self, Self::MeanFlux)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum LineProbeRepresentation {
    Arclength,
    Waterfall,
    Integral,
}

impl LineProbeRepresentation {
    pub(super) const ALL: [Self; 3] = [Self::Arclength, Self::Waterfall, Self::Integral];

    pub(super) const fn label(self) -> &'static str {
        match self {
            Self::Arclength => "vs s",
            Self::Waterfall => "Waterfall",
            Self::Integral => "∫ vs t",
        }
    }

    pub(super) const fn offset(self) -> usize {
        match self {
            Self::Arclength => 0,
            Self::Waterfall => 1,
            Self::Integral => 2,
        }
    }
}

/// Per-readout presentation: which traces are drawn and the shared time window
/// every trace in that window pans and zooms together.
#[derive(Clone, Debug)]
pub(super) struct ProbeViewState {
    pub(super) live: bool,
    pub(super) end_time: f64,
    pub(super) span: f64,
    pub(super) field: bool,
    pub(super) secondary_field: bool,
    pub(super) transverse_field: bool,
    pub(super) poynting: bool,
    pub(super) energy: bool,
    pub(super) area_mean_field: bool,
    pub(super) area_rms_field: bool,
    pub(super) area_rms_transverse: bool,
    pub(super) area_mean_energy: bool,
    pub(super) area_total_energy: bool,
    pub(super) line_plots: [bool; 15],
    /// Seconds of flux the `MeanFlux` row averages over. Fixed rather than tied
    /// to the visible window, so panning and zooming move the view over the
    /// same data instead of rewriting it.
    pub(super) mean_window: f64,
    pub(super) far_waterfall: bool,
    pub(super) far_polar: bool,
    pub(super) far_power: bool,
    pub(super) waterfall_gain: f32,
}

impl ProbeViewState {
    pub(super) fn new(span: f64) -> Self {
        Self {
            live: true,
            end_time: 0.0,
            span: span.min(2.0),
            field: true,
            secondary_field: false,
            transverse_field: false,
            poynting: false,
            energy: true,
            area_mean_field: false,
            area_rms_field: true,
            area_rms_transverse: false,
            area_mean_energy: false,
            area_total_energy: true,
            // Field versus arclength and its waterfall, the mean flux profile,
            // and the two integrals.
            line_plots: [
                true, true, false, // primary component
                false, false, false, // transverse magnitude
                false, false, true, // normal flux
                true, false, false, // trailing mean of the normal flux
                false, false, true, // energy density
            ],
            // Two and a half periods of the default source, five of the flux,
            // which oscillates at twice the driven frequency.
            mean_window: 1.0,
            far_waterfall: true,
            far_polar: true,
            far_power: true,
            waterfall_gain: 1.0,
        }
    }
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
