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
        let span = view.readout.span.min(available);
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
    use funfern_app::topology_editor::{
        ClosedCurvePurpose, TopologyAcceptance, TopologyBoundaryProbeTarget, TopologyProbeTarget,
    };

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
    #[test]
    fn polyline_midpoint_splits_the_path_by_arclength() {
        let path = [
            Point2::new(0.0, 0.0),
            Point2::new(2.0, 0.0),
            Point2::new(2.0, 2.0),
        ];
        let midpoint = Playground::polyline_midpoint(&path).unwrap();
        assert!((midpoint - Point2::new(2.0, 0.0)).norm() < 1.0e-12);
        assert_eq!(
            Playground::polyline_midpoint(&[Point2::new(1.0, 3.0)]),
            Some(Point2::new(1.0, 3.0))
        );
        assert_eq!(Playground::polyline_midpoint(&[]), None);
    }

    #[test]
    fn polyline_anchor_carries_the_segment_it_landed_on() {
        let path = [
            Point2::new(0.0, 0.0),
            Point2::new(2.0, 0.0),
            Point2::new(2.0, 2.0),
        ];
        let (point, segment) = Playground::polyline_anchor(&path, 0.25).unwrap();
        assert!((point - Point2::new(1.0, 0.0)).norm() < 1.0e-12);
        assert_eq!(segment, [path[0], path[1]]);
        let (point, segment) = Playground::polyline_anchor(&path, 0.75).unwrap();
        assert!((point - Point2::new(2.0, 1.0)).norm() < 1.0e-12);
        assert_eq!(segment, [path[1], path[2]]);
        assert_eq!(
            Playground::polyline_anchor(&[Point2::new(1.0, 3.0)], 0.5),
            None
        );
    }

    /// The arrow is drawn from the trace the probe reads into its badge, so it
    /// has to run out of that trace. `Left` names the face on the left of
    /// increasing parameter, so on a path running east the left trace is the
    /// northern one and an arrow arriving from it points south.
    #[test]
    fn a_boundary_probes_arrow_leaves_the_trace_it_reads() {
        let path = [Point2::new(-1.0, 0.0), Point2::new(1.0, 0.0)];
        let outward = |side, reversed| boundary_probe_orientation(&path, side, reversed).unwrap().1;
        assert!((outward(CurveTraceSide::Left, false) - Point2::new(0.0, -1.0)).norm() < 1.0e-12);
        assert!((outward(CurveTraceSide::Right, false) - Point2::new(0.0, 1.0)).norm() < 1.0e-12);
        // Reversing turns the arclength axis, never the side that is read.
        assert!((outward(CurveTraceSide::Left, true) - Point2::new(0.0, -1.0)).norm() < 1.0e-12);
        assert!((outward(CurveTraceSide::Right, true) - Point2::new(0.0, 1.0)).norm() < 1.0e-12);
    }

    /// The two arrows share a corner, so reversing has to turn one of them and
    /// leave the other where it is: the arclength axis runs the other way, the
    /// side that is read does not change, and the corner stays put.
    #[test]
    fn reversing_a_boundary_probe_turns_its_arclength_arrow_alone() {
        let path = [
            Point2::new(-1.0, 0.0),
            Point2::new(0.0, 0.0),
            Point2::new(0.0, 2.0),
        ];
        let orientation =
            |reversed| boundary_probe_orientation(&path, CurveTraceSide::Left, reversed).unwrap();
        let (forward_point, forward_outward, forward_along) = orientation(false);
        let (back_point, back_outward, back_along) = orientation(true);
        assert!((forward_point - Point2::new(0.0, 0.5)).norm() < 1.0e-12);
        assert!((back_point - forward_point).norm() < 1.0e-12);
        assert!((forward_along - Point2::new(0.0, 1.0)).norm() < 1.0e-12);
        assert!((back_along - Point2::new(0.0, -1.0)).norm() < 1.0e-12);
        assert!((forward_outward - Point2::new(1.0, 0.0)).norm() < 1.0e-12);
        assert!((back_outward - forward_outward).norm() < 1.0e-12);
    }

    #[test]
    fn a_boundary_path_too_short_to_orient_draws_no_marks() {
        for path in [vec![], vec![Point2::new(0.0, 0.0)]] {
            assert!(boundary_probe_orientation(&path, CurveTraceSide::Left, false).is_none());
        }
        let stationary = [Point2::new(1.0, 1.0), Point2::new(1.0, 1.0)];
        assert!(boundary_probe_orientation(&stationary, CurveTraceSide::Left, false).is_none());
    }

    /// Every probe kind must resolve a badge point, otherwise it draws no label.
    #[test]
    fn every_probe_kind_resolves_a_label_anchor() {
        let mut state = Playground::default();
        let curve = state
            .editor
            .create_closed_curve(
                PeriodicCubicSpline::rounded(Point2::new(0.0, 0.0), 0.4),
                ClosedCurvePurpose::Hole,
            )
            .unwrap();
        for _ in 0..100_000 {
            state.editor.validate_frame(256);
            if state.editor.acceptance != TopologyAcceptance::Pending {
                break;
            }
        }
        assert_eq!(state.editor.acceptance, TopologyAcceptance::Valid);
        let spans = state
            .editor
            .document
            .model
            .draft
            .geometry
            .curves
            .iter()
            .find(|candidate| candidate.id == curve)
            .unwrap()
            .spans
            .iter()
            .map(|span| span.id)
            .collect::<Vec<_>>();
        state
            .editor
            .create_probe(
                "Edge".into(),
                [248, 196, 112],
                TopologyProbeTarget::Boundary(TopologyBoundaryProbeTarget {
                    curve,
                    spans,
                    side: CurveTraceSide::Left,
                    reversed: false,
                    preset: ProbeSamplingPreset::Medium,
                }),
            )
            .unwrap();
        state
            .editor
            .create_probe(
                "Line".into(),
                [91, 220, 194],
                TopologyProbeTarget::Segment {
                    start: Point2::new(-0.8, -0.2),
                    end: Point2::new(0.8, 0.2),
                    preset: ProbeSamplingPreset::Medium,
                },
            )
            .unwrap();
        state
            .editor
            .create_probe(
                "Spot".into(),
                [91, 220, 194],
                TopologyProbeTarget::Point(Point2::new(0.6, 0.6)),
            )
            .unwrap();
        state.refresh_samples(viewport());
        assert!(state.sampled.is_some());

        for probe in state.editor.document.model.probes.clone() {
            let badge = state
                .probe_badge(&probe)
                .unwrap_or_else(|| panic!("{} has no label anchor", probe.name));
            assert!(
                badge.finite(),
                "{} anchored at a non-finite point",
                probe.name
            );
            if matches!(probe.target, TopologyProbeTarget::Boundary(_)) {
                assert!(
                    (badge.norm() - 0.4).abs() < 0.05,
                    "boundary badge left its curve: {badge:?}"
                );
            }
        }
    }

    /// A click on a probe must select it without starting a drag, and the badge
    /// of a boundary probe must be grabbable at its drawn position.
    #[test]
    fn probe_hit_testing_covers_every_kind() {
        let mut state = Playground::default();
        state
            .editor
            .create_probe(
                "Spot".into(),
                [91, 220, 194],
                TopologyProbeTarget::Point(Point2::new(0.2, 0.1)),
            )
            .unwrap();
        state
            .editor
            .create_probe(
                "Line".into(),
                [91, 220, 194],
                TopologyProbeTarget::Segment {
                    start: Point2::new(-0.6, -0.3),
                    end: Point2::new(0.6, -0.3),
                    preset: ProbeSamplingPreset::Medium,
                },
            )
            .unwrap();
        state
            .editor
            .create_probe(
                "Disk".into(),
                [91, 220, 194],
                TopologyProbeTarget::AreaDisk {
                    center: Point2::new(-0.5, 0.5),
                    radius: 0.2,
                },
            )
            .unwrap();
        state.refresh_samples(viewport());
        let id_of = |state: &Playground, name: &str| {
            state
                .editor
                .document
                .model
                .probes
                .iter()
                .find(|probe| probe.name == name)
                .unwrap()
                .id
        };
        let point = id_of(&state, "Spot");
        let line = id_of(&state, "Line");
        let disk = id_of(&state, "Disk");

        let at = |state: &Playground, world: Point2| state.screen(world, viewport());
        assert_eq!(
            state.hit_probe(at(&state, Point2::new(0.2, 0.1)), viewport()),
            Some(ProbeHit::Point(point))
        );
        assert_eq!(
            state.hit_probe(at(&state, Point2::new(-0.6, -0.3)), viewport()),
            Some(ProbeHit::SegmentEndpoint(line, true))
        );
        assert_eq!(
            state.hit_probe(at(&state, Point2::new(0.6, -0.3)), viewport()),
            Some(ProbeHit::SegmentEndpoint(line, false))
        );
        assert_eq!(
            state.hit_probe(at(&state, Point2::new(0.0, -0.3)), viewport()),
            Some(ProbeHit::SegmentBody(line))
        );
        assert_eq!(
            state.hit_probe(at(&state, Point2::new(-0.3, 0.5)), viewport()),
            Some(ProbeHit::AreaDiskRadius(disk))
        );
        assert_eq!(
            state.hit_probe(at(&state, Point2::new(-0.5, 0.5)), viewport()),
            Some(ProbeHit::AreaDiskBody(disk))
        );
        assert_eq!(
            state.hit_probe(at(&state, Point2::new(0.9, 0.9)), viewport()),
            None
        );
    }

    /// Placing a probe reads the same modifier dragging one already does, with
    /// the same conventions: a position lands on the grid, and a disk's radius
    /// is itself a multiple of it rather than the distance to a snapped rim.
    ///
    /// The grid is the zoom's, so the zoom is named. At 400 pixels per unit it
    /// draws fifths of a unit and divides them in four, which is the 0.05 this
    /// used to be fixed at.
    #[test]
    fn shift_places_a_probe_on_the_grid() {
        let step = grid_steps(400.0).1;
        assert!((step - 0.05).abs() < 1.0e-9, "the zoom moved: {step}");
        let on_grid = |value: f64| {
            let steps = value / step;
            (steps - steps.round()).abs() < 1.0e-9
        };
        let at = |point: Point2, x: f64, y: f64| {
            assert!(
                on_grid(point.x) && on_grid(point.y),
                "{point:?} is off the grid"
            );
            assert!(
                (point.x - x).abs() < 1.0e-9 && (point.y - y).abs() < 1.0e-9,
                "{point:?} is not the nearest node to ({x}, {y})"
            );
        };
        let mut state = Playground {
            probe_mode: Some(ProbePlacement::Point),
            scale: 400.0,
            ..Playground::default()
        };
        state.probe_placement_click(Point2::new(0.117, -0.233), true);
        let TopologyProbeTarget::Point(placed) =
            state.editor.document.model.probes.last().unwrap().target
        else {
            panic!("a point probe")
        };
        at(placed, 0.10, -0.25);

        state.probe_mode = Some(ProbePlacement::Segment { start: None });
        state.probe_placement_click(Point2::new(-0.312, 0.081), true);
        state.probe_placement_click(Point2::new(0.446, -0.377), true);
        let TopologyProbeTarget::Segment { start, end, .. } = state
            .editor
            .document
            .model
            .probes
            .last()
            .unwrap()
            .target
            .clone()
        else {
            panic!("a line probe")
        };
        at(start, -0.30, 0.10);
        at(end, 0.45, -0.40);

        // The rim click lands at a distance of 0.3111…, which is no multiple of
        // the grid; snapping the point rather than the radius would keep it.
        state.probe_mode = Some(ProbePlacement::Disk { center: None });
        state.probe_placement_click(Point2::new(0.019, -0.022), true);
        state.probe_placement_click(Point2::new(0.244, 0.193), true);
        let TopologyProbeTarget::AreaDisk { center, radius } =
            state.editor.document.model.probes.last().unwrap().target
        else {
            panic!("a disk probe")
        };
        at(center, 0.0, 0.0);
        assert!(on_grid(radius), "radius {radius} is off the grid");
        assert!(radius >= step);

        // Without the modifier nothing moves.
        state.probe_mode = Some(ProbePlacement::Point);
        state.probe_placement_click(Point2::new(0.117, -0.233), false);
        let TopologyProbeTarget::Point(loose) =
            state.editor.document.model.probes.last().unwrap().target
        else {
            panic!("a point probe")
        };
        assert_eq!(loose, Point2::new(0.117, -0.233));
    }

    #[test]
    fn shift_snaps_the_probe_rather_than_the_cursor() {
        let mut state = Playground::default();
        state
            .editor
            .create_probe(
                "Spot".into(),
                [91, 220, 194],
                TopologyProbeTarget::Point(Point2::new(0.0, 0.0)),
            )
            .unwrap();
        let probe = state.editor.document.model.probes.last().unwrap().clone();
        // Grabbed well off centre, then dragged to an arbitrary place.
        for grab_offset in [0.0, 0.017, -0.023] {
            let original = TopologyProbeTarget::Point(Point2::new(0.0, 0.0));
            let pointer = Point2::new(0.31 + grab_offset, -0.22 + grab_offset);
            let grab = Point2::new(grab_offset, grab_offset);
            state.drag_probe(ProbeHit::Point(probe.id), &original, pointer - grab, true);
            let TopologyProbeTarget::Point(position) =
                state.editor.document.model.probes.last().unwrap().target
            else {
                panic!("a point probe")
            };
            for value in [position.x, position.y] {
                let steps = value / 0.05;
                assert!(
                    (steps - steps.round()).abs() < 1.0e-9,
                    "grabbed at {grab_offset}, landed off the grid at {value}"
                );
            }
        }
        // A disk radius snaps too, rather than jumping by the grab offset.
        state
            .editor
            .create_probe(
                "Disk".into(),
                [91, 220, 194],
                TopologyProbeTarget::AreaDisk {
                    center: Point2::new(0.0, 0.0),
                    radius: 0.2,
                },
            )
            .unwrap();
        let disk = state.editor.document.model.probes.last().unwrap().clone();
        state.drag_probe(
            ProbeHit::AreaDiskRadius(disk.id),
            &disk.target,
            Point2::new(0.113, 0.0),
            true,
        );
        let TopologyProbeTarget::AreaDisk { radius, .. } =
            state.editor.document.model.probes.last().unwrap().target
        else {
            panic!("a disk probe")
        };
        let steps = radius / 0.05;
        assert!((steps - steps.round()).abs() < 1.0e-9, "radius {radius}");
    }
}
