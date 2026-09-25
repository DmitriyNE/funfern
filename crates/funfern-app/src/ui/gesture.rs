//! What a pointer gesture is currently doing, and what it landed on: draw and
//! drag state, marquee modes, and the gizmo and probe hit results.
use super::DrawTool;
use bevy_egui::egui::Pos2;
use funfern_app::document::ProbeId;
use funfern_app::topology_editor::{TopologyAttachment, TopologyProbeTarget};
use funfern_app::topology_viewport::{AttachmentHit, TopologyHandle, TopologySpanTarget};
use funfern_core::*;
use std::collections::BTreeSet;

#[derive(Clone, Debug)]
pub(super) struct DrawGesture {
    pub(super) tool: DrawTool,
    pub(super) points: Vec<Point2>,
    pub(super) attachments: Vec<Option<TopologyAttachment>>,
}

/// Which part of the outer rectangle a domain resize has hold of.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum DomainDrag {
    Side { side: OuterSide, start: DomainRect },
    Corner { index: usize, start: DomainRect },
}

#[derive(Clone, Debug)]
pub(super) enum DragGesture {
    Handle {
        handle: TopologyHandle,
    },
    /// The outer rectangle being resized by one of its sides or corners.
    Domain {
        drag: DomainDrag,
    },
    /// A loose end of an open curve on the move. `snap` is the target it would
    /// weld onto if released now, for the preview ring only.
    Endpoint {
        curve: CurveId,
        node: usize,
        snap: Option<AttachmentHit>,
    },
    Spans {
        start: Point2,
        pivot: Point2,
        custom_pivot: bool,
        gizmo_before: Option<(BTreeSet<TopologySpanTarget>, Point2)>,
        geometry: TopologyGeometry,
    },
    Rotate {
        pivot: Point2,
        start_angle: f64,
        geometry: TopologyGeometry,
    },
    Scale {
        axis: GizmoScaleAxis,
        pivot: Point2,
        start_distance: f64,
        geometry: TopologyGeometry,
    },
    Pivot {
        previous: Option<(BTreeSet<TopologySpanTarget>, Point2)>,
        offset: Point2,
    },
    Marquee {
        start: Pos2,
        current: Pos2,
        base: BTreeSet<TopologySpanTarget>,
        operation: MarqueeOperation,
    },
    Source,
    MaterialFrame {
        region: RegionId,
        start: MaterialFrame,
        hit: MaterialFrameGizmoHit,
        grab: f64,
    },
    Probe {
        hit: ProbeHit,
        grab: Point2,
        original: TopologyProbeTarget,
    },
}

/// A change worked out as far as the survivor question, waiting for the click
/// that answers it.
#[derive(Clone, Debug)]
pub(super) struct PendingMerge {
    pub(super) action: MergeAction,
    pub(super) choices: Vec<RegionId>,
}

/// What will be done once the question is answered. Both kinds fold two
/// subdomains into one face - a deletion by removing the edge between them, a
/// weld by moving an end until the circuit that separated them no longer
/// closes - and neither says by itself which material survives.
#[derive(Clone, Debug)]
pub(super) enum MergeAction {
    /// The span selection the deletion named, rather than the target planned
    /// from it, so the staleness guard has one thing to check and the target is
    /// rebuilt against whatever the document says when the answer arrives.
    Delete(BTreeSet<CurveSpanId>),
    Weld {
        curve: CurveId,
        node: usize,
        endpoint: usize,
        target: TopologyAttachment,
    },
}

/// The two grips of a region's material/source frame: its origin and the ring
/// that turns its local axes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum MaterialFrameGizmoHit {
    Origin,
    Rotate,
}

pub(super) const MATERIAL_FRAME_RADIUS: f32 = 42.0;
/// How near a control's dot a press must land to take the control rather than
/// a selected span it lies on: the drawn dot and a little margin.
pub(super) const DIRECT_HANDLE_RADIUS: f32 = 6.0;

/// What a pointer landed on within a probe. Endpoint and radius grips take
/// priority over the body so a small probe stays reshapeable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ProbeHit {
    Point(ProbeId),
    SegmentEndpoint(ProbeId, bool),
    SegmentBody(ProbeId),
    Boundary(ProbeId),
    AreaDiskBody(ProbeId),
    AreaDiskRadius(ProbeId),
    AreaRegion(ProbeId),
}

impl ProbeHit {
    pub(super) const fn id(self) -> ProbeId {
        match self {
            Self::Point(id)
            | Self::SegmentEndpoint(id, _)
            | Self::SegmentBody(id)
            | Self::Boundary(id)
            | Self::AreaDiskBody(id)
            | Self::AreaDiskRadius(id)
            | Self::AreaRegion(id) => id,
        }
    }

    pub(super) const fn draggable(self) -> bool {
        !matches!(self, Self::Boundary(_) | Self::AreaRegion(_))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum TransformGizmoHit {
    Pivot,
    Rotate,
    Scale(GizmoScaleAxis),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum GizmoScaleAxis {
    Uniform,
    X,
    Y,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum MarqueeOperation {
    Replace,
    Add,
    Subtract,
}

impl MarqueeOperation {
    pub(super) const fn label(self) -> &'static str {
        match self {
            Self::Replace => "Replace",
            Self::Add => "Add",
            Self::Subtract => "Subtract",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum MarqueeContainment {
    Enclosed,
    Crossing,
}

impl MarqueeContainment {
    pub(super) const fn from_drag(start: Pos2, current: Pos2) -> Self {
        if current.x >= start.x {
            Self::Enclosed
        } else {
            Self::Crossing
        }
    }

    pub(super) const fn label(self) -> &'static str {
        match self {
            Self::Enclosed => "Enclosed",
            Self::Crossing => "Crossing",
        }
    }
}
