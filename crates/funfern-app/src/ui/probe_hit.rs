//! Finding and moving probes in the viewport: hit tolerance, markers and
//! badges, drag handling, and the metadata each probe carries.

use crate::wave_gpu::PointProbeRecord;
use bevy::prelude::*;
use bevy_egui::egui::{self, Color32, Pos2, Rect};
use funfern_app::document::ProbeId;
use funfern_app::topology_editor::{
    TopologyBoundaryProbeTarget, TopologyProbeDefinition, TopologyProbeTarget,
};
use funfern_app::topology_runtime::{TopologyProbeCompilation, TopologyProbeStencil};
use funfern_app::topology_viewport::TopologySpanTarget;
use funfern_core::*;
use std::collections::{BTreeMap, BTreeSet};

use super::*;

impl Playground {
    pub(super) fn hit_tolerance(&self, mouse: f32) -> f32 {
        if self.touch_active {
            mouse.max(18.0)
        } else {
            mouse
        }
    }
    pub(super) fn probe_visible(&self, target: &TopologyProbeTarget) -> bool {
        let presentation = self.editor.document.presentation;
        match target {
            TopologyProbeTarget::Point(_) => presentation.point_probes,
            TopologyProbeTarget::Segment { .. } => presentation.line_probes,
            TopologyProbeTarget::Boundary(_) => presentation.boundary_probes,
            TopologyProbeTarget::AreaDisk { .. } | TopologyProbeTarget::AreaRegion(_) => {
                presentation.area_probes
            }
        }
    }
    /// Colour rule shared by markers and labels: a probe that failed to compile
    /// reads red, one that is not recording reads grey.
    pub(super) fn probe_color(&self, probe: &TopologyProbeDefinition) -> Color32 {
        if self.probe_status.contains_key(&probe.id) {
            RED
        } else if !probe.enabled {
            Color32::from_rgb(112, 130, 143)
        } else {
            Color32::from_rgb(probe.color[0], probe.color[1], probe.color[2])
        }
    }
    /// The drawn path of a boundary probe, in the target's own span order.
    pub(super) fn boundary_probe_polyline(
        &self,
        target: &TopologyBoundaryProbeTarget,
    ) -> Vec<Point2> {
        let Some(sampled) = &self.sampled else {
            return vec![];
        };
        let mut path = Vec::new();
        for span in &target.spans {
            let Some(samples) = sampled
                .spans
                .iter()
                .find(|candidate| candidate.target == TopologySpanTarget::Curve(*span))
            else {
                continue;
            };
            for sample in &samples.samples {
                if path.last() != Some(&sample.point) {
                    path.push(sample.point);
                }
            }
        }
        path
    }
    /// Point a fraction of the way along a polyline by arclength, with the
    /// segment carrying it - which is what orients anything drawn there.
    pub(super) fn polyline_anchor(path: &[Point2], fraction: f64) -> Option<(Point2, [Point2; 2])> {
        let total: f64 = path.windows(2).map(|pair| (pair[1] - pair[0]).norm()).sum();
        let mut remaining = total * fraction;
        for pair in path.windows(2) {
            let length = (pair[1] - pair[0]).norm();
            if remaining <= length {
                let fraction = if length > 0.0 {
                    remaining / length
                } else {
                    0.0
                };
                return Some((pair[0].lerp(pair[1], fraction), [pair[0], pair[1]]));
            }
            remaining -= length;
        }
        None
    }
    /// Point half way along a polyline by arclength, used to badge and hit the
    /// probe where the eye expects it rather than at an arbitrary end.
    pub(super) fn polyline_midpoint(path: &[Point2]) -> Option<Point2> {
        Self::polyline_anchor(path, 0.5)
            .map(|(point, _)| point)
            .or_else(|| path.first().copied())
    }
    pub(super) fn probe_badge(&self, probe: &TopologyProbeDefinition) -> Option<Point2> {
        match &probe.target {
            TopologyProbeTarget::Point(point) => Some(*point),
            TopologyProbeTarget::Segment { start, end, .. } => Some(start.lerp(*end, 0.5)),
            TopologyProbeTarget::Boundary(target) => {
                Self::polyline_midpoint(&self.boundary_probe_polyline(target))
            }
            TopologyProbeTarget::AreaDisk { center, .. } => Some(*center),
            TopologyProbeTarget::AreaRegion(_) => self.probe_anchors.get(&probe.id).copied(),
        }
    }
    /// Topmost probe under the pointer. Later probes win, matching the draw order.
    pub(super) fn hit_probe(&self, point: Pos2, viewport: Rect) -> Option<ProbeHit> {
        self.editor
            .document
            .model
            .probes
            .iter()
            .rev()
            .filter(|probe| self.probe_visible(&probe.target))
            .find_map(|probe| match &probe.target {
                TopologyProbeTarget::Point(position) => {
                    (self.screen(*position, viewport).distance(point) <= self.hit_tolerance(13.0))
                        .then_some(ProbeHit::Point(probe.id))
                }
                TopologyProbeTarget::Segment { start, end, .. } => {
                    let a = self.screen(*start, viewport);
                    let b = self.screen(*end, viewport);
                    if a.distance(point) <= self.hit_tolerance(13.0) {
                        Some(ProbeHit::SegmentEndpoint(probe.id, true))
                    } else if b.distance(point) <= self.hit_tolerance(13.0) {
                        Some(ProbeHit::SegmentEndpoint(probe.id, false))
                    } else if screen_segment_distance(point, a, b) <= self.hit_tolerance(9.0) {
                        Some(ProbeHit::SegmentBody(probe.id))
                    } else {
                        None
                    }
                }
                TopologyProbeTarget::Boundary(target) => {
                    let badge = Self::polyline_midpoint(&self.boundary_probe_polyline(target))?;
                    (self.screen(badge, viewport).distance(point) <= self.hit_tolerance(13.0))
                        .then_some(ProbeHit::Boundary(probe.id))
                }
                TopologyProbeTarget::AreaDisk { center, radius } => {
                    let center = self.screen(*center, viewport);
                    let radius = (radius * self.scale) as f32;
                    let handle = center + egui::vec2(radius, 0.0);
                    if handle.distance(point) <= self.hit_tolerance(13.0) {
                        Some(ProbeHit::AreaDiskRadius(probe.id))
                    } else if center.distance(point) <= radius + self.hit_tolerance(9.0) {
                        Some(ProbeHit::AreaDiskBody(probe.id))
                    } else {
                        None
                    }
                }
                TopologyProbeTarget::AreaRegion(_) => {
                    let anchor = self.probe_anchors.get(&probe.id).copied()?;
                    (self.screen(anchor, viewport).distance(point) <= self.hit_tolerance(15.0))
                        .then_some(ProbeHit::AreaRegion(probe.id))
                }
            })
    }
    /// Applies a probe drag to the target captured when the gesture started, so
    /// repeated updates stay exact instead of accumulating rounding.
    /// The radius a second placement click asks for. Snapping puts the radius
    /// itself on the grid rather than the point it was measured to, which is
    /// what dragging a disk's rim already does - snapping the rim point would
    /// leave a radius that is no multiple of anything.
    pub(super) fn placed_disk_radius(center: Point2, point: Point2, snap: bool, step: f64) -> f64 {
        let radius = (point - center).norm();
        if snap {
            // A click inside the first grid step would round the disk away.
            Self::snap_scalar(radius, step).max(step)
        } else {
            radius
        }
    }
    pub(super) fn snap_scalar(value: f64, step: f64) -> f64 {
        (value / step).round() * step
    }

    pub(super) fn drag_probe(
        &mut self,
        hit: ProbeHit,
        original: &TopologyProbeTarget,
        delta: Point2,
        snap: bool,
    ) {
        let step = self.snap_step();
        let place = |point: Point2| {
            if snap {
                Self::snap_point(point + delta, step)
            } else {
                point + delta
            }
        };
        let Some(mut probe) = self
            .editor
            .document
            .model
            .probes
            .iter()
            .find(|probe| probe.id == hit.id())
            .cloned()
        else {
            return;
        };
        probe.target = original.clone();
        match (hit, &mut probe.target) {
            (ProbeHit::Point(_), TopologyProbeTarget::Point(position)) => {
                *position = place(*position);
            }
            (
                ProbeHit::SegmentEndpoint(_, first),
                TopologyProbeTarget::Segment { start, end, .. },
            ) => {
                let endpoint = if first { start } else { end };
                *endpoint = place(*endpoint);
            }
            (ProbeHit::SegmentBody(_), TopologyProbeTarget::Segment { start, end, .. }) => {
                // Translate rigidly, putting the start on the grid.
                let shift = place(*start) - *start;
                *start = *start + shift;
                *end = *end + shift;
            }
            (ProbeHit::AreaDiskBody(_), TopologyProbeTarget::AreaDisk { center, .. }) => {
                *center = place(*center);
            }
            (ProbeHit::AreaDiskRadius(_), TopologyProbeTarget::AreaDisk { center, radius }) => {
                let grown = *radius + delta.x;
                *radius = if snap {
                    Self::snap_scalar(grown, step)
                } else {
                    grown
                }
                .max(1.0e-3);
                let _ = center;
            }
            _ => return,
        }
        if let Err(error) = self.editor.update_probe_during_edit(probe) {
            self.message = error;
        }
    }
    /// Mirrors the committed compilation status and path metrics of every probe
    /// so the readouts can report a precise reason and a real arclength axis,
    /// and places each subdomain probe's marker from the compiled geometry.
    pub(super) fn refresh_probe_metadata(&mut self) {
        let ids = self
            .editor
            .document
            .model
            .probes
            .iter()
            .map(|probe| probe.id)
            .collect::<BTreeSet<_>>();
        self.probe_status.retain(|id, _| ids.contains(id));
        self.probe_metrics.retain(|id, _| ids.contains(id));
        self.probe_traces.retain(|id, _| ids.contains(id));
        self.curve_probe_traces.retain(|id, _| ids.contains(id));
        self.area_probe_traces.retain(|id, _| ids.contains(id));
        self.probe_views.retain(|id, _| ids.contains(id));
        self.probe_windows.retain(|id| ids.contains(id));
        self.probe_anchors.retain(|id, _| ids.contains(id));
        // Compilation status and path metrics mirror a committed candidate, so
        // they change only when one is published or the probe set moves. Region
        // anchors come from the compiled geometry instead, which is why the
        // token survives having no candidate at all.
        let token = (
            self.runtime.active().map(|active| active.bundle.token),
            self.editor.revision,
        );
        if self.probe_metadata_token == Some(token) {
            return;
        }
        self.probe_metadata_token = Some(token);
        let scene = self
            .editor
            .compiled_draft
            .as_ref()
            .unwrap_or(&self.editor.compiled_accepted);
        self.probe_anchors = self
            .editor
            .document
            .model
            .probes
            .iter()
            .filter_map(|probe| match probe.target {
                TopologyProbeTarget::AreaRegion(region) => {
                    Some((probe.id, region_anchor(scene, region)?))
                }
                _ => None,
            })
            .collect::<BTreeMap<_, _>>();
        let Some(active) = self.runtime.active().cloned() else {
            return;
        };
        for compiled in active.probes.iter() {
            match &compiled.result {
                TopologyProbeCompilation::Ready(stencil) => {
                    self.probe_status.remove(&compiled.id);
                    if let TopologyProbeStencil::Boundary(samples) = stencil.as_ref() {
                        let closed = self.boundary_probe_is_closed(compiled.id);
                        let mut length = 0.0;
                        for pair in samples.windows(2) {
                            length += (pair[1].point - pair[0].point).norm();
                        }
                        if closed
                            && let (Some(first), Some(last)) = (samples.first(), samples.last())
                        {
                            length += (first.point - last.point).norm();
                        }
                        self.probe_metrics.insert(compiled.id, (length, closed));
                    }
                }
                TopologyProbeCompilation::Failed(reason) => {
                    self.probe_status.insert(compiled.id, reason.clone());
                }
                TopologyProbeCompilation::Disabled => {
                    self.probe_status
                        .insert(compiled.id, "Recording is off".into());
                }
            }
        }
        for probe in &self.editor.document.model.probes {
            if let TopologyProbeTarget::Segment { start, end, .. } = probe.target {
                self.probe_metrics
                    .insert(probe.id, ((end - start).norm(), false));
            }
        }
    }
    /// A boundary probe covering every span of a periodic curve samples a closed
    /// loop, so its last sample joins its first when integrating.
    pub(super) fn boundary_probe_is_closed(&self, id: ProbeId) -> bool {
        let Some(TopologyProbeTarget::Boundary(target)) = self
            .editor
            .document
            .model
            .probes
            .iter()
            .find(|probe| probe.id == id)
            .map(|probe| &probe.target)
        else {
            return false;
        };
        self.editor
            .document
            .model
            .accepted
            .geometry
            .curves
            .iter()
            .find(|curve| curve.id == target.curve)
            .is_some_and(|curve| {
                !curve.spline.is_open()
                    && curve
                        .spans
                        .iter()
                        .all(|span| target.spans.contains(&span.id))
            })
    }
    pub(super) fn clear_probe_trace(&mut self, id: ProbeId) {
        self.probe_traces.remove(&id);
        self.curve_probe_traces.remove(&id);
        self.area_probe_traces.remove(&id);
    }
    pub(super) fn probe_time_window(
        samples: &[PointProbeRecord],
        view: &mut ProbeViewState,
    ) -> Option<(f64, f64)> {
        let first = samples.first()?.time;
        let last = samples.last()?.time;
        let available = last - first;
        if !available.is_finite() || available <= f64::EPSILON {
            return None;
        }
        let span = view.span.min(available);
        if view.live {
            view.end_time = last;
        }
        view.end_time = view.end_time.clamp(first + span, last);
        Some((view.end_time - span, view.end_time))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Dragging an endpoint must move only that endpoint, and a body drag must
    /// translate the whole probe.
    #[test]
    fn probe_drags_reshape_and_translate() {
        let mut state = Playground::default();
        state
            .editor
            .create_probe(
                "Line".into(),
                [91, 220, 194],
                TopologyProbeTarget::Segment {
                    start: Point2::new(-0.5, 0.0),
                    end: Point2::new(0.5, 0.0),
                    preset: ProbeSamplingPreset::Medium,
                },
            )
            .unwrap();
        let index = state
            .editor
            .document
            .model
            .probes
            .iter()
            .position(|probe| probe.name == "Line")
            .unwrap();
        let id = state.editor.document.model.probes[index].id;
        let original = state.editor.document.model.probes[index].target.clone();

        state.drag_probe(
            ProbeHit::SegmentEndpoint(id, true),
            &original,
            Point2::new(0.0, 0.25),
            false,
        );
        let TopologyProbeTarget::Segment { start, end, .. } =
            state.editor.document.model.probes[index].target.clone()
        else {
            panic!("expected a segment")
        };
        assert!((start - Point2::new(-0.5, 0.25)).norm() < 1.0e-12);
        assert!((end - Point2::new(0.5, 0.0)).norm() < 1.0e-12);

        state.drag_probe(
            ProbeHit::SegmentBody(id),
            &original,
            Point2::new(0.1, -0.1),
            false,
        );
        let TopologyProbeTarget::Segment { start, end, .. } =
            state.editor.document.model.probes[index].target.clone()
        else {
            panic!("expected a segment")
        };
        assert!((start - Point2::new(-0.4, -0.1)).norm() < 1.0e-12);
        assert!((end - Point2::new(0.6, -0.1)).norm() < 1.0e-12);
    }
}
