//! The guided tour. A first launch, with nothing to restore and the tour
//! not yet seen, opens a small scene and lays the tour over it: nine steps,
//! each naming one thing to do and the control to do it with, spotlit where
//! the bar or the panel drew it this frame, so the light follows the bar's
//! folds; a step moves on a moment after the state shows the action done,
//! and Next and Skip are always there. By the end the panels have each been
//! opened once and said what they hold. The tour is back under Examples
//! and on the scene card; seen once is a marker beside the autosave.

use super::*;
use funfern_app::document::VectorOverlay;
use funfern_app::topology_examples::{GUIDE_GLASS, GUIDE_OBSTACLE, GUIDE_REGION, guide_scene};

/// A control a step points at. The top bar and the panels record where they
/// drew these this frame; a step asks for its controls in order of
/// preference, the first one drawn getting the light.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Spotlight {
    RunPause,
    Draw,
    Examples,
    Panels,
    Tab(InspectorPanel),
    /// The Probes panel's point-probe button.
    ProbePoint,
    /// A region's material choice in the Materials panel, and an entry of
    /// its list while the list is open.
    RegionMaterial(RegionId),
    MaterialChoice {
        region: RegionId,
        material: MaterialId,
    },
    /// An entry of the View panel's vector overlay list while it is open.
    OverlayChoice(VectorOverlay),
    /// The Edit panel's boundary law picker, drawn while a boundary is
    /// selected, and its second-order outgoing entry while the list is open.
    BoundaryLaw,
    OutgoingLaw,
    /// The View panel's vector overlay choice.
    VectorOverlay,
    /// The Draw palette's line tool, and its Finish button while a line is
    /// being drawn.
    DrawLine,
    DrawFinish,
    /// The source, where the viewport draws it.
    Source,
    /// The obstacle, where the viewport draws it.
    Obstacle,
    /// The right-hand wall, where the viewport draws it.
    Wall,
}

/// Where the spotlit controls were drawn this frame.
#[derive(Default)]
pub(super) struct Spotlights {
    rects: Vec<(Spotlight, Rect)>,
}

impl Spotlights {
    pub(super) fn clear(&mut self) {
        self.rects.clear();
    }

    pub(super) fn record(&mut self, target: Spotlight, rect: Rect) {
        self.rects.retain(|(recorded, _)| *recorded != target);
        self.rects.push((target, rect));
    }

    pub(super) fn rect(&self, target: Spotlight) -> Option<Rect> {
        self.rects
            .iter()
            .find(|(recorded, _)| *recorded == target)
            .map(|(_, rect)| *rect)
    }
}

/// One step of the tour: what to do, where, and how the state shows it done.
pub(super) struct GuideStep {
    pub(super) title: &'static str,
    pub(super) text: &'static str,
    /// The controls to light, in order of preference, each with the hint
    /// shown when it is the one lit: a panel's tab folded into the Panels
    /// menu lights the menu and says so. A tab counts as drawn only while
    /// its panel is closed, so once the panel is open the next target, a
    /// control inside it, takes the light.
    targets: &'static [(Spotlight, Option<&'static str>)],
    /// Lit as well, with no hint, when drawn: the thing the step is about.
    extra: &'static [Spotlight],
    /// The light is a dashed rectangle, a marquee's shape, rather than a
    /// ring.
    dashed: bool,
    done: fn(&Playground) -> bool,
}

/// A done step waits this long before moving on, so its tick is seen.
const GUIDE_ADVANCE_DELAY: f64 = 0.9;
const GUIDE_CARD_WIDTH: f32 = 400.0;
/// How dark the rest of the screen goes round the lit controls.
const GUIDE_DIM_ALPHA: u8 = 140;

const PANELS_HINT: Option<&str> = Some("The panel is under Panels in the bar.");

pub(super) static STEPS: [GuideStep; 10] = [
    GuideStep {
        title: "Welcome to funfern",
        text: "The wave runs as soon as its mesh is ready. Run and Pause, Step and Reset \
               sit at the right end of the bar. Pause the wave, then run it again.",
        targets: &[(Spotlight::RunPause, None)],
        extra: &[],
        dashed: false,
        done: |state| state.guide.paused_seen && state.wave_running,
    },
    GuideStep {
        title: "The source",
        text: "The circled point is the source; the Simulation panel sets its signal. \
               Drag it somewhere else and watch the field follow.",
        targets: &[(Spotlight::Source, None)],
        extra: &[],
        dashed: false,
        done: |state| {
            state.editor.document.model.source.position != state.guide.baseline.source
                && !state.editor.editing()
        },
    },
    GuideStep {
        title: "Edit",
        text: "Edit is the selection: click a curve or a wall and the panel offers its \
               tools, from reshaping it to its boundary law. The right-hand wall echoes. \
               Open Edit, click that wall and give it an outgoing law; the echo stops.",
        targets: &[
            (Spotlight::OutgoingLaw, None),
            (Spotlight::BoundaryLaw, Some("Pick an outgoing law for it.")),
            (Spotlight::Tab(InspectorPanel::Edit), None),
            (Spotlight::Panels, PANELS_HINT),
            (
                Spotlight::Wall,
                Some("Click the right-hand wall to select it."),
            ),
        ],
        extra: &[Spotlight::Wall],
        dashed: false,
        done: |state| {
            state.editor.document.model.draft.outer_boundaries.sides[OuterSide::Right.index()]
                != state.guide.baseline.right_wall
        },
    },
    GuideStep {
        title: "Select and move",
        text: "Press on empty space beside the round obstacle, drag a rectangle over the \
               whole of it and let go: that is a marquee, and everything inside it is \
               selected. Handles appear round it, the gizmo: the four-way grip moves it, \
               the ring turns it, the arrows scale it, and the dot at the centre is the \
               pivot the others work about. Drag the obstacle itself, or the four-way grip, \
               and put it somewhere else.",
        targets: &[(Spotlight::Obstacle, None)],
        extra: &[],
        dashed: true,
        done: |state| {
            state.guide_obstacle_controls() != state.guide.baseline.obstacle
                && !state.editor.editing()
        },
    },
    GuideStep {
        title: "Draw",
        text: "+ Draw opens the palette. Under Open curve pick Polyline, then click two or \
               three points across the scene; each click adds a vertex. Press Finish in the \
               palette, or Enter, to end the line, which becomes a wall. A line that crosses \
               itself cannot be finished: Cancel, or Esc, and draw it again.",
        targets: &[
            (
                Spotlight::DrawFinish,
                Some("Press Finish, or Enter, to end the line."),
            ),
            (
                Spotlight::DrawLine,
                Some("Pick Polyline, then click the line's points in the scene."),
            ),
            (Spotlight::Draw, None),
        ],
        extra: &[],
        dashed: false,
        done: |state| {
            state.editor.document.model.draft.geometry.curves.len() > state.guide.baseline.curves
        },
    },
    GuideStep {
        title: "Materials",
        text: "Materials holds the library and says which region has which. Give the round \
               region at the bottom the glass.",
        targets: &[
            (
                Spotlight::MaterialChoice {
                    region: GUIDE_REGION,
                    material: GUIDE_GLASS,
                },
                None,
            ),
            (
                Spotlight::RegionMaterial(GUIDE_REGION),
                Some("Pick the glass for it."),
            ),
            (
                Spotlight::Tab(InspectorPanel::Materials),
                Some("Open the Materials panel first."),
            ),
            (Spotlight::Panels, PANELS_HINT),
        ],
        extra: &[],
        dashed: false,
        done: |state| {
            state
                .editor
                .document
                .model
                .draft
                .region(GUIDE_REGION)
                .map(|region| region.material)
                != state.guide.baseline.region_material
        },
    },
    GuideStep {
        title: "View",
        text: "View is what is drawn: the field's colours and exposure, the vector overlay \
               as arrows or streamlines, material overlays, the mesh. Switch the vector \
               overlay on.",
        targets: &[
            (
                Spotlight::OverlayChoice(VectorOverlay::RelativeEnergyFlow),
                None,
            ),
            (Spotlight::VectorOverlay, Some("Pick the energy flow.")),
            (Spotlight::Tab(InspectorPanel::View), None),
            (Spotlight::Panels, PANELS_HINT),
        ],
        extra: &[],
        dashed: false,
        done: |state| state.editor.document.presentation.vector_overlay != VectorOverlay::Off,
    },
    GuideStep {
        title: "Simulation",
        text: "Simulation holds the mesh size and adaptation, the solver's settings and the \
               source's signal; the status line below says what is being rebuilt. Open it.",
        targets: &[
            (Spotlight::Tab(InspectorPanel::Simulation), None),
            (Spotlight::Panels, PANELS_HINT),
        ],
        extra: &[],
        dashed: false,
        done: |state| state.inspector == Some(InspectorPanel::Simulation),
    },
    GuideStep {
        title: "Probes",
        text: "Probes read the field at a point, along a line, over a disk or a region, and \
               in the far field, each with its own plots. Add a point probe and click in \
               the scene.",
        targets: &[
            (Spotlight::ProbePoint, None),
            (
                Spotlight::Tab(InspectorPanel::Probes),
                Some("Open the Probes panel first."),
            ),
            (Spotlight::Panels, PANELS_HINT),
        ],
        extra: &[],
        dashed: false,
        done: |state| {
            state.editor.document.model.probes.len() > state.guide.baseline.probes
                && !state.probe_windows.is_empty()
        },
    },
    GuideStep {
        title: "The gallery",
        text: "Examples holds ready scenes in sections, each with a card saying what it \
               shows and what to look for. Open it and pick one. This tour is there too, \
               whenever you want it back.",
        targets: &[(Spotlight::Examples, None)],
        extra: &[],
        dashed: false,
        done: |state| state.examples_open,
    },
];

/// What the scene held when the tour started, for the steps that ask for a
/// change.
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct GuideBaseline {
    pub(super) source: Point2,
    pub(super) right_wall: OuterBoundaryCondition,
    /// The obstacle's control points.
    pub(super) obstacle: Vec<Point2>,
    pub(super) curves: usize,
    pub(super) region_material: Option<MaterialId>,
    pub(super) probes: usize,
}

#[derive(Default)]
pub(super) struct Guide {
    pub(super) active: bool,
    pub(super) step: usize,
    /// When the current step's action was first seen done.
    done_since: Option<f64>,
    /// Whether the wave has been paused during the current step.
    pub(super) paused_seen: bool,
    pub(super) baseline: GuideBaseline,
    /// Whether the tour has ended, by its last step or by Skip, which is
    /// what the marker beside the autosave records.
    pub(super) finished: bool,
    persisted: bool,
}

impl Playground {
    /// Opens the guide scene and starts the tour, as an undoable scene
    /// change when `history` is set, as launch's own when not.
    pub(super) fn start_guide(&mut self, history: bool) {
        match self.set_document(guide_scene(), history, true) {
            Ok(()) => {
                self.examples_open = false;
                // Each panel's step opens that panel; one already open would
                // make its step a puzzle.
                self.inspector = None;
                self.guide = Guide {
                    active: true,
                    step: 0,
                    done_since: None,
                    paused_seen: false,
                    baseline: self.guide_baseline(),
                    finished: false,
                    persisted: self.guide.persisted,
                };
                self.notify("Guided tour");
            }
            Err(error) => self.notify(error),
        }
    }

    fn guide_baseline(&self) -> GuideBaseline {
        let model = &self.editor.document.model;
        GuideBaseline {
            source: model.source.position,
            right_wall: model.draft.outer_boundaries.sides[OuterSide::Right.index()],
            obstacle: self.guide_obstacle_controls(),
            curves: model.draft.geometry.curves.len(),
            region_material: model
                .draft
                .region(GUIDE_REGION)
                .map(|region| region.material),
            probes: model.probes.len(),
        }
    }

    /// The obstacle's control points, which any move, turn or scaling of it
    /// changes; empty once it is gone.
    pub(super) fn guide_obstacle_controls(&self) -> Vec<Point2> {
        self.editor
            .document
            .model
            .draft
            .geometry
            .curves
            .iter()
            .find(|curve| curve.id == GUIDE_OBSTACLE)
            .map(|curve| match &curve.spline {
                CurveSpline::Closed(spline) => spline.controls().to_vec(),
                CurveSpline::Open(spline) => spline.controls().to_vec(),
            })
            .unwrap_or_default()
    }

    /// Whether the current step's action shows in the state.
    pub(super) fn guide_step_done(&self) -> bool {
        self.guide.active
            && STEPS
                .get(self.guide.step)
                .is_some_and(|step| (step.done)(self))
    }

    /// Reads the state for the current step and moves on a moment after
    /// its action is done. `now` is the frame's clock.
    pub(super) fn guide_update(&mut self, now: f64) {
        if !self.guide.active {
            return;
        }
        if !self.wave_running {
            self.guide.paused_seen = true;
        }
        if self.guide_step_done() {
            let since = *self.guide.done_since.get_or_insert(now);
            if now - since >= GUIDE_ADVANCE_DELAY {
                self.guide_advance();
            }
        } else {
            self.guide.done_since = None;
        }
    }

    /// The next step, or the end after the last. A step's baseline is what
    /// the scene holds as the step starts, so an action done early, the
    /// source dragged before its step, does not count for the step when it
    /// comes: the user does it again, or presses Next.
    pub(super) fn guide_advance(&mut self) {
        self.guide.done_since = None;
        if self.guide.step + 1 >= STEPS.len() {
            self.finish_guide();
        } else {
            self.guide.step += 1;
            self.guide.paused_seen = false;
            self.guide.baseline = self.guide_baseline();
        }
    }

    pub(super) fn finish_guide(&mut self) {
        self.guide.active = false;
        self.guide.finished = true;
    }

    /// Writes the seen marker once the tour has ended, through `mark`, the
    /// store beside the autosave.
    pub(super) fn persist_guide_seen(&mut self, mark: impl FnOnce() -> Result<(), String>) {
        if self.guide.finished && !self.guide.persisted {
            self.guide.persisted = true;
            if let Err(error) = mark() {
                self.notify(format!("The tour's marker was not written: {error}"));
            }
        }
    }

    /// Where a target is this frame: the recorded rect for a control, the
    /// viewport's own for the source, the obstacle and the wall, and
    /// nothing for a tab whose panel is already open.
    pub(super) fn spotlight_rect(&self, target: Spotlight, viewport: Rect) -> Option<Rect> {
        match target {
            Spotlight::Source => {
                let source = &self.editor.document.model.source;
                source.enabled.then(|| {
                    Rect::from_center_size(
                        self.screen(source.position, viewport),
                        egui::vec2(48.0, 48.0),
                    )
                })
            }
            Spotlight::Obstacle => {
                let controls = self.guide_obstacle_controls();
                (!controls.is_empty()).then(|| {
                    let mut rect = Rect::NOTHING;
                    for control in controls {
                        rect.extend_with(self.screen(control, viewport));
                    }
                    rect.expand(10.0)
                })
            }
            Spotlight::Wall => {
                let domain = self.editor.document.model.draft.geometry.domain;
                let top = self.screen(Point2::new(domain.max_x, domain.max_y), viewport);
                let bottom = self.screen(Point2::new(domain.max_x, domain.min_y), viewport);
                Some(Rect::from_two_pos(top, bottom).expand(8.0))
            }
            Spotlight::Tab(panel) if self.inspector == Some(panel) => None,
            other => self.spotlights.rect(other),
        }
    }

    /// What the tour lights this frame: the current step's spotlight, or
    /// nothing once the step is done.
    pub(super) fn guide_light(&self, viewport: Rect) -> Option<(Rect, Option<&'static str>)> {
        if !self.guide.active || self.guide_step_done() {
            return None;
        }
        self.guide_spotlight(&STEPS[self.guide.step], viewport)
    }

    /// The rect the step lights this frame and the hint for it, the first
    /// of its targets that was drawn.
    pub(super) fn guide_spotlight(
        &self,
        step: &GuideStep,
        viewport: Rect,
    ) -> Option<(Rect, Option<&'static str>)> {
        step.targets.iter().find_map(|(target, hint)| {
            self.spotlight_rect(*target, viewport)
                .map(|rect| (rect, *hint))
        })
    }

    /// The tour's frame: the step read against the state, the screen dimmed
    /// but for the control it lights, and the step's card over the
    /// viewport's top.
    pub(super) fn guide_frame(&mut self, ctx: &egui::Context, viewport: Rect) {
        if !self.guide.active {
            return;
        }
        self.guide_update(ctx.input(|input| input.time));
        if !self.guide.active {
            return;
        }
        let step = &STEPS[self.guide.step];
        let done = self.guide_step_done();
        // A done step lights nothing while it waits to move on: its chain
        // of targets would otherwise fall back to an earlier control, the
        // Polyline tool after a line was finished, for the moment the tick
        // shows.
        let spotlight = self.guide_light(viewport);
        let screen = ctx.content_rect();
        let mut holes = spotlight
            .iter()
            .map(|(rect, _)| rect.expand(6.0).intersect(screen))
            .collect::<Vec<_>>();
        if !done {
            for extra in step.extra {
                if let Some(rect) = self.spotlight_rect(*extra, viewport) {
                    let hole = rect.expand(6.0).intersect(screen);
                    if !holes.contains(&hole) {
                        holes.push(hole);
                    }
                }
            }
        }
        // The dimming is paint on a foreground layer, no widget, so clicks
        // go through it to the controls underneath. Open lists and menus
        // are foreground areas too, drawn before this paint, so each is a
        // hole as well, without a glow: a list the step had the user open
        // went dark otherwise, its entries with it.
        let painter = ctx.layer_painter(egui::LayerId::new(
            egui::Order::Foreground,
            egui::Id::new("guide-spotlight"),
        ));
        let mut clear = holes.clone();
        clear.extend(ctx.memory(|memory| {
            memory
                .areas()
                .visible_layer_ids()
                .into_iter()
                .filter(|layer| {
                    layer.order == egui::Order::Foreground
                        && layer.id != egui::Id::new("guide-spotlight")
                })
                .filter_map(|layer| memory.area_rect(layer.id))
                .collect::<Vec<_>>()
        }));
        {
            for shade in dim_except(screen, &clear) {
                painter.rect_filled(shade, 0.0, Color32::from_black_alpha(GUIDE_DIM_ALPHA));
            }
            for hole in &holes {
                if step.dashed {
                    let corners = [
                        hole.left_top(),
                        hole.right_top(),
                        hole.right_bottom(),
                        hole.left_bottom(),
                        hole.left_top(),
                    ];
                    painter.add(egui::Shape::dashed_line(
                        &corners,
                        Stroke::new(2.5, TEAL),
                        8.0,
                        5.0,
                    ));
                } else {
                    // A glow: three rings, the outer ones faint.
                    for (width, alpha) in [(9.0, 0.16), (5.0, 0.43), (2.5, 1.0)] {
                        painter.rect_stroke(
                            *hole,
                            6.0,
                            Stroke::new(width, TEAL.gamma_multiply(alpha)),
                            egui::StrokeKind::Outside,
                        );
                    }
                }
            }
        }
        let (index, total) = (self.guide.step + 1, STEPS.len());
        let last = index == total;
        let mut skip = false;
        let mut next = false;
        let frame = egui::Frame::window(&ctx.global_style()).stroke(Stroke::new(1.5, TEAL));
        let card = egui::Window::new("Guided tour")
            .id(egui::Id::new("guide-card"))
            .title_bar(false)
            .resizable(false)
            .collapsible(false)
            .frame(frame)
            // At the viewport's foot: the palette, the floating inspector,
            // the probe windows and the gallery all open at its top.
            .pivot(egui::Align2::CENTER_BOTTOM)
            .fixed_pos(viewport.center_bottom() - egui::vec2(0.0, 14.0))
            .default_width(GUIDE_CARD_WIDTH)
            .show(ctx, |ui| {
                ui.set_width(GUIDE_CARD_WIDTH.min(viewport.width() - 24.0));
                ui.horizontal(|ui| {
                    ui.colored_label(TEAL, egui::RichText::new("GUIDED TOUR").small().strong());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.weak(format!("step {index} of {total}"));
                    });
                });
                ui.heading(step.title);
                ui.label(step.text);
                if let Some((_, Some(hint))) = spotlight {
                    ui.colored_label(TEAL, hint);
                }
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if ui.small_button("Skip the tour").clicked() {
                        skip = true;
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button(if last { "Done" } else { "Next" }).clicked() {
                            next = true;
                        }
                        if done {
                            ui.colored_label(TEAL, "✓ done");
                        }
                    });
                });
            });
        // A pointer from the card to the control the step points at: a thin
        // curve leaving the card square to its edge and arriving square to
        // the control's, with a small head.
        if let (Some(card), Some(hole)) = (card, holes.first())
            && let Some((from, to)) = pointer_between(card.response.rect, *hole)
        {
            let leave = edge_normal(card.response.rect, from);
            let arrive = edge_normal(*hole, to);
            let reach = (to - from).length() * 0.4;
            let points = [from, from + leave * reach, to + arrive * reach, to];
            for (width, alpha) in [(5.0, 0.18), (1.5, 0.9)] {
                painter.add(egui::Shape::CubicBezier(
                    egui::epaint::CubicBezierShape::from_points_stroke(
                        points,
                        false,
                        Color32::TRANSPARENT,
                        Stroke::new(width, TEAL.gamma_multiply(alpha)),
                    ),
                ));
            }
            let direction = -arrive;
            let normal = egui::vec2(-direction.y, direction.x);
            let head = 7.0;
            painter.add(egui::Shape::convex_polygon(
                vec![
                    to,
                    to - direction * head + normal * head * 0.45,
                    to - direction * head - normal * head * 0.45,
                ],
                TEAL,
                Stroke::NONE,
            ));
        }
        if skip {
            self.finish_guide();
        } else if next {
            self.guide_advance();
        }
    }
}

/// The pointer from the card to a lit rect: from the middle of the card's
/// edge that faces the rect to the middle of the rect's edge that faces the
/// card, so the curve between them is a connector's S; nothing when the two
/// touch or overlap.
pub(super) fn pointer_between(card: Rect, hole: Rect) -> Option<(Pos2, Pos2)> {
    if card.intersects(hole) {
        return None;
    }
    let (from, to) = if hole.max.y <= card.min.y {
        (card.center_top(), hole.center_bottom())
    } else if hole.min.y >= card.max.y {
        (card.center_bottom(), hole.center_top())
    } else if hole.max.x <= card.min.x {
        (card.left_center(), hole.right_center())
    } else {
        (card.right_center(), hole.left_center())
    };
    ((to - from).length() > 24.0).then_some((from, to))
}

/// The outward normal of the edge of `rect` that `point` lies on, for a
/// point the rect's edge was clamped to; the direction out from the centre
/// for any other.
pub(super) fn edge_normal(rect: Rect, point: Pos2) -> egui::Vec2 {
    if (point.y - rect.min.y).abs() < 0.5 {
        egui::vec2(0.0, -1.0)
    } else if (point.y - rect.max.y).abs() < 0.5 {
        egui::vec2(0.0, 1.0)
    } else if (point.x - rect.min.x).abs() < 0.5 {
        egui::vec2(-1.0, 0.0)
    } else if (point.x - rect.max.x).abs() < 0.5 {
        egui::vec2(1.0, 0.0)
    } else {
        (point - rect.center()).normalized()
    }
}

/// The rects that cover `screen` but for `holes`: the screen cut into
/// horizontal bands at every hole's top and bottom, and each band filled
/// between the holes that cross it.
pub(super) fn dim_except(screen: Rect, holes: &[Rect]) -> Vec<Rect> {
    let mut edges = vec![screen.min.y, screen.max.y];
    for hole in holes {
        edges.push(hole.min.y.clamp(screen.min.y, screen.max.y));
        edges.push(hole.max.y.clamp(screen.min.y, screen.max.y));
    }
    edges.sort_by(f32::total_cmp);
    edges.dedup();
    let mut shades = Vec::new();
    for band in edges.windows(2) {
        let (top, bottom) = (band[0], band[1]);
        if bottom <= top {
            continue;
        }
        let middle = 0.5 * (top + bottom);
        let mut crossing = holes
            .iter()
            .filter(|hole| hole.min.y < middle && middle < hole.max.y)
            .map(|hole| (hole.min.x.max(screen.min.x), hole.max.x.min(screen.max.x)))
            .collect::<Vec<_>>();
        crossing.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut x = screen.min.x;
        for (from, to) in crossing {
            if from > x {
                shades.push(Rect::from_min_max(
                    Pos2::new(x, top),
                    Pos2::new(from, bottom),
                ));
            }
            x = x.max(to);
        }
        if screen.max.x > x {
            shades.push(Rect::from_min_max(
                Pos2::new(x, top),
                Pos2::new(screen.max.x, bottom),
            ));
        }
    }
    shades
}

#[cfg(test)]
mod tests {
    use super::*;
    use funfern_app::topology_editor::{OpenCurvePurpose, TopologyProbeTarget};
    use funfern_app::topology_viewport::{
        RigidTransform, TopologySpanTarget, plan_rigid_transform,
    };

    /// Each step is done by the action it asks for, read from the state,
    /// and a done step moves on after its moment, the last one ending the
    /// tour.
    fn step_index(title: &str) -> usize {
        STEPS.iter().position(|step| step.title == title).unwrap()
    }

    #[test]
    fn the_steps_are_done_by_the_actions_they_ask_for() {
        let mut state = Playground::default();
        state.start_guide(false);
        assert!(state.guide.active);
        assert_eq!(state.example_opened, None, "the guide scene has no card");
        assert_eq!(
            state.inspector, None,
            "the panels are closed for their steps"
        );
        let mut now = 10.0;
        let mut expect_step =
            |state: &mut Playground, step: usize, action: &dyn Fn(&mut Playground)| {
                assert_eq!(state.guide.step, step, "{}", STEPS[step].title);
                // The app validates the topology every frame; an edit on an
                // unsettled draft is refused.
                settle(&mut state.editor);
                state.guide_update(now);
                assert!(
                    !state.guide_step_done(),
                    "{} done before its action",
                    STEPS[step].title
                );
                action(state);
                state.guide_update(now);
                assert!(
                    state.guide_step_done(),
                    "{} not done by its action",
                    STEPS[step].title
                );
                assert_eq!(state.guide.step, step, "moved on at once");
                now += GUIDE_ADVANCE_DELAY + 0.1;
                state.guide_update(now);
            };
        expect_step(&mut state, 0, &|state| {
            state.wave_running = false;
            state.guide_update(0.0);
            state.wave_running = true;
        });
        expect_step(&mut state, 1, &|state| {
            let mut source = state.editor.document.model.source;
            source.position = source.position + Point2::new(0.2, 0.1);
            state.editor.set_point_source(source).unwrap();
        });
        expect_step(&mut state, 2, &|state| {
            state
                .editor
                .set_outer_condition(
                    &[OuterSide::Right].into(),
                    OuterBoundaryCondition::SecondOrderOutgoing,
                )
                .unwrap();
        });
        expect_step(&mut state, 3, &|state| {
            // What the gizmo does on a drag: the whole obstacle selected and
            // moved by a rigid transform, in one edit.
            let geometry = state.editor.document.model.draft.geometry.clone();
            let selected = geometry
                .curves
                .iter()
                .find(|curve| curve.id == GUIDE_OBSTACLE)
                .unwrap()
                .spans
                .iter()
                .map(|span| TopologySpanTarget::Curve(span.id))
                .collect::<BTreeSet<_>>();
            let updates = plan_rigid_transform(
                &geometry,
                &selected,
                RigidTransform {
                    pivot: Point2::default(),
                    translation: Point2::new(-0.15, 0.0),
                    rotation_radians: 0.0,
                    scale: 1.0,
                },
            )
            .unwrap();
            state.editor.begin();
            state
                .editor
                .apply_transform_updates_during_edit(&updates)
                .unwrap();
            state.editor.commit();
        });
        expect_step(&mut state, 4, &|state| {
            let wall =
                OpenCubicSpline::polyline(vec![Point2::new(-0.3, 0.8), Point2::new(-0.3, 0.5)])
                    .unwrap();
            state
                .editor
                .create_open_curve(wall, OpenCurvePurpose::BoundaryBaffle, None, None)
                .unwrap();
        });
        expect_step(&mut state, 5, &|state| {
            state
                .editor
                .set_region_material(GUIDE_REGION, GUIDE_GLASS)
                .unwrap();
        });
        expect_step(&mut state, 6, &|state| {
            state.editor.document.presentation.vector_overlay = VectorOverlay::RelativeEnergyFlow;
        });
        expect_step(&mut state, 7, &|state| {
            state.inspector = Some(InspectorPanel::Simulation);
        });
        expect_step(&mut state, 8, &|state| {
            let id = state
                .editor
                .create_probe(
                    "Here".into(),
                    [91, 220, 194],
                    TopologyProbeTarget::Point(Point2::new(0.6, 0.6)),
                )
                .unwrap();
            state.probe_windows.insert(id);
        });
        expect_step(&mut state, 9, &|state| {
            state.examples_open = true;
        });
        assert!(!state.guide.active, "the last step ends the tour");
        assert!(state.guide.finished);
        let mut marked = false;
        state.persist_guide_seen(|| {
            marked = true;
            Ok(())
        });
        assert!(marked);
        marked = false;
        state.persist_guide_seen(|| {
            marked = true;
            Ok(())
        });
        assert!(!marked, "the marker is written once");
    }

    /// A step lights the first of its controls that was drawn: the panel's
    /// tab when the bar shows tabs, the Panels menu with a hint when it has
    /// folded them away, and the panel's own button when the panel is open.
    #[test]
    fn the_spotlight_falls_back_along_the_steps_targets() {
        let mut state = Playground::default();
        state.start_guide(false);
        state.guide.step = step_index("Probes");
        let step = &STEPS[state.guide.step];
        let viewport = viewport();
        assert_eq!(
            state.guide_spotlight(step, viewport),
            None,
            "nothing drawn yet"
        );
        let panels = Rect::from_min_size(Pos2::new(300.0, 4.0), egui::vec2(60.0, 24.0));
        state.spotlights.record(Spotlight::Panels, panels);
        assert_eq!(
            state.guide_spotlight(step, viewport),
            Some((panels, PANELS_HINT))
        );
        let tab = Rect::from_min_size(Pos2::new(400.0, 4.0), egui::vec2(60.0, 24.0));
        state
            .spotlights
            .record(Spotlight::Tab(InspectorPanel::Probes), tab);
        assert_eq!(
            state.guide_spotlight(step, viewport),
            Some((tab, Some("Open the Probes panel first.")))
        );
        let button = Rect::from_min_size(Pos2::new(600.0, 80.0), egui::vec2(50.0, 20.0));
        state.spotlights.record(Spotlight::ProbePoint, button);
        assert_eq!(state.guide_spotlight(step, viewport), Some((button, None)));
        state.spotlights.clear();
        assert_eq!(state.guide_spotlight(step, viewport), None);

        // A tab is lit only while its panel is closed; open, the light
        // moves on to the control inside the panel, or to the thing in the
        // scene the step is about.
        let edit = &STEPS[step_index("Edit")];
        state
            .spotlights
            .record(Spotlight::Tab(InspectorPanel::Edit), tab);
        assert_eq!(state.guide_spotlight(edit, viewport), Some((tab, None)));
        state.inspector = Some(InspectorPanel::Edit);
        let wall = state.spotlight_rect(Spotlight::Wall, viewport).unwrap();
        assert_eq!(
            state.guide_spotlight(edit, viewport),
            Some((wall, Some("Click the right-hand wall to select it.")))
        );
        let right = state.editor.document.model.draft.geometry.domain.max_x;
        assert!(wall.contains(state.screen(Point2::new(right, 0.0), viewport)));
        assert!(!wall.contains(state.screen(Point2::new(right - 0.5, 0.0), viewport)));
        state.spotlights.record(Spotlight::BoundaryLaw, button);
        assert_eq!(
            state.guide_spotlight(edit, viewport),
            Some((button, Some("Pick an outgoing law for it.")))
        );
        let entry = Rect::from_min_size(Pos2::new(610.0, 130.0), egui::vec2(120.0, 18.0));
        state.spotlights.record(Spotlight::OutgoingLaw, entry);
        assert_eq!(state.guide_spotlight(edit, viewport), Some((entry, None)));

        // The source and the obstacle are lit where the viewport draws them.
        let (rect, hint) = state
            .guide_spotlight(&STEPS[step_index("The source")], viewport)
            .unwrap();
        let at = state.screen(state.editor.document.model.source.position, viewport);
        assert!(rect.contains(at) && hint.is_none());
        let (rect, _) = state
            .guide_spotlight(&STEPS[step_index("Select and move")], viewport)
            .unwrap();
        for control in state.guide_obstacle_controls() {
            assert!(rect.contains(state.screen(control, viewport)));
        }
    }

    /// The lists' entries are lit once the lists are open: the glass in the
    /// region's material list, the energy flow in the overlay list; before
    /// that, the combo with a hint to pick them.
    #[test]
    fn the_list_entries_are_lit_once_the_lists_are_open() {
        let mut state = Playground::default();
        state.start_guide(false);
        let viewport = viewport();
        state.inspector = Some(InspectorPanel::Materials);
        let combo = Rect::from_min_size(Pos2::new(600.0, 200.0), egui::vec2(120.0, 20.0));
        state
            .spotlights
            .record(Spotlight::RegionMaterial(GUIDE_REGION), combo);
        let materials = &STEPS[step_index("Materials")];
        assert_eq!(
            state.guide_spotlight(materials, viewport),
            Some((combo, Some("Pick the glass for it.")))
        );
        let entry = Rect::from_min_size(Pos2::new(600.0, 240.0), egui::vec2(120.0, 18.0));
        state.spotlights.record(
            Spotlight::MaterialChoice {
                region: GUIDE_REGION,
                material: GUIDE_GLASS,
            },
            entry,
        );
        assert_eq!(
            state.guide_spotlight(materials, viewport),
            Some((entry, None))
        );

        state.spotlights.clear();
        state.inspector = Some(InspectorPanel::View);
        state.spotlights.record(Spotlight::VectorOverlay, combo);
        let view = &STEPS[step_index("View")];
        assert_eq!(
            state.guide_spotlight(view, viewport),
            Some((combo, Some("Pick the energy flow.")))
        );
        state.spotlights.record(
            Spotlight::OverlayChoice(VectorOverlay::RelativeEnergyFlow),
            entry,
        );
        assert_eq!(state.guide_spotlight(view, viewport), Some((entry, None)));
    }

    /// A done step lights nothing while it waits to move on, so its chain
    /// of targets does not fall back to an earlier control.
    #[test]
    fn a_done_step_lights_nothing() {
        let mut state = Playground::default();
        state.start_guide(false);
        let viewport = viewport();
        let pause = Rect::from_min_size(Pos2::new(700.0, 4.0), egui::vec2(60.0, 24.0));
        state.spotlights.record(Spotlight::RunPause, pause);
        assert_eq!(state.guide_light(viewport), Some((pause, None)));
        state.wave_running = false;
        state.guide_update(1.0);
        state.wave_running = true;
        state.guide_update(1.0);
        assert!(state.guide_step_done());
        assert_eq!(state.guide_light(viewport), None);
        state.guide_update(1.0 + GUIDE_ADVANCE_DELAY + 0.1);
        assert_eq!(STEPS[state.guide.step].title, "The source");
        assert!(
            state.guide_light(viewport).is_some(),
            "the next step lights its own"
        );
    }

    /// A step's baseline is taken as the step starts: the source dragged
    /// during the first step does not count for the second, which asks for
    /// it again.
    #[test]
    fn an_action_done_early_does_not_skip_its_step() {
        let mut state = Playground::default();
        state.start_guide(false);
        settle(&mut state.editor);
        let mut source = state.editor.document.model.source;
        source.position = source.position + Point2::new(0.2, 0.0);
        state.editor.set_point_source(source).unwrap();
        state.guide_advance();
        assert_eq!(STEPS[state.guide.step].title, "The source");
        state.guide_update(1.0);
        assert!(!state.guide_step_done(), "the early drag counted");
        source.position = source.position + Point2::new(0.0, 0.2);
        state.editor.set_point_source(source).unwrap();
        state.guide_update(1.0);
        assert!(state.guide_step_done());
    }

    /// The arrow runs from the card's edge to the lit rect's edge, and is
    /// left out when the two meet.
    #[test]
    fn the_pointer_runs_between_the_cards_edge_and_the_lit_rects_edge() {
        let card = Rect::from_min_size(Pos2::new(200.0, 500.0), egui::vec2(400.0, 100.0));
        let above = Rect::from_min_size(Pos2::new(380.0, 40.0), egui::vec2(60.0, 24.0));
        let (from, to) = pointer_between(card, above).unwrap();
        assert_eq!(from, card.center_top());
        assert_eq!(to, above.center_bottom());
        let aside = Rect::from_min_size(Pos2::new(700.0, 520.0), egui::vec2(40.0, 40.0));
        let (from, to) = pointer_between(card, aside).unwrap();
        assert_eq!(from, card.right_center());
        assert_eq!(to, aside.left_center());
        let touching = Rect::from_min_size(Pos2::new(590.0, 540.0), egui::vec2(40.0, 40.0));
        assert_eq!(pointer_between(card, touching), None);

        // The curve leaves and arrives square to the edges.
        let (from, to) = pointer_between(card, above).unwrap();
        assert_eq!(edge_normal(card, from), egui::vec2(0.0, -1.0));
        assert_eq!(edge_normal(above, to), egui::vec2(0.0, 1.0));
        let (from, to) = pointer_between(card, aside).unwrap();
        assert_eq!(edge_normal(card, from), egui::vec2(1.0, 0.0));
        assert_eq!(edge_normal(aside, to), egui::vec2(-1.0, 0.0));
    }

    /// With the right-hand wall selected and Edit open, the panel records
    /// where it drew the wall's boundary law picker, which the Edit step
    /// lights.
    #[test]
    fn the_edit_panel_records_the_boundary_law_picker_for_a_selected_wall() {
        let mut state = Playground::default();
        state.start_guide(false);
        state.inspector = Some(InspectorPanel::Edit);
        state.selection =
            TopologySelection::Spans([TopologySpanTarget::Outer(OuterSide::Right)].into());
        let context = egui::Context::default();
        theme::apply(&context);
        for _ in 0..2 {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1400.0, 900.0),
                )),
                time: Some(0.1),
                ..egui::RawInput::default()
            };
            state.spotlights.clear();
            let _ = context.run_ui(input, |ui| state.side_panel(ui));
        }
        let picker = state
            .spotlights
            .rect(Spotlight::BoundaryLaw)
            .expect("the picker's rect");
        assert!(picker.is_positive());
        let step = &STEPS[step_index("Edit")];
        assert_eq!(
            state.guide_spotlight(step, viewport()),
            Some((picker, Some("Pick an outgoing law for it.")))
        );
    }

    /// The dimming covers the screen but for the holes, each point once.
    #[test]
    fn the_dimming_covers_everything_but_the_holes_once() {
        let screen = Rect::from_min_size(Pos2::ZERO, egui::vec2(800.0, 600.0));
        let holes = [
            Rect::from_min_size(Pos2::new(100.0, 50.0), egui::vec2(60.0, 30.0)),
            Rect::from_min_size(Pos2::new(700.0, 40.0), egui::vec2(50.0, 50.0)),
            Rect::from_min_size(Pos2::new(300.0, 400.0), egui::vec2(200.0, 100.0)),
            // One reaching past the screen's edge.
            Rect::from_min_size(Pos2::new(780.0, 580.0), egui::vec2(60.0, 60.0)),
        ];
        let shades = dim_except(screen, &holes);
        let mut covered = 0.0;
        for shade in &shades {
            assert!(screen.contains_rect(*shade));
            covered += shade.area();
            for hole in &holes {
                assert!(
                    !shade.intersects(hole.shrink(0.01)),
                    "{shade:?} over {hole:?}"
                );
            }
        }
        let holed: f32 = holes.iter().map(|hole| hole.intersect(screen).area()).sum();
        assert!(
            (covered + holed - screen.area()).abs() < 1.0,
            "{covered} + {holed}"
        );
        for y in (5..600).step_by(37) {
            for x in (5..800).step_by(29) {
                let at = Pos2::new(x as f32, y as f32);
                let in_hole = holes.iter().any(|hole| hole.contains(at));
                let shaded = shades.iter().filter(|shade| shade.contains(at)).count();
                assert_eq!(shaded, usize::from(!in_hole), "at {at:?}");
            }
        }
        assert!(dim_except(screen, &[]).len() == 1);
    }

    /// The card shows the step and Skip ends the tour.
    #[test]
    fn the_card_shows_the_step_and_skip_ends_the_tour() {
        let mut state = Playground::default();
        state.start_guide(false);
        let context = egui::Context::default();
        theme::apply(&context);
        context.enable_accesskit();
        let mut time = 0.0;
        let mut frame = |state: &mut Playground, events: Vec<egui::Event>| {
            time += 0.05;
            let input = egui::RawInput {
                screen_rect: Some(viewport()),
                time: Some(time),
                events,
                ..egui::RawInput::default()
            };
            context.run_ui(input, |ui| state.guide_frame(ui.ctx(), viewport()))
        };
        frame(&mut state, vec![]);
        let output = frame(&mut state, vec![]);
        let shown = laid_out(&output);
        assert!(
            shown.iter().any(|widget| widget.label == STEPS[0].title),
            "{shown:?}"
        );
        let skip = shown
            .iter()
            .find(|widget| widget.label == "Skip the tour")
            .expect("the skip button");
        frame(&mut state, click(skip));
        frame(&mut state, vec![]);
        assert!(!state.guide.active && state.guide.finished);
    }
}
