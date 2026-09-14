//! Stable, topology-native viewport selection and manipulation.
//!
//! The visible UI supplies screen coordinates and paints the returned sampled
//! geometry. This module owns the interaction semantics so they can be tested
//! without Bevy or egui and so snapshot-local face IDs never become selection
//! identities.

use funfern_core::*;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ScreenPoint {
    pub x: f64,
    pub y: f64,
}

impl ScreenPoint {
    pub const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }

    fn distance(self, other: Self) -> f64 {
        (self.x - other.x).hypot(self.y - other.y)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViewportTransform {
    pub screen_center: ScreenPoint,
    pub world_center: Point2,
    pub pixels_per_world: f64,
}

impl ViewportTransform {
    pub fn world_to_screen(self, point: Point2) -> ScreenPoint {
        ScreenPoint::new(
            self.screen_center.x + (point.x - self.world_center.x) * self.pixels_per_world,
            self.screen_center.y - (point.y - self.world_center.y) * self.pixels_per_world,
        )
    }

    pub fn screen_to_world(self, point: ScreenPoint) -> Point2 {
        Point2::new(
            self.world_center.x + (point.x - self.screen_center.x) / self.pixels_per_world,
            self.world_center.y - (point.y - self.screen_center.y) / self.pixels_per_world,
        )
    }

    pub fn valid(self) -> bool {
        self.screen_center.x.is_finite()
            && self.screen_center.y.is_finite()
            && self.world_center.finite()
            && self.pixels_per_world.is_finite()
            && self.pixels_per_world > 0.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TopologySpanTarget {
    Outer(OuterSide),
    Curve(CurveSpanId),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TopologyHandle {
    Control { curve: CurveId, control: usize },
    Junction(TopologyVertexId),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum TopologySelection {
    #[default]
    None,
    Handle(TopologyHandle),
    Spans(BTreeSet<TopologySpanTarget>),
}

impl TopologySelection {
    pub fn spans(&self) -> Option<&BTreeSet<TopologySpanTarget>> {
        match self {
            Self::Spans(spans) => Some(spans),
            _ => None,
        }
    }

    pub fn select_handle(&mut self, handle: TopologyHandle) {
        *self = Self::Handle(handle);
    }

    pub fn select_span(
        &mut self,
        geometry: &TopologyGeometry,
        target: TopologySpanTarget,
        shift: bool,
        whole_curve: bool,
    ) {
        let targets = if whole_curve {
            match target {
                TopologySpanTarget::Curve(span) => geometry
                    .curves
                    .iter()
                    .find(|curve| curve.spans.iter().any(|candidate| candidate.id == span))
                    .map(|curve| {
                        curve
                            .spans
                            .iter()
                            .map(|span| TopologySpanTarget::Curve(span.id))
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default(),
                TopologySpanTarget::Outer(_) => OuterSide::ALL
                    .into_iter()
                    .map(TopologySpanTarget::Outer)
                    .collect(),
            }
        } else {
            vec![target]
        };
        let mut selected = if shift {
            self.spans().cloned().unwrap_or_default()
        } else {
            BTreeSet::new()
        };
        for target in targets {
            if shift && !selected.insert(target) {
                selected.remove(&target);
            } else {
                selected.insert(target);
            }
        }
        *self = if selected.is_empty() {
            Self::None
        } else {
            Self::Spans(selected)
        };
    }

    pub fn select_incident_spans(&mut self, geometry: &TopologyGeometry, vertex: TopologyVertexId) {
        let mut selected = self.spans().cloned().unwrap_or_default();
        selected.extend(
            incident_spans(geometry, vertex)
                .into_iter()
                .map(TopologySpanTarget::Curve),
        );
        *self = if selected.is_empty() {
            Self::None
        } else {
            Self::Spans(selected)
        };
    }

    pub fn apply_hit(
        &mut self,
        geometry: &TopologyGeometry,
        hit: TopologyHit,
        shift: bool,
        whole_curve: bool,
    ) {
        match hit {
            TopologyHit::Handle { handle, .. } => self.select_handle(handle),
            TopologyHit::Span { target, .. } => {
                self.select_span(geometry, target, shift, whole_curve)
            }
        }
    }
}

#[derive(Clone, Debug)]
pub struct SampledTopologySpan {
    pub target: TopologySpanTarget,
    pub curve: Option<CurveId>,
    pub samples: Vec<Sample>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SampledTopologyHandle {
    pub handle: TopologyHandle,
    pub point: Point2,
}

#[derive(Clone, Debug, Default)]
pub struct SampledTopologyGeometry {
    pub spans: Vec<SampledTopologySpan>,
    pub handles: Vec<SampledTopologyHandle>,
}

impl SampledTopologyGeometry {
    pub fn new(
        geometry: &TopologyGeometry,
        transform: ViewportTransform,
        pixel_tolerance: f64,
    ) -> Result<Self, SamplingError> {
        if !transform.valid() || !pixel_tolerance.is_finite() || pixel_tolerance <= 0.0 {
            return Err(SamplingError::NonFinite);
        }
        let options = SamplingOptions {
            tolerance: pixel_tolerance / transform.pixels_per_world,
            max_depth: 16,
            max_points: 4096,
        };
        let mut result = Self::default();
        let domain = geometry.domain;
        let corners = [
            Point2::new(domain.min_x, domain.min_y),
            Point2::new(domain.max_x, domain.min_y),
            Point2::new(domain.max_x, domain.max_y),
            Point2::new(domain.min_x, domain.max_y),
        ];
        for (index, side) in OuterSide::ALL.into_iter().enumerate() {
            result.spans.push(SampledTopologySpan {
                target: TopologySpanTarget::Outer(side),
                curve: None,
                samples: vec![
                    Sample {
                        t: 0.0,
                        point: corners[index],
                    },
                    Sample {
                        t: 1.0,
                        point: corners[(index + 1) % 4],
                    },
                ],
            });
        }
        for curve in &geometry.curves {
            let samples = match &curve.spline {
                CurveSpline::Closed(spline) => sample(spline, options)?,
                CurveSpline::Open(spline) => sample_open(spline, options)?,
            };
            for (span_index, span) in curve.spans.iter().enumerate() {
                let [start, end] = curve.spline.span_bounds(span_index).unwrap();
                let mut span_samples = samples
                    .iter()
                    .copied()
                    .filter(|sample| sample.t >= start && sample.t <= end)
                    .collect::<Vec<_>>();
                if span_samples.first().is_none_or(|sample| sample.t > start) {
                    span_samples.insert(
                        0,
                        Sample {
                            t: start,
                            point: evaluate(&curve.spline, start),
                        },
                    );
                }
                if span_samples.last().is_none_or(|sample| sample.t < end) {
                    span_samples.push(Sample {
                        t: end,
                        point: evaluate(&curve.spline, end),
                    });
                }
                result.spans.push(SampledTopologySpan {
                    target: TopologySpanTarget::Curve(span.id),
                    curve: Some(curve.id),
                    samples: span_samples,
                });
            }
            let controls = match &curve.spline {
                CurveSpline::Closed(spline) => spline.controls(),
                CurveSpline::Open(spline) => spline.controls(),
            };
            result.handles.extend(
                controls
                    .iter()
                    .copied()
                    .enumerate()
                    .map(|(control, point)| SampledTopologyHandle {
                        handle: TopologyHandle::Control {
                            curve: curve.id,
                            control,
                        },
                        point,
                    }),
            );
        }
        // Junctions are appended after raw controls and win ties in hit testing.
        result
            .handles
            .extend(geometry.vertices.iter().filter_map(|vertex| {
                vertex.point(domain).map(|point| SampledTopologyHandle {
                    handle: TopologyHandle::Junction(vertex.id),
                    point,
                })
            }));
        Ok(result)
    }

    pub fn hit_test(
        &self,
        transform: ViewportTransform,
        pointer: ScreenPoint,
        handle_radius: f64,
        span_radius: f64,
    ) -> Option<TopologyHit> {
        let handle = self
            .handles
            .iter()
            .rev()
            .filter_map(|candidate| {
                let distance = transform.world_to_screen(candidate.point).distance(pointer);
                (distance <= handle_radius).then_some((candidate.handle, distance))
            })
            .min_by(|left, right| left.1.total_cmp(&right.1));
        if let Some((handle, distance)) = handle {
            return Some(TopologyHit::Handle { handle, distance });
        }
        self.spans
            .iter()
            .filter_map(|span| {
                span.samples
                    .windows(2)
                    .map(|segment| {
                        screen_segment_distance(
                            pointer,
                            transform.world_to_screen(segment[0].point),
                            transform.world_to_screen(segment[1].point),
                        )
                    })
                    .min_by(f64::total_cmp)
                    .map(|distance| (span.target, distance))
            })
            .filter(|(_, distance)| *distance <= span_radius)
            .min_by(|left, right| left.1.total_cmp(&right.1))
            .map(|(target, distance)| TopologyHit::Span { target, distance })
    }

    pub fn marquee_hits(
        &self,
        transform: ViewportTransform,
        start: ScreenPoint,
        end: ScreenPoint,
    ) -> BTreeSet<TopologySpanTarget> {
        let rect = ScreenRect::from_points(start, end);
        let fully_enclose = end.x >= start.x;
        self.spans
            .iter()
            .filter(|span| {
                if fully_enclose {
                    span.samples
                        .iter()
                        .all(|sample| rect.contains(transform.world_to_screen(sample.point)))
                } else {
                    span.samples.windows(2).any(|segment| {
                        rect.intersects_segment(
                            transform.world_to_screen(segment[0].point),
                            transform.world_to_screen(segment[1].point),
                        )
                    })
                }
            })
            .map(|span| span.target)
            .collect()
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TopologyHit {
    Handle {
        handle: TopologyHandle,
        distance: f64,
    },
    Span {
        target: TopologySpanTarget,
        distance: f64,
    },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RigidTransform {
    pub pivot: Point2,
    pub translation: Point2,
    pub rotation_radians: f64,
    pub scale: f64,
}

impl RigidTransform {
    pub fn apply(self, point: Point2) -> Point2 {
        let relative = (point - self.pivot) * self.scale;
        let (sin, cos) = self.rotation_radians.sin_cos();
        self.pivot
            + Point2::new(
                cos * relative.x - sin * relative.y,
                sin * relative.x + cos * relative.y,
            )
            + self.translation
    }

    pub fn valid(self) -> bool {
        self.pivot.finite()
            && self.translation.finite()
            && self.rotation_radians.is_finite()
            && self.scale.is_finite()
            && self.scale > 0.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TopologyTransformUpdate {
    Control {
        curve: CurveId,
        control: usize,
        point: Point2,
    },
    Vertex {
        vertex: TopologyVertexId,
        point: Point2,
    },
}

pub fn plan_handle_drag(
    geometry: &TopologyGeometry,
    handle: TopologyHandle,
    point: Point2,
) -> Result<TopologyTransformUpdate, TopologyTransformIssue> {
    if !point.finite() {
        return Err(TopologyTransformIssue::InvalidTransform);
    }
    match handle {
        TopologyHandle::Control { curve, control } => {
            let curve = geometry
                .curves
                .iter()
                .find(|candidate| candidate.id == curve)
                .ok_or(TopologyTransformIssue::EmptySelection)?;
            let count = match &curve.spline {
                CurveSpline::Closed(spline) => spline.controls().len(),
                CurveSpline::Open(spline) => spline.controls().len(),
            };
            if control >= count {
                return Err(TopologyTransformIssue::EmptySelection);
            }
            Ok(TopologyTransformUpdate::Control {
                curve: curve.id,
                control,
                point,
            })
        }
        TopologyHandle::Junction(vertex) => geometry
            .vertices
            .iter()
            .any(|candidate| candidate.id == vertex)
            .then_some(TopologyTransformUpdate::Vertex { vertex, point })
            .ok_or(TopologyTransformIssue::EmptySelection),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TopologyTransformIssue {
    EmptySelection,
    OuterBoundarySelected,
    NonC0Selection,
    PartialJunction {
        vertex: TopologyVertexId,
        missing: BTreeSet<CurveSpanId>,
    },
    InvalidTransform,
}

impl std::fmt::Display for TopologyTransformIssue {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PartialJunction { .. } => {
                formatter.write_str("Junction also belongs to unselected spans")
            }
            Self::EmptySelection => formatter.write_str("Select one or more curve spans"),
            Self::OuterBoundarySelected => {
                formatter.write_str("Outer boundary extents use their contextual controls")
            }
            Self::NonC0Selection => formatter
                .write_str("Selected span section must end at a corner or include the whole curve"),
            Self::InvalidTransform => formatter.write_str("Transform values are invalid"),
        }
    }
}

impl std::error::Error for TopologyTransformIssue {}

pub fn plan_rigid_transform(
    geometry: &TopologyGeometry,
    selected: &BTreeSet<TopologySpanTarget>,
    transform: RigidTransform,
) -> Result<Vec<TopologyTransformUpdate>, TopologyTransformIssue> {
    if !transform.valid() {
        return Err(TopologyTransformIssue::InvalidTransform);
    }
    if selected.is_empty() {
        return Err(TopologyTransformIssue::EmptySelection);
    }
    if selected
        .iter()
        .any(|target| matches!(target, TopologySpanTarget::Outer(_)))
    {
        return Err(TopologyTransformIssue::OuterBoundarySelected);
    }
    let selected_spans = selected
        .iter()
        .filter_map(|target| match target {
            TopologySpanTarget::Curve(span) => Some(*span),
            TopologySpanTarget::Outer(_) => None,
        })
        .collect::<BTreeSet<_>>();
    for vertex in &geometry.vertices {
        let incident = incident_spans(geometry, vertex.id);
        let touched = incident.intersection(&selected_spans).count();
        if touched > 0 && touched < incident.len() {
            return Err(TopologyTransformIssue::PartialJunction {
                vertex: vertex.id,
                missing: incident.difference(&selected_spans).copied().collect(),
            });
        }
    }

    let mut controls = BTreeSet::new();
    for curve in &geometry.curves {
        let chosen = curve
            .spans
            .iter()
            .map(|span| selected_spans.contains(&span.id))
            .collect::<Vec<_>>();
        if !chosen.iter().any(|value| *value) {
            continue;
        }
        let whole = chosen.iter().all(|value| *value);
        if !whole && !selection_is_c0_isolated(curve, &chosen) {
            return Err(TopologyTransformIssue::NonC0Selection);
        }
        for (index, chosen) in chosen.into_iter().enumerate() {
            if !chosen {
                continue;
            }
            let indices = match &curve.spline {
                CurveSpline::Closed(spline) => spline.span_control_indices(index),
                CurveSpline::Open(spline) => spline.span_control_indices(index),
            }
            .ok_or(TopologyTransformIssue::NonC0Selection)?;
            controls.extend(indices.map(|control| (curve.id, control)));
        }
    }
    let mut updates = Vec::new();
    for (curve_id, control) in controls {
        let curve = geometry
            .curves
            .iter()
            .find(|curve| curve.id == curve_id)
            .unwrap();
        let point = match &curve.spline {
            CurveSpline::Closed(spline) => spline.controls()[control],
            CurveSpline::Open(spline) => spline.controls()[control],
        };
        updates.push(TopologyTransformUpdate::Control {
            curve: curve_id,
            control,
            point: transform.apply(point),
        });
    }
    for vertex in &geometry.vertices {
        let incident = incident_spans(geometry, vertex.id);
        if !incident.is_empty() && incident.is_subset(&selected_spans) {
            updates.push(TopologyTransformUpdate::Vertex {
                vertex: vertex.id,
                point: transform.apply(vertex.point(geometry.domain).unwrap()),
            });
        }
    }
    Ok(updates)
}

/// Plans independent world-axis scaling around a pivot while retaining the same
/// topology eligibility rules as rigid/uniform transforms.
pub fn plan_axis_scale(
    geometry: &TopologyGeometry,
    selected: &BTreeSet<TopologySpanTarget>,
    pivot: Point2,
    scale_x: f64,
    scale_y: f64,
) -> Result<Vec<TopologyTransformUpdate>, TopologyTransformIssue> {
    if !pivot.finite()
        || !scale_x.is_finite()
        || !scale_y.is_finite()
        || scale_x <= 0.0
        || scale_y <= 0.0
    {
        return Err(TopologyTransformIssue::InvalidTransform);
    }
    let mut updates = plan_rigid_transform(
        geometry,
        selected,
        RigidTransform {
            pivot,
            translation: Point2::default(),
            rotation_radians: 0.0,
            scale: 1.0,
        },
    )?;
    for update in &mut updates {
        let point = match update {
            TopologyTransformUpdate::Control { point, .. }
            | TopologyTransformUpdate::Vertex { point, .. } => point,
        };
        let relative = *point - pivot;
        *point = pivot + Point2::new(relative.x * scale_x, relative.y * scale_y);
    }
    Ok(updates)
}

pub fn incident_spans(
    geometry: &TopologyGeometry,
    vertex: TopologyVertexId,
) -> BTreeSet<CurveSpanId> {
    let mut result = BTreeSet::new();
    for curve in &geometry.curves {
        let span_count = curve.spans.len();
        for (node_index, node) in curve.nodes.iter().enumerate() {
            if node.vertex != Some(vertex) {
                continue;
            }
            if curve.spline.is_open() {
                if node_index > 0 {
                    result.insert(curve.spans[node_index - 1].id);
                }
                if node_index < span_count {
                    result.insert(curve.spans[node_index].id);
                }
            } else {
                result.insert(curve.spans[(node_index + span_count - 1) % span_count].id);
                result.insert(curve.spans[node_index % span_count].id);
            }
        }
    }
    result
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TopologyTraceContext {
    pub regions: BTreeSet<RegionId>,
    pub active: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TopologySpanContext {
    pub curve: CurveId,
    pub span: CurveSpanId,
    pub behavior: SpanBehavior,
    pub left: TopologyTraceContext,
    pub right: TopologyTraceContext,
    pub direction_origin: Point2,
    pub direction: Point2,
}

/// Resolves contextual boundary controls from the current compiled draft. Left
/// and right always follow increasing curve parameter, regardless of screen
/// orientation or the order in which multiple spans were selected.
pub fn span_context(
    scene: &CompiledTopologyScene,
    span: CurveSpanId,
) -> Option<TopologySpanContext> {
    let curve = scene
        .geometry
        .curves
        .iter()
        .find(|curve| curve.spans.iter().any(|candidate| candidate.id == span))?;
    let span_index = curve
        .spans
        .iter()
        .position(|candidate| candidate.id == span)?;
    let [start, end] = curve.spline.span_bounds(span_index)?;
    let parameter = (start + end) * 0.5;
    let direction_origin = evaluate(&curve.spline, parameter);
    let derivative = match &curve.spline {
        CurveSpline::Closed(spline) => spline.derivative(parameter, 1),
        CurveSpline::Open(spline) => spline.derivative(parameter, 1),
    };
    let direction = if derivative.norm() > 0.0 {
        derivative / derivative.norm()
    } else {
        Point2::default()
    };
    let assignment = scene
        .assignments
        .iter()
        .map(|assignment| (assignment.face, assignment.region))
        .collect::<BTreeMap<_, _>>();
    let mut left = BTreeSet::new();
    let mut right = BTreeSet::new();
    for edge in scene.topology.span_edges(span) {
        if let Some(Some(region)) = assignment.get(&edge.left) {
            left.insert(*region);
        }
        if let Some(Some(region)) = assignment.get(&edge.right) {
            right.insert(*region);
        }
    }
    Some(TopologySpanContext {
        curve: curve.id,
        span,
        behavior: curve.spans[span_index].behavior,
        left: TopologyTraceContext {
            active: !left.is_empty(),
            regions: left,
        },
        right: TopologyTraceContext {
            active: !right.is_empty(),
            regions: right,
        },
        direction_origin,
        direction,
    })
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AttachmentHit {
    pub attachment: crate::topology_editor::TopologyAttachment,
    pub point: Point2,
    pub distance: f64,
}

/// Finds an authored junction or boundary point. Junctions win over coincident
/// edge hits, and `face` restricts separators to one active face.
pub fn hit_attachment(
    scene: &CompiledTopologyScene,
    transform: ViewportTransform,
    pointer: ScreenPoint,
    radius: f64,
    face: Option<FaceId>,
) -> Option<AttachmentHit> {
    hit_junction(scene, transform, pointer, radius, face)
        .or_else(|| hit_edge(scene, transform, pointer, radius, face, None))
}

/// Everything a loose end may be welded onto, in precedence order: an authored
/// junction, another loose end, a vertex-less breakpoint, then a compiled edge.
/// Point targets come from the authored `geometry`, which is current mid-drag
/// when the compiled scene is not. `exclude` is the dragged end: it never snaps
/// to itself, and its whole curve is skipped for breakpoints and edges because
/// self-attachment is not supported.
pub fn weld_hit(
    scene: &CompiledTopologyScene,
    geometry: &TopologyGeometry,
    transform: ViewportTransform,
    pointer: ScreenPoint,
    radius: f64,
    exclude: Option<(CurveId, usize)>,
    face: Option<FaceId>,
) -> Option<AttachmentHit> {
    // A curve may attach to itself, so only the dragged tip's own neighbourhood
    // is off limits: its end span always lies under the cursor. Every other
    // node and span of the same curve is a target like any other.
    let blocked = exclude.and_then(|(curve, node)| {
        let curve = geometry.curve(curve)?;
        let span = if node == 0 {
            curve.spans.first()?
        } else {
            curve.spans.last()?
        };
        Some(span.id)
    });
    hit_junction(scene, transform, pointer, radius, face)
        .or_else(|| hit_loose_end(scene, geometry, transform, pointer, radius, exclude, face))
        .or_else(|| hit_breakpoint(scene, geometry, transform, pointer, radius, face))
        .or_else(|| hit_edge(scene, transform, pointer, radius, face, blocked))
}

fn hit_loose_end(
    scene: &CompiledTopologyScene,
    geometry: &TopologyGeometry,
    transform: ViewportTransform,
    pointer: ScreenPoint,
    radius: f64,
    exclude: Option<(CurveId, usize)>,
    face: Option<FaceId>,
) -> Option<AttachmentHit> {
    geometry
        .curves
        .iter()
        .filter(|curve| curve.spline.is_open())
        .flat_map(|curve| {
            let last = curve.nodes.len() - 1;
            [(0usize, 0usize), (last, 1usize)]
                .into_iter()
                .filter_map(move |(node, endpoint)| {
                    if curve.nodes[node].vertex.is_some() || exclude == Some((curve.id, node)) {
                        return None;
                    }
                    let point = curve.spline.node_point(node)?;
                    let distance = transform.world_to_screen(point).distance(pointer);
                    if distance > radius {
                        return None;
                    }
                    if let Some(face) = face {
                        // A free tip is a slit inside one face, which its end
                        // span sees on both sides.
                        let span = if node == 0 {
                            curve.spans[0].id
                        } else {
                            curve.spans[curve.spans.len() - 1].id
                        };
                        let lies_in = scene
                            .topology
                            .edges
                            .iter()
                            .find(|edge| edge.source == CompiledEdgeSource::Curve(span))
                            .map(|edge| edge.left);
                        if lies_in != Some(face) {
                            return None;
                        }
                    }
                    Some(AttachmentHit {
                        attachment: crate::topology_editor::TopologyAttachment::LooseEnd {
                            curve: curve.id,
                            endpoint,
                        },
                        point,
                        distance,
                    })
                })
        })
        .min_by(|left, right| left.distance.total_cmp(&right.distance))
}

fn hit_breakpoint(
    scene: &CompiledTopologyScene,
    geometry: &TopologyGeometry,
    transform: ViewportTransform,
    pointer: ScreenPoint,
    radius: f64,
    face: Option<FaceId>,
) -> Option<AttachmentHit> {
    // Open-curve endpoints are skipped below, so a dragged tip can never pick
    // itself and its own curve needs no exclusion here.
    geometry
        .curves
        .iter()
        .flat_map(|curve| {
            let open = curve.spline.is_open();
            let count = curve.nodes.len();
            (0..count).filter_map(move |node| {
                if curve.nodes[node].vertex.is_some() || (open && (node == 0 || node + 1 == count))
                {
                    return None;
                }
                let point = curve.spline.node_point(node)?;
                let distance = transform.world_to_screen(point).distance(pointer);
                if distance > radius {
                    return None;
                }
                // The node owns no vertex, so the span starting at it sees the
                // same faces the node does.
                let [a, b] = curve.spline.span_bounds(node)?;
                let midpoint = (a + b) * 0.5;
                let span = curve.spans[node].id;
                let side_face = |side| {
                    FaceAnchor::Curve {
                        curve: curve.id,
                        span,
                        side,
                        parameter: midpoint,
                    }
                    .resolve(&scene.topology)
                    .ok()
                };
                let side = if let Some(face) = face {
                    [CurveTraceSide::Left, CurveTraceSide::Right]
                        .into_iter()
                        .find(|side| side_face(*side) == Some(face))?
                } else {
                    // World-space left of increasing parameter.
                    let tangent = evaluate(&curve.spline, midpoint) - point;
                    let toward = transform.screen_to_world(pointer) - point;
                    if tangent.x * toward.y - tangent.y * toward.x >= 0.0 {
                        CurveTraceSide::Left
                    } else {
                        CurveTraceSide::Right
                    }
                };
                Some(AttachmentHit {
                    attachment: crate::topology_editor::TopologyAttachment::Breakpoint {
                        curve: curve.id,
                        node,
                        side,
                    },
                    point,
                    distance,
                })
            })
        })
        .min_by(|left, right| left.distance.total_cmp(&right.distance))
}

fn hit_junction(
    scene: &CompiledTopologyScene,
    transform: ViewportTransform,
    pointer: ScreenPoint,
    radius: f64,
    face: Option<FaceId>,
) -> Option<AttachmentHit> {
    scene
        .topology
        .vertices
        .iter()
        .filter_map(|vertex| {
            let authored = vertex.authored?;
            let distance = transform.world_to_screen(vertex.point).distance(pointer);
            if distance > radius {
                return None;
            }
            let target_face = face.or_else(|| {
                vertex
                    .traces
                    .iter()
                    .find(|trace| {
                        scene.assignments.iter().any(|assignment| {
                            assignment.face == trace.face && assignment.region.is_some()
                        })
                    })
                    .or_else(|| vertex.traces.first())
                    .map(|trace| trace.face)
            })?;
            vertex
                .traces
                .iter()
                .any(|trace| trace.face == target_face)
                .then_some(AttachmentHit {
                    attachment: crate::topology_editor::TopologyAttachment::Junction {
                        vertex: authored,
                        face: target_face,
                    },
                    point: vertex.point,
                    distance,
                })
        })
        .min_by(|left, right| left.distance.total_cmp(&right.distance))
}

fn hit_edge(
    scene: &CompiledTopologyScene,
    transform: ViewportTransform,
    pointer: ScreenPoint,
    radius: f64,
    face: Option<FaceId>,
    exclude: Option<CurveSpanId>,
) -> Option<AttachmentHit> {
    scene
        .topology
        .edges
        .iter()
        .filter(
            |edge| !matches!(edge.source, CompiledEdgeSource::Curve(span) if Some(span) == exclude),
        )
        .filter_map(|edge| {
            let a = transform.world_to_screen(edge.points[0]);
            let b = transform.world_to_screen(edge.points[1]);
            let (distance, fraction) = screen_segment_projection(pointer, a, b);
            if distance > radius {
                return None;
            }
            match edge.source {
                CompiledEdgeSource::Outer(side) => {
                    if face.is_some_and(|face| face != edge.left) {
                        return None;
                    }
                    let parameter =
                        edge.parameter[0] + (edge.parameter[1] - edge.parameter[0]) * fraction;
                    Some(AttachmentHit {
                        attachment: crate::topology_editor::TopologyAttachment::Boundary(
                            FaceAnchor::Outer {
                                side,
                                fraction: parameter,
                            },
                        ),
                        point: edge.points[0].lerp(edge.points[1], fraction),
                        distance,
                    })
                }
                CompiledEdgeSource::Curve(span) => {
                    let curve = edge.curve?;
                    let (side, target_face) = if let Some(face) = face {
                        if face == edge.right {
                            (CurveTraceSide::Right, face)
                        } else if face == edge.left {
                            (CurveTraceSide::Left, face)
                        } else {
                            return None;
                        }
                    } else {
                        // World-space left becomes the negative cross-product
                        // side after the viewport's vertical-axis flip.
                        let cross =
                            (b.x - a.x) * (pointer.y - a.y) - (b.y - a.y) * (pointer.x - a.x);
                        if cross <= 0.0 {
                            (CurveTraceSide::Left, edge.left)
                        } else {
                            (CurveTraceSide::Right, edge.right)
                        }
                    };
                    let parameter =
                        edge.parameter[0] + (edge.parameter[1] - edge.parameter[0]) * fraction;
                    let attachment = FaceAnchor::Curve {
                        curve,
                        span,
                        side,
                        parameter,
                    };
                    (attachment.resolve(&scene.topology).ok() == Some(target_face)).then_some(
                        AttachmentHit {
                            attachment: crate::topology_editor::TopologyAttachment::Boundary(
                                attachment,
                            ),
                            point: edge.points[0].lerp(edge.points[1], fraction),
                            distance,
                        },
                    )
                }
            }
        })
        .min_by(|left, right| left.distance.total_cmp(&right.distance))
}

/// The controls that shape the selected curve spans, as `(curve, control)`
/// pairs: a control belongs to a selection only through a span it supports, so
/// picking one span of a long curve lights four controls, not the whole curve.
pub fn selected_span_controls(
    geometry: &TopologyGeometry,
    selected: &BTreeSet<TopologySpanTarget>,
) -> BTreeSet<(CurveId, usize)> {
    selected
        .iter()
        .filter_map(|target| match target {
            TopologySpanTarget::Curve(span) => Some(*span),
            TopologySpanTarget::Outer(_) => None,
        })
        .filter_map(|span| {
            geometry.curves.iter().find_map(|curve| {
                curve
                    .spans
                    .iter()
                    .position(|candidate| candidate.id == span)
                    .map(|index| (curve, index))
            })
        })
        .filter_map(|(curve, index)| {
            match &curve.spline {
                CurveSpline::Closed(spline) => spline.span_control_indices(index),
                CurveSpline::Open(spline) => spline.span_control_indices(index),
            }
            .map(|controls| (curve.id, controls))
        })
        .flat_map(|(curve, controls)| controls.into_iter().map(move |control| (curve, control)))
        .collect()
}

fn evaluate(spline: &CurveSpline, parameter: f64) -> Point2 {
    match spline {
        CurveSpline::Closed(spline) => spline.evaluate(parameter),
        CurveSpline::Open(spline) => spline.evaluate(parameter),
    }
}

fn selection_is_c0_isolated(curve: &TopologyCurve, selected: &[bool]) -> bool {
    let count = selected.len();
    for index in 0..count {
        let previous = if index == 0 {
            (!curve.spline.is_open()).then_some(count - 1)
        } else {
            Some(index - 1)
        };
        if previous.is_some_and(|previous| selected[previous] != selected[index]) {
            let continuity = match &curve.spline {
                CurveSpline::Closed(spline) => spline.continuity(index),
                CurveSpline::Open(spline) => spline.continuity(index),
            };
            if continuity != Some(0) {
                return false;
            }
        }
    }
    if curve.spline.is_open() {
        for breakpoint in 1..count {
            if selected[breakpoint - 1] != selected[breakpoint] {
                let continuity = match &curve.spline {
                    CurveSpline::Open(spline) => spline.continuity(breakpoint),
                    CurveSpline::Closed(_) => unreachable!(),
                };
                if continuity != Some(0) {
                    return false;
                }
            }
        }
    }
    true
}

#[derive(Clone, Copy)]
struct ScreenRect {
    min: ScreenPoint,
    max: ScreenPoint,
}

impl ScreenRect {
    fn from_points(a: ScreenPoint, b: ScreenPoint) -> Self {
        Self {
            min: ScreenPoint::new(a.x.min(b.x), a.y.min(b.y)),
            max: ScreenPoint::new(a.x.max(b.x), a.y.max(b.y)),
        }
    }

    fn contains(self, point: ScreenPoint) -> bool {
        point.x >= self.min.x
            && point.x <= self.max.x
            && point.y >= self.min.y
            && point.y <= self.max.y
    }

    fn intersects_segment(self, a: ScreenPoint, b: ScreenPoint) -> bool {
        if self.contains(a) || self.contains(b) {
            return true;
        }
        let corners = [
            self.min,
            ScreenPoint::new(self.max.x, self.min.y),
            self.max,
            ScreenPoint::new(self.min.x, self.max.y),
        ];
        (0..4).any(|index| segments_intersect(a, b, corners[index], corners[(index + 1) % 4]))
    }
}

fn screen_segment_distance(point: ScreenPoint, a: ScreenPoint, b: ScreenPoint) -> f64 {
    screen_segment_projection(point, a, b).0
}

fn screen_segment_projection(point: ScreenPoint, a: ScreenPoint, b: ScreenPoint) -> (f64, f64) {
    let ab = ScreenPoint::new(b.x - a.x, b.y - a.y);
    let length_squared = ab.x * ab.x + ab.y * ab.y;
    let fraction = if length_squared == 0.0 {
        0.0
    } else {
        (((point.x - a.x) * ab.x + (point.y - a.y) * ab.y) / length_squared).clamp(0.0, 1.0)
    };
    let projected = ScreenPoint::new(a.x + ab.x * fraction, a.y + ab.y * fraction);
    (point.distance(projected), fraction)
}

fn segments_intersect(a: ScreenPoint, b: ScreenPoint, c: ScreenPoint, d: ScreenPoint) -> bool {
    fn cross(a: ScreenPoint, b: ScreenPoint, c: ScreenPoint) -> f64 {
        (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x)
    }
    let bounds_overlap = a.x.min(b.x) <= c.x.max(d.x)
        && c.x.min(d.x) <= a.x.max(b.x)
        && a.y.min(b.y) <= c.y.max(d.y)
        && c.y.min(d.y) <= a.y.max(b.y);
    if !bounds_overlap {
        return false;
    }
    let ab_c = cross(a, b, c);
    let ab_d = cross(a, b, d);
    let cd_a = cross(c, d, a);
    let cd_b = cross(c, d, b);
    ab_c * ab_d <= 0.0 && cd_a * cd_b <= 0.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view() -> ViewportTransform {
        ViewportTransform {
            screen_center: ScreenPoint::new(100.0, 100.0),
            world_center: Point2::default(),
            pixels_per_world: 100.0,
        }
    }

    fn open_curve(id: u64, span: u64, points: &[(f64, f64)]) -> TopologyCurve {
        let spline =
            OpenCubicSpline::polyline(points.iter().map(|&(x, y)| Point2::new(x, y)).collect())
                .unwrap();
        TopologyCurve::new(
            CurveId(id),
            CurveSpline::Open(spline),
            (0..points.len() - 1)
                .map(|offset| CurveSpan {
                    id: CurveSpanId(span + offset as u64),
                    behavior: SpanBehavior::REFLECTING,
                })
                .collect(),
        )
        .unwrap()
    }

    #[test]
    fn junction_handle_wins_over_curve_and_control() {
        let mut curve = open_curve(1, 1, &[(-0.5, 0.0), (0.0, 0.0)]);
        curve.nodes[1].vertex = Some(TopologyVertexId(1));
        let geometry = TopologyGeometry {
            domain: DomainRect::UNIT,
            curves: vec![curve],
            vertices: vec![TopologyVertex {
                id: TopologyVertexId(1),
                location: TopologyVertexLocation::Interior(Point2::default()),
            }],
        };
        let sampled = SampledTopologyGeometry::new(&geometry, view(), 0.25).unwrap();
        assert_eq!(
            sampled.hit_test(view(), ScreenPoint::new(100.0, 100.0), 8.0, 8.0),
            Some(TopologyHit::Handle {
                handle: TopologyHandle::Junction(TopologyVertexId(1)),
                distance: 0.0,
            })
        );
    }

    #[test]
    fn span_click_supports_toggle_and_whole_curve() {
        let geometry = TopologyGeometry {
            domain: DomainRect::UNIT,
            curves: vec![open_curve(1, 10, &[(-0.8, 0.0), (0.0, 0.0), (0.8, 0.0)])],
            vertices: vec![],
        };
        let mut selection = TopologySelection::None;
        selection.select_span(
            &geometry,
            TopologySpanTarget::Curve(CurveSpanId(10)),
            false,
            true,
        );
        assert_eq!(selection.spans().unwrap().len(), 2);
        selection.select_span(
            &geometry,
            TopologySpanTarget::Curve(CurveSpanId(10)),
            true,
            false,
        );
        assert_eq!(
            selection.spans().unwrap(),
            &BTreeSet::from([TopologySpanTarget::Curve(CurveSpanId(11))])
        );
    }

    #[test]
    fn marquee_direction_switches_enclosure_and_crossing() {
        let geometry = TopologyGeometry {
            domain: DomainRect::UNIT,
            curves: vec![open_curve(1, 10, &[(-0.8, 0.0), (0.8, 0.0)])],
            vertices: vec![],
        };
        let sampled = SampledTopologyGeometry::new(&geometry, view(), 0.25).unwrap();
        let left_to_right = sampled.marquee_hits(
            view(),
            ScreenPoint::new(90.0, 90.0),
            ScreenPoint::new(110.0, 110.0),
        );
        let right_to_left = sampled.marquee_hits(
            view(),
            ScreenPoint::new(110.0, 90.0),
            ScreenPoint::new(90.0, 110.0),
        );
        assert!(!left_to_right.contains(&TopologySpanTarget::Curve(CurveSpanId(10))));
        assert!(right_to_left.contains(&TopologySpanTarget::Curve(CurveSpanId(10))));
    }

    #[test]
    fn partial_junction_blocks_rigid_transform_and_can_expand_selection() {
        let vertex = TopologyVertexId(1);
        let mut left = open_curve(1, 10, &[(-0.8, 0.0), (0.0, 0.0)]);
        left.nodes[1].vertex = Some(vertex);
        let mut right = open_curve(2, 20, &[(0.0, 0.0), (0.8, 0.0)]);
        right.nodes[0].vertex = Some(vertex);
        let geometry = TopologyGeometry {
            domain: DomainRect::UNIT,
            curves: vec![left, right],
            vertices: vec![TopologyVertex {
                id: vertex,
                location: TopologyVertexLocation::Interior(Point2::default()),
            }],
        };
        let transform = RigidTransform {
            pivot: Point2::default(),
            translation: Point2::new(0.0, 0.2),
            rotation_radians: 0.0,
            scale: 1.0,
        };
        let selected = BTreeSet::from([TopologySpanTarget::Curve(CurveSpanId(10))]);
        assert_eq!(
            plan_rigid_transform(&geometry, &selected, transform),
            Err(TopologyTransformIssue::PartialJunction {
                vertex,
                missing: BTreeSet::from([CurveSpanId(20)]),
            })
        );
        let mut selection = TopologySelection::Spans(selected);
        selection.select_incident_spans(&geometry, vertex);
        let updates =
            plan_rigid_transform(&geometry, selection.spans().unwrap(), transform).unwrap();
        assert_eq!(
            updates
                .iter()
                .filter(|update| matches!(update, TopologyTransformUpdate::Vertex { .. }))
                .count(),
            1
        );
    }

    #[test]
    fn axis_scale_uses_the_rigid_selection_contract() {
        let geometry = TopologyGeometry {
            curves: vec![open_curve(1, 10, &[(-0.5, -0.25), (0.5, 0.25)])],
            ..TopologyGeometry::default()
        };
        let selected = BTreeSet::from([TopologySpanTarget::Curve(CurveSpanId(10))]);
        let updates = plan_axis_scale(&geometry, &selected, Point2::default(), 2.0, 0.5).unwrap();
        let points = updates
            .iter()
            .filter_map(|update| match update {
                TopologyTransformUpdate::Control { point, .. } => Some(*point),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(points.first().copied(), Some(Point2::new(-1.0, -0.125)));
        assert_eq!(points.last().copied(), Some(Point2::new(1.0, 0.125)));
    }

    #[test]
    fn attachment_hit_respects_requested_face_side() {
        let scene = TopologyScene::default().compile(1).unwrap();
        let inside = scene.assignments[0].face;
        let hit = hit_attachment(
            &scene,
            view(),
            ScreenPoint::new(100.0, 200.0),
            5.0,
            Some(inside),
        )
        .unwrap();
        assert!(matches!(
            hit.attachment,
            crate::topology_editor::TopologyAttachment::Boundary(FaceAnchor::Outer {
                side: OuterSide::Bottom,
                fraction
            }) if (fraction - 0.5).abs() < 1.0e-12
        ));
        assert!(
            hit_attachment(
                &scene,
                view(),
                ScreenPoint::new(100.0, 200.0),
                5.0,
                Some(EXTERIOR_FACE),
            )
            .is_none()
        );
    }

    #[test]
    fn span_context_keeps_parameter_relative_sides_and_active_faces() {
        let mut editor = crate::topology_editor::TopologyEditor::default();
        let curve = editor
            .create_closed_curve(
                PeriodicCubicSpline::rounded(Point2::default(), 0.25),
                crate::topology_editor::ClosedCurvePurpose::Hole,
            )
            .unwrap();
        for _ in 0..100_000 {
            editor.validate_frame(64);
            if editor.acceptance != crate::topology_editor::TopologyAcceptance::Pending {
                break;
            }
        }
        let authored = editor
            .document
            .model
            .draft
            .geometry
            .curves
            .iter()
            .find(|candidate| candidate.id == curve)
            .unwrap();
        let context = span_context(&editor.compiled_accepted, authored.spans[0].id).unwrap();
        assert_eq!(context.curve, curve);
        assert_eq!(context.behavior, SpanBehavior::REFLECTING);
        assert_ne!(context.left.active, context.right.active);
        assert!((context.direction.norm() - 1.0).abs() < 1.0e-12);
    }

    #[test]
    fn attachment_hit_finds_an_inner_curve_on_its_active_face() {
        let mut editor = crate::topology_editor::TopologyEditor::default();
        let curve = editor
            .create_closed_curve(
                PeriodicCubicSpline::rounded(Point2::default(), 0.25),
                crate::topology_editor::ClosedCurvePurpose::Hole,
            )
            .unwrap();
        for _ in 0..100_000 {
            editor.validate_frame(64);
            if editor.acceptance != crate::topology_editor::TopologyAcceptance::Pending {
                break;
            }
        }
        let compiled = &editor.compiled_accepted;
        let active_faces = compiled
            .assignments
            .iter()
            .filter_map(|assignment| assignment.region.map(|_| assignment.face))
            .collect::<BTreeSet<_>>();
        let edge = compiled
            .topology
            .edges
            .iter()
            .find(|edge| edge.curve == Some(curve))
            .unwrap();
        let face = [edge.left, edge.right]
            .into_iter()
            .find(|face| active_faces.contains(face))
            .unwrap();
        let point = edge.points[0].lerp(edge.points[1], 0.5);
        let hit = hit_attachment(
            compiled,
            view(),
            view().world_to_screen(point),
            5.0,
            Some(face),
        )
        .unwrap();
        assert!(matches!(
            hit.attachment,
            crate::topology_editor::TopologyAttachment::Boundary(FaceAnchor::Curve {
                curve: candidate,
                ..
            }) if candidate == curve
        ));

        for scale in [100.0, 420.0] {
            let transform = ViewportTransform {
                pixels_per_world: scale,
                ..view()
            };
            let a = transform.world_to_screen(edge.points[0]);
            let b = transform.world_to_screen(edge.points[1]);
            let tangent = ScreenPoint::new(b.x - a.x, b.y - a.y);
            let length = tangent.x.hypot(tangent.y);
            let normal = if face == edge.left {
                ScreenPoint::new(tangent.y / length, -tangent.x / length)
            } else {
                ScreenPoint::new(-tangent.y / length, tangent.x / length)
            };
            let midpoint = transform.world_to_screen(point);
            let pointer =
                ScreenPoint::new(midpoint.x + normal.x * 3.0, midpoint.y + normal.y * 3.0);
            let unspecialized = hit_attachment(compiled, transform, pointer, 5.0, None).unwrap();
            let resolved = match unspecialized.attachment {
                crate::topology_editor::TopologyAttachment::Boundary(anchor) => {
                    anchor.resolve(&compiled.topology).unwrap()
                }
                crate::topology_editor::TopologyAttachment::Junction { face, .. } => face,
                crate::topology_editor::TopologyAttachment::LooseEnd { .. }
                | crate::topology_editor::TopologyAttachment::Breakpoint { .. } => {
                    panic!("edge hits never resolve to loose ends or breakpoints")
                }
            };
            assert_eq!(resolved, face);
            assert!((unspecialized.distance - 3.0).abs() < 1.0e-9);
        }
    }

    /// One selected span lights exactly its four supporting controls; two
    /// adjacent spans share one and light five.
    #[test]
    fn selected_span_controls_follow_span_support_not_the_curve() {
        let spline = PeriodicCubicSpline::rounded(Point2::default(), 0.4);
        let spans = (1..=8)
            .map(|index| CurveSpan {
                id: CurveSpanId(index),
                behavior: SpanBehavior::Transmitting,
            })
            .collect();
        let curve = TopologyCurve::new(CurveId(1), CurveSpline::Closed(spline), spans).unwrap();
        let geometry = TopologyGeometry {
            curves: vec![curve],
            ..TopologyGeometry::default()
        };
        let one = [TopologySpanTarget::Curve(CurveSpanId(3))]
            .into_iter()
            .collect::<BTreeSet<_>>();
        let controls = selected_span_controls(&geometry, &one);
        assert_eq!(controls.len(), 4);
        assert!(controls.iter().all(|(curve, _)| *curve == CurveId(1)));
        let two = [
            TopologySpanTarget::Curve(CurveSpanId(3)),
            TopologySpanTarget::Curve(CurveSpanId(4)),
        ]
        .into_iter()
        .collect::<BTreeSet<_>>();
        assert_eq!(selected_span_controls(&geometry, &two).len(), 5);
        let outer = [TopologySpanTarget::Outer(OuterSide::Bottom)]
            .into_iter()
            .collect::<BTreeSet<_>>();
        assert!(selected_span_controls(&geometry, &outer).is_empty());
    }
}
