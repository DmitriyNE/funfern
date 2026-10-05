//! The guided tour. A first launch, with nothing to restore and the tour
//! not yet seen, opens a small scene and lays the tour over it: a step at a
//! time, each naming one thing to do and the control to do it with, spotlit
//! where the bar or the panel drew it this frame, so the light follows the
//! bar's folds; a step moves on a moment after the state shows the action
//! done, a refused line or an invalid scene lights the way back first, and
//! Next and Skip are always there. By the end the panels have each been
//! opened once and said what they hold. The tour is back under Examples
//! and on the scene card; seen once is a marker beside the autosave.

use super::*;
use funfern_app::document::VectorOverlay;
use funfern_app::topology_editor::TopologyAcceptance;
use funfern_app::topology_examples::{
    GUIDE_GLASS, GUIDE_OBSTACLE, GUIDE_REGION, GUIDE_REGION_CURVE, guide_scene,
};
use funfern_app::topology_viewport::TopologySpanTarget;

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
    /// The Draw palette's Undo point and Cancel, while a line is drawn.
    DrawBack,
    /// The bar's Undo.
    Undo,
    /// The Simulation panel's source frequency and its Pulse choice.
    SourceFrequency,
    SourcePulse,
    /// The source, where the viewport draws it.
    Source,
    /// The obstacle, where the viewport draws it.
    Obstacle,
    /// The gizmo's grips round the selected obstacle: the four-way grip,
    /// the square that stretches sideways, and a spot on the ring.
    GizmoMove,
    GizmoStretch,
    GizmoTurn,
    /// The round region, where the viewport draws it.
    Region,
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
    /// The light on the obstacle, while it waits for a marquee, is a dashed
    /// rectangle, a marquee's shape, rather than a glow.
    dashed: bool,
    /// What the step is about, which has to be in the scene for it to be
    /// done: one that is gone holds the step until it is back.
    needs: &'static [GuideSubject],
    /// Actions the step asks for together, each ticked on the card as it
    /// is seen, with where it is done: `Guide::ticks` holds them.
    checklist: &'static [(&'static str, &'static str)],
    done: fn(&Playground) -> bool,
}

/// A part of the tour's scene a step is about.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum GuideSubject {
    Source,
    Obstacle,
    Region,
    Glass,
}

impl GuideSubject {
    /// What the card says when it is missing.
    fn missing(self) -> &'static str {
        match self {
            Self::Source => "The source is switched off",
            Self::Obstacle => "The round obstacle is gone",
            Self::Region => "The round region is gone",
            Self::Glass => "The glass is gone from the materials",
        }
    }
}

/// A done step waits this long before moving on, so its tick is seen.
const GUIDE_ADVANCE_DELAY: f64 = 0.9;
/// What ticks the transform step's checklist, read from the map that best
/// takes the obstacle as the step began onto it now: its centre moved this
/// far, its two stretches this far apart, or a turn this large.
const TICK_SHIFT: f64 = 0.05;
const TICK_STRETCH: f64 = 1.15;
const TICK_TURN: f64 = 15.0 * std::f64::consts::PI / 180.0;
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
        dashed: false,
        needs: &[],
        checklist: &[],
        done: |state| state.guide.paused_seen && state.wave_running,
    },
    GuideStep {
        title: "The source",
        text: "The circled point is the source; the Simulation panel sets its signal. \
               Drag it somewhere else and watch the field follow.",
        targets: &[(Spotlight::Source, None)],
        dashed: false,
        needs: &[GuideSubject::Source],
        checklist: &[],
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
            (
                Spotlight::Wall,
                Some("Click the right-hand wall to select it."),
            ),
            (Spotlight::Tab(InspectorPanel::Edit), None),
            (Spotlight::Panels, PANELS_HINT),
        ],
        dashed: false,
        needs: &[],
        checklist: &[],
        done: |state| {
            state.editor.document.model.draft.outer_boundaries.sides[OuterSide::Right.index()]
                != state.guide.baseline.right_wall
        },
    },
    GuideStep {
        title: "Select and transform",
        text: "Press on empty space beside the round obstacle, drag a rectangle over the \
               whole of it and let go: that is a marquee, and everything inside it is \
               selected. Handles appear round it, the gizmo; the dot at its centre is the \
               pivot the others work about, and dragging the dot moves the pivot.",
        targets: &[
            (
                Spotlight::GizmoMove,
                Some("Now move it: drag the four-way grip, or the obstacle itself."),
            ),
            (
                Spotlight::GizmoStretch,
                Some(
                    "Now stretch it into an oval with the square on the right; the top one \
                     stretches up and down, the corner one both alike.",
                ),
            ),
            (
                Spotlight::GizmoTurn,
                Some("Now turn it: drag the ring round."),
            ),
            (
                Spotlight::Obstacle,
                Some("Drag a rectangle over the whole of it."),
            ),
        ],
        dashed: true,
        needs: &[GuideSubject::Obstacle],
        checklist: &[
            ("Move it", "the four-way grip, bottom left"),
            ("Stretch it", "the square on the right"),
            ("Turn it", "the ring"),
        ],
        done: |state| state.guide.ticks.iter().all(|tick| *tick) && !state.editor.editing(),
    },
    GuideStep {
        title: "Draw",
        text: "+ Draw opens the palette. Under Open curve pick Polyline, then click two or \
               three points across the scene; each click adds a vertex. Press Finish in the \
               palette, or Enter, to end the line, which becomes a wall. If the scene cannot \
               take the line, this card says why and how to take it back.",
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
        dashed: false,
        needs: &[],
        checklist: &[],
        done: |state| {
            state.editor.document.model.draft.geometry.curves.len() > state.guide.baseline.curves
        },
    },
    GuideStep {
        title: "Materials",
        text: "Materials holds the library and says which region has which. Open it, \
               click inside the round region at the bottom to select it, and give it the \
               glass.",
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
                Spotlight::Region,
                Some("Click inside the round region to select it."),
            ),
            (
                Spotlight::Tab(InspectorPanel::Materials),
                Some("Open the Materials panel first."),
            ),
            (Spotlight::Panels, PANELS_HINT),
        ],
        dashed: false,
        needs: &[GuideSubject::Region, GuideSubject::Glass],
        checklist: &[],
        done: |state| {
            state
                .editor
                .document
                .model
                .draft
                .region(GUIDE_REGION)
                .map(|region| region.material)
                != state.guide.baseline.region_material
                && state.region_selection == GUIDE_REGION
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
            (Spotlight::VectorOverlay, Some("Pick the Poynting flow.")),
            (Spotlight::Tab(InspectorPanel::View), None),
            (Spotlight::Panels, PANELS_HINT),
        ],
        dashed: false,
        needs: &[],
        checklist: &[],
        done: |state| state.editor.document.presentation.vector_overlay != VectorOverlay::Off,
    },
    GuideStep {
        title: "Simulation",
        text: "Simulation holds the mesh size and adaptation, the solver's settings and the \
               source's signal; the status line below says what is being rebuilt. Open it, \
               change the source's frequency and watch the waves' length follow, then \
               switch it to Pulse: bursts leave the source, one after another.",
        targets: &[
            (
                Spotlight::SourceFrequency,
                Some("Drag the Hz field, or click it and type."),
            ),
            (Spotlight::SourcePulse, Some("Now pick Pulse.")),
            (
                Spotlight::Tab(InspectorPanel::Simulation),
                Some("Open the Simulation panel first."),
            ),
            (Spotlight::Panels, PANELS_HINT),
        ],
        dashed: false,
        needs: &[GuideSubject::Source],
        checklist: &[],
        done: |state| {
            let signal = state.editor.document.model.source.signal;
            signal.is_pulsed() && signal.carrier()[2] != state.guide.baseline.source_frequency
        },
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
        dashed: false,
        needs: &[],
        checklist: &[],
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
        dashed: false,
        needs: &[],
        checklist: &[],
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
    pub(super) source_frequency: f64,
}

/// Something done along the way that the scene cannot take, which has to
/// be taken back before the tour goes on: the control that takes it back,
/// and what the card says.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct GuideSetback {
    pub(super) target: Spotlight,
    pub(super) text: String,
    /// Whether the card offers the tour's scene back, for a step's subject
    /// gone with edits made since, which Undo alone would unwind too.
    pub(super) restore: bool,
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
    /// Whether the step has been held for a missing subject: the baseline is
    /// taken again once it is back, since the one taken without it would
    /// count its return as the step's action.
    held: bool,
    /// The current step's checklist, ticked as each action is seen and
    /// kept ticked.
    pub(super) ticks: [bool; 3],
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
                    held: false,
                    ticks: [false; 3],
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
            source_frequency: model.source.signal.carrier()[2],
        }
    }

    /// What has to be taken back first: a line Finish refused, kept for a
    /// point to be taken back or the line given up, or a draft the topology
    /// check found invalid, which Undo takes back.
    pub(super) fn guide_setback(&self) -> Option<GuideSetback> {
        if let Some(reason) = self.draw.as_ref().and_then(DrawGesture::refusal) {
            return Some(GuideSetback {
                target: Spotlight::DrawBack,
                text: format!(
                    "The scene cannot take this line: {reason}. Undo point, or Backspace, \
                     takes its last point back; Cancel, or Esc, gives the line up."
                ),
                restore: false,
            });
        }
        if let TopologyAcceptance::Invalid(issue) = self.editor.acceptance {
            return Some(GuideSetback {
                target: Spotlight::Undo,
                text: format!("The scene cannot take that change: {issue}. Undo takes it back."),
                restore: false,
            });
        }
        self.guide_missing().map(|subject| GuideSetback {
            target: Spotlight::Undo,
            text: format!(
                "{}, and this step needs it. Undo brings it back, or Restore the scene \
                 puts the tour's scene back as it started.",
                subject.missing()
            ),
            restore: true,
        })
    }

    /// The first of the current step's subjects missing from the scene.
    pub(super) fn guide_missing(&self) -> Option<GuideSubject> {
        let model = &self.editor.document.model;
        let present = |subject: GuideSubject| match subject {
            GuideSubject::Source => model.source.enabled,
            GuideSubject::Obstacle => model
                .draft
                .geometry
                .curves
                .iter()
                .any(|curve| curve.id == GUIDE_OBSTACLE),
            GuideSubject::Region => model.draft.region(GUIDE_REGION).is_some(),
            GuideSubject::Glass => model
                .draft
                .materials
                .iter()
                .any(|material| material.id == GUIDE_GLASS),
        };
        STEPS
            .get(self.guide.step)
            .filter(|_| self.guide.active)?
            .needs
            .iter()
            .copied()
            .find(|subject| !present(*subject))
    }

    /// The tour's scene again, as one undoable scene change, on the same
    /// step, whose baseline it takes afresh.
    pub(super) fn restore_guide_scene(&mut self) {
        match self.set_document(guide_scene(), true, true) {
            Ok(()) => {
                self.guide.done_since = None;
                self.guide.paused_seen = false;
                self.guide.baseline = self.guide_baseline();
                self.notify("The tour's scene is back");
            }
            Err(error) => self.notify(error),
        }
    }

    /// The obstacle's control points, which any move, turn or scaling of it
    /// changes; empty once it is gone.
    pub(super) fn guide_obstacle_controls(&self) -> Vec<Point2> {
        self.guide_curve_controls(GUIDE_OBSTACLE)
    }

    fn guide_curve_controls(&self, id: CurveId) -> Vec<Point2> {
        self.editor
            .document
            .model
            .draft
            .geometry
            .curves
            .iter()
            .find(|curve| curve.id == id)
            .map(|curve| match &curve.spline {
                CurveSpline::Closed(spline) => spline.controls().to_vec(),
                CurveSpline::Open(spline) => spline.controls().to_vec(),
            })
            .unwrap_or_default()
    }

    /// The screen rect round a curve's controls, padded; none once it is
    /// gone.
    fn guide_curve_rect(&self, id: CurveId, viewport: Rect) -> Option<Rect> {
        let controls = self.guide_curve_controls(id);
        (!controls.is_empty()).then(|| {
            let mut rect = Rect::NOTHING;
            for control in controls {
                rect.extend_with(self.screen(control, viewport));
            }
            rect.expand(10.0)
        })
    }

    /// Whether the whole obstacle is selected, so the gizmo is round it.
    pub(super) fn guide_obstacle_selected(&self) -> bool {
        let Some(selected) = self.selection.spans() else {
            return false;
        };
        self.editor
            .document
            .model
            .draft
            .geometry
            .curves
            .iter()
            .find(|curve| curve.id == GUIDE_OBSTACLE)
            .is_some_and(|curve| {
                curve
                    .spans
                    .iter()
                    .all(|span| selected.contains(&TopologySpanTarget::Curve(span.id)))
            })
    }

    /// Whether the current step's action shows in the state, in a scene
    /// the topology check has accepted: an edit it is still checking, or
    /// one it refused, moves no step on.
    pub(super) fn guide_step_done(&self) -> bool {
        self.guide.active
            && self.editor.acceptance == TopologyAcceptance::Valid
            && self.guide_setback().is_none()
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
        if self.guide_missing().is_some() {
            self.guide.held = true;
        } else if self.guide.held {
            self.guide.held = false;
            self.guide.baseline = self.guide_baseline();
        }
        if !STEPS[self.guide.step].checklist.is_empty()
            && let Some(change) = fitted_change(
                &self.guide.baseline.obstacle,
                &self.guide_obstacle_controls(),
            )
        {
            let ticks = &mut self.guide.ticks;
            ticks[0] |= change.shift > TICK_SHIFT;
            ticks[1] |= change.stretch > TICK_STRETCH;
            ticks[2] |= change.turn > TICK_TURN;
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
            self.guide.ticks = [false; 3];
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
            // The marquee's cue until the obstacle is selected; its grips
            // after, each until its action is ticked.
            Spotlight::Obstacle if self.guide_obstacle_selected() => None,
            Spotlight::Obstacle => self.guide_curve_rect(GUIDE_OBSTACLE, viewport),
            Spotlight::GizmoMove | Spotlight::GizmoStretch | Spotlight::GizmoTurn => {
                let index = match target {
                    Spotlight::GizmoMove => 0,
                    Spotlight::GizmoStretch => 1,
                    _ => 2,
                };
                if self.guide.ticks[index] || !self.guide_obstacle_selected() {
                    return None;
                }
                let (_, center, radius, x_radius, _) = self.transform_gizmo(viewport)?;
                let diagonal = radius * std::f32::consts::FRAC_1_SQRT_2;
                let (at, size) = match target {
                    Spotlight::GizmoMove => (egui::vec2(-diagonal, diagonal), 22.0),
                    Spotlight::GizmoStretch => (egui::vec2(x_radius, 0.0), 20.0),
                    // A spot on the ring where no grip is.
                    _ => (egui::vec2(-diagonal, -diagonal), 22.0),
                };
                Some(Rect::from_center_size(center + at, egui::vec2(size, size)))
            }
            // Lit once Materials is open and until the region is selected,
            // and its material list only after that, so the step reads:
            // the panel, the region, its list.
            Spotlight::Region
                if self.inspector != Some(InspectorPanel::Materials)
                    || self.region_selection == GUIDE_REGION =>
            {
                None
            }
            Spotlight::Region => self.guide_curve_rect(GUIDE_REGION_CURVE, viewport),
            Spotlight::RegionMaterial(GUIDE_REGION)
            | Spotlight::MaterialChoice {
                region: GUIDE_REGION,
                ..
            } if self.region_selection != GUIDE_REGION => None,
            // Lit once Edit is open, so the step reads Edit first, then the
            // wall.
            Spotlight::Wall if self.inspector != Some(InspectorPanel::Edit) => None,
            Spotlight::Wall => {
                let domain = self.editor.document.model.draft.geometry.domain;
                let top = self.screen(Point2::new(domain.max_x, domain.max_y), viewport);
                let bottom = self.screen(Point2::new(domain.max_x, domain.min_y), viewport);
                Some(Rect::from_two_pos(top, bottom).expand(8.0))
            }
            Spotlight::Tab(panel) if self.inspector == Some(panel) => None,
            // Each half of the Simulation step goes dark once done.
            Spotlight::SourceFrequency
                if self.editor.document.model.source.signal.carrier()[2]
                    != self.guide.baseline.source_frequency =>
            {
                None
            }
            Spotlight::SourcePulse if self.editor.document.model.source.signal.is_pulsed() => None,
            other => self.spotlights.rect(other),
        }
    }

    /// What the tour lights this frame: the control that takes a setback
    /// back, the current step's spotlight, or nothing once the step is done.
    pub(super) fn guide_light(&self, viewport: Rect) -> Option<(Rect, Option<&'static str>)> {
        if !self.guide.active {
            return None;
        }
        if let Some(setback) = self.guide_setback() {
            return self
                .spotlight_rect(setback.target, viewport)
                .map(|rect| (rect, None));
        }
        if self.guide_step_done() {
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
        let setback = self.guide_setback();

        // A done step lights nothing while it waits to move on: its chain
        // of targets would otherwise fall back to an earlier control, the
        // Polyline tool after a line was finished, for the moment the tick
        // shows.
        let spotlight = self.guide_light(viewport);
        // The marquee's dashed cue is for the obstacle while it waits to be
        // selected; the grips after it glow as every other control does.
        let marquee = step.dashed
            && setback.is_none()
            && spotlight.is_some_and(|(rect, _)| {
                self.spotlight_rect(Spotlight::Obstacle, viewport) == Some(rect)
            });
        let screen = ctx.content_rect();
        let holes = spotlight
            .iter()
            .map(|(rect, _)| rect.expand(6.0).intersect(screen))
            .collect::<Vec<_>>();
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
                if marquee {
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
        let ticks = self.guide.ticks;
        let (index, total) = (self.guide.step + 1, STEPS.len());
        let last = index == total;
        let mut skip = false;
        let mut next = false;
        let mut restore = false;
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
                // The checklist: a drawn tick for each action seen, the next
                // one in bold with a teal ring, the rest a grey ring.
                let next_item = ticks.iter().position(|tick| !tick);
                for (item, (action, place)) in step.checklist.iter().enumerate() {
                    let ticked = ticks[item];
                    ui.horizontal(|ui| {
                        ui.add_space(8.0);
                        let (mark, _) =
                            ui.allocate_exact_size(egui::vec2(16.0, 16.0), egui::Sense::hover());
                        let middle = mark.center();
                        if ticked {
                            ui.painter().circle_filled(middle, 7.0, TEAL);
                            ui.painter().add(egui::Shape::line(
                                vec![
                                    middle + egui::vec2(-3.5, 0.2),
                                    middle + egui::vec2(-1.0, 2.8),
                                    middle + egui::vec2(3.8, -2.6),
                                ],
                                Stroke::new(1.8, Color32::from_rgb(16, 23, 31)),
                            ));
                        } else {
                            let ring = if next_item == Some(item) {
                                TEAL
                            } else {
                                Color32::from_gray(120)
                            };
                            ui.painter()
                                .circle_stroke(middle, 6.5, Stroke::new(1.5, ring));
                        }
                        let action = egui::RichText::new(*action);
                        ui.label(if ticked {
                            action.color(TEAL)
                        } else if next_item == Some(item) {
                            action.strong()
                        } else {
                            action
                        });
                        ui.weak(*place);
                    });
                }
                if let Some(setback) = &setback {
                    ui.colored_label(GOLD, &setback.text);
                    if setback.restore && ui.button("Restore the scene").clicked() {
                        restore = true;
                    }
                } else if let Some((_, Some(hint))) = spotlight {
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
        if restore {
            self.restore_guide_scene();
        }
        if skip {
            self.finish_guide();
        } else if next {
            self.guide_advance();
        }
    }
}

/// How the obstacle changed: how far its centre moved, how far apart the
/// two stretches of the map are, as a ratio, and how far it turned, in
/// radians.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct FittedChange {
    pub(super) shift: f64,
    pub(super) stretch: f64,
    pub(super) turn: f64,
}

/// The affine map that best takes `from` onto `to`, point for point, by
/// least squares, read as a change: exact for the gizmo's moves, turns and
/// stretches and any sequence of them. None when the points do not pair
/// up or do not span the plane.
pub(super) fn fitted_change(from: &[Point2], to: &[Point2]) -> Option<FittedChange> {
    if from.len() != to.len() || from.len() < 3 {
        return None;
    }
    let count = from.len() as f64;
    let mean = |points: &[Point2]| {
        let sum = points
            .iter()
            .fold(Point2::new(0.0, 0.0), |sum, point| sum + *point);
        sum * (1.0 / count)
    };
    let (from_mean, to_mean) = (mean(from), mean(to));
    // A = (Σ q pᵀ)(Σ p pᵀ)⁻¹ over the centred points.
    let (mut qp, mut pp) = ([[0.0; 2]; 2], [[0.0; 2]; 2]);
    for (p, q) in from.iter().zip(to) {
        let (p, q) = (*p - from_mean, *q - to_mean);
        let (p, q) = ([p.x, p.y], [q.x, q.y]);
        for row in 0..2 {
            for column in 0..2 {
                qp[row][column] += q[row] * p[column];
                pp[row][column] += p[row] * p[column];
            }
        }
    }
    let determinant = pp[0][0] * pp[1][1] - pp[0][1] * pp[1][0];
    if determinant.abs() <= f64::EPSILON {
        return None;
    }
    let inverse = [
        [pp[1][1] / determinant, -pp[0][1] / determinant],
        [-pp[1][0] / determinant, pp[0][0] / determinant],
    ];
    let mut a = [[0.0; 2]; 2];
    for row in 0..2 {
        for column in 0..2 {
            a[row][column] = qp[row][0] * inverse[0][column] + qp[row][1] * inverse[1][column];
        }
    }
    // The turn is the rotation of A's polar decomposition; the stretches
    // are its singular values.
    let turn = (a[1][0] - a[0][1]).atan2(a[0][0] + a[1][1]).abs();
    let frobenius = a.iter().flatten().map(|value| value * value).sum::<f64>();
    let area = (a[0][0] * a[1][1] - a[0][1] * a[1][0]).abs();
    let spread = (frobenius * frobenius - 4.0 * area * area).max(0.0).sqrt();
    let (larger, smaller) = (
        ((frobenius + spread) / 2.0).sqrt(),
        ((frobenius - spread) / 2.0).max(0.0).sqrt(),
    );
    Some(FittedChange {
        shift: (to_mean - from_mean).norm(),
        stretch: if smaller > 0.0 {
            larger / smaller
        } else {
            f64::INFINITY
        },
        turn,
    })
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
        RigidTransform, TopologySpanTarget, plan_axis_scale, plan_rigid_transform,
    };

    /// Each step is done by the action it asks for, read from the state,
    /// and a done step moves on after its moment, the last one ending the
    /// tour.
    fn step_index(title: &str) -> usize {
        STEPS.iter().position(|step| step.title == title).unwrap()
    }

    #[derive(Clone, Copy, Debug, PartialEq)]
    enum Drag {
        Move,
        Stretch,
        Turn,
    }

    /// One of the gizmo's drags on the whole obstacle, as one edit about
    /// its centre: what the four-way grip, the side square and the ring do.
    fn gizmo_drag(state: &mut Playground, drag: Drag) {
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
        let controls = state.guide_obstacle_controls();
        let pivot = controls
            .iter()
            .fold(Point2::new(0.0, 0.0), |sum, point| sum + *point)
            * (1.0 / controls.len() as f64);
        let rigid = |translation, rotation_radians| RigidTransform {
            pivot,
            translation,
            rotation_radians,
            scale: 1.0,
        };
        let updates = match drag {
            Drag::Move => {
                plan_rigid_transform(&geometry, &selected, rigid(Point2::new(-0.15, 0.0), 0.0))
            }
            Drag::Stretch => plan_axis_scale(&geometry, &selected, pivot, 1.4, 1.0),
            Drag::Turn => plan_rigid_transform(&geometry, &selected, rigid(Point2::default(), 0.6)),
        }
        .unwrap();
        state.editor.begin();
        state
            .editor
            .apply_transform_updates_during_edit(&updates)
            .unwrap();
        state.editor.commit();
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
                settle(&mut state.editor);
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
            // What the gizmo does on its three drags: moved, stretched
            // sideways and turned, each an edit of its own.
            // Not done before all three; the last one's update is the
            // sequence's own, whose clock moves the tour on.
            for drag in [Drag::Move, Drag::Stretch] {
                gizmo_drag(state, drag);
                settle(&mut state.editor);
                state.guide_update(0.0);
                assert!(!state.guide_step_done(), "done at {drag:?}");
            }
            gizmo_drag(state, Drag::Turn);
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
            settle(&mut state.editor);
            state.guide_update(0.0);
            assert!(
                !state.guide_step_done(),
                "the glass set without the region picked"
            );
            state.select_region(GUIDE_REGION);
        });
        expect_step(&mut state, 6, &|state| {
            state.editor.document.presentation.vector_overlay = VectorOverlay::RelativeEnergyFlow;
        });
        expect_step(&mut state, 7, &|state| {
            let mut source = state.editor.document.model.source;
            *source.signal.carrier_mut().2 += 6.0;
            state.editor.set_point_source(source).unwrap();
            assert!(!state.guide_step_done(), "the frequency alone");
            source.signal = pulse_from(source.signal, SignalUse::Source, 0.0);
            state.editor.set_point_source(source).unwrap();
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

    /// A line Finish refuses keeps the tour where it is and lights the
    /// palette's way back, Undo point and Cancel, with the refusal on the
    /// card; a point taken back clears it.
    #[test]
    fn a_refused_line_lights_its_way_back() {
        let mut state = Playground::default();
        state.start_guide(false);
        settle(&mut state.editor);
        state.guide.step = step_index("Draw");
        state.guide.baseline = state.guide_baseline();
        state.begin_draw(DrawTool::Polyline);
        // Across the obstacle, which a wall cannot cross.
        for point in [Point2::new(0.0, 0.3), Point2::new(0.5, 0.3)] {
            let draw = state.draw.as_mut().unwrap();
            draw.points.push(point);
            draw.attachments.push(None);
        }
        state.finish_draw();
        let setback = state.guide_setback().expect("the refusal is a setback");
        assert_eq!(setback.target, Spotlight::DrawBack);
        assert!(setback.text.contains("touch"), "{}", setback.text);
        let viewport = viewport();
        let back = Rect::from_min_size(Pos2::new(300.0, 120.0), egui::vec2(140.0, 20.0));
        state.spotlights.record(Spotlight::DrawBack, back);
        state.spotlights.record(
            Spotlight::DrawFinish,
            back.translate(egui::vec2(-60.0, 0.0)),
        );
        assert_eq!(state.guide_light(viewport), Some((back, None)));
        state.guide_update(10.0);
        assert!(!state.guide_step_done());
        state.undo_draw_point();
        assert_eq!(state.guide_setback(), None, "a point taken back clears it");
        assert_eq!(
            state.guide_light(viewport).map(|(rect, _)| rect),
            Some(back.translate(egui::vec2(-60.0, 0.0))),
            "the step's own light is back"
        );
    }

    /// A scene the topology check finds invalid holds every step, done or
    /// not, and lights the bar's Undo; Undo takes it back.
    #[test]
    fn an_invalid_scene_holds_the_tour_and_lights_undo() {
        let mut state = Playground::default();
        state.start_guide(false);
        settle(&mut state.editor);
        state.guide.step = step_index("Select and transform");
        state.guide.baseline = state.guide_baseline();
        // The obstacle's control dragged out of the domain: moved, which
        // the step asks for, but invalid.
        state.editor.begin();
        state
            .editor
            .set_control(GUIDE_OBSTACLE, 0, Point2::new(4.0, 0.0))
            .unwrap();
        state.editor.commit();
        settle(&mut state.editor);
        assert!(matches!(
            state.editor.acceptance,
            TopologyAcceptance::Invalid(_)
        ));
        let setback = state
            .guide_setback()
            .expect("an invalid scene is a setback");
        assert_eq!(setback.target, Spotlight::Undo);
        let undo = Rect::from_min_size(Pos2::new(8.0, 8.0), egui::vec2(28.0, 22.0));
        state.spotlights.record(Spotlight::Undo, undo);
        assert_eq!(state.guide_light(viewport()), Some((undo, None)));
        for now in [10.0, 12.0] {
            state.guide_update(now);
        }
        assert!(!state.guide_step_done());
        assert_eq!(state.guide.step, step_index("Select and transform"));
        state.undo();
        settle(&mut state.editor);
        assert_eq!(state.guide_setback(), None);
        assert_eq!(state.editor.acceptance, TopologyAcceptance::Valid);
    }

    /// A step whose subject is gone is held, the deletion not counting as
    /// its action, with Undo lit and the scene offered back; once Undo
    /// brings the subject back the step starts from there, so its return
    /// does not count either.
    #[test]
    fn a_missing_subject_holds_its_step() {
        let at = |title: &str| {
            let mut state = Playground::default();
            state.start_guide(false);
            settle(&mut state.editor);
            state.guide.step = step_index(title);
            state.guide.baseline = state.guide_baseline();
            state
        };
        let held = |state: &mut Playground, subject: GuideSubject| {
            settle(&mut state.editor);
            for now in [10.0, 12.0] {
                state.guide_update(now);
            }
            assert_eq!(state.guide_missing(), Some(subject));
            assert!(!state.guide_step_done(), "{subject:?} gone counted as done");
            let setback = state.guide_setback().unwrap();
            assert_eq!(setback.target, Spotlight::Undo);
            assert!(setback.restore);
            assert!(
                setback.text.starts_with(subject.missing()),
                "{}",
                setback.text
            );
        };

        // The obstacle deleted during its step, and before it.
        let mut state = at("Select and transform");
        state.editor.remove_curve(GUIDE_OBSTACLE, None).unwrap();
        held(&mut state, GuideSubject::Obstacle);
        let mut state = at("Edit");
        state.editor.remove_curve(GUIDE_OBSTACLE, None).unwrap();
        settle(&mut state.editor);
        state.guide_advance();
        assert_eq!(state.guide.step, step_index("Select and transform"));
        held(&mut state, GuideSubject::Obstacle);
        state.undo();
        settle(&mut state.editor);
        state.guide_update(14.0);
        assert_eq!(state.guide_setback(), None);
        assert!(
            !state.guide_step_done(),
            "the obstacle's return counted as a move"
        );

        // The region, deleted into the background, and the glass.
        let mut state = at("Materials");
        let region_curve = state
            .editor
            .document
            .model
            .draft
            .geometry
            .curves
            .iter()
            .find(|curve| curve.id != GUIDE_OBSTACLE)
            .unwrap()
            .id;
        state
            .editor
            .remove_curve(region_curve, Some(BACKGROUND_REGION))
            .unwrap();
        held(&mut state, GuideSubject::Region);
        // Restore puts the scene back on the same step, from which the
        // step's action counts again.
        state.restore_guide_scene();
        settle(&mut state.editor);
        state.guide_update(14.0);
        assert_eq!(state.guide.step, step_index("Materials"));
        assert_eq!(state.guide_setback(), None);
        assert!(!state.guide_step_done());
        state
            .editor
            .set_region_material(GUIDE_REGION, GUIDE_GLASS)
            .unwrap();
        state.select_region(GUIDE_REGION);
        settle(&mut state.editor);
        state.guide_update(15.0);
        assert!(state.guide_step_done());
        let mut state = at("Materials");
        state.editor.delete_material(GUIDE_GLASS).unwrap();
        held(&mut state, GuideSubject::Glass);

        // The source switched off.
        for title in ["The source", "Simulation"] {
            let mut state = at(title);
            let mut source = state.editor.document.model.source;
            source.enabled = false;
            source.position = source.position + Point2::new(0.2, 0.0);
            state.editor.set_point_source(source).unwrap();
            held(&mut state, GuideSubject::Source);
        }
    }

    /// The fitted change reads the gizmo's moves, stretches and turns back
    /// exactly, alone and composed, and a uniform scaling stretches nothing.
    #[test]
    fn the_fitted_change_reads_a_move_a_stretch_and_a_turn() {
        let from = PeriodicCubicSpline::rounded(Point2::new(0.25, 0.3), 0.18)
            .controls()
            .to_vec();
        let map = |scale_x: f64, scale_y: f64, turn: f64, shift: Point2| {
            from.iter()
                .map(|point| {
                    let (x, y) = (point.x * scale_x, point.y * scale_y);
                    let (sin, cos) = turn.sin_cos();
                    Point2::new(cos * x - sin * y, sin * x + cos * y) + shift
                })
                .collect::<Vec<_>>()
        };
        let close = |a: f64, b: f64| (a - b).abs() < 1.0e-9;
        let change = fitted_change(&from, &from).unwrap();
        assert!(close(change.shift, 0.0) && close(change.stretch, 1.0) && close(change.turn, 0.0));
        let change = fitted_change(&from, &map(1.7, 1.7, 0.0, Point2::default())).unwrap();
        assert!(close(change.stretch, 1.0), "uniform: {change:?}");
        let change = fitted_change(&from, &map(1.4, 1.0, 0.6, Point2::new(-0.3, 0.2))).unwrap();
        assert!(close(change.stretch, 1.4), "{change:?}");
        assert!(close(change.turn, 0.6), "{change:?}");
        // Turned about the origin, the centre moves with the shift and the
        // turn alike.
        let mean = |points: &[Point2]| {
            points
                .iter()
                .fold(Point2::new(0.0, 0.0), |sum, point| sum + *point)
                * (1.0 / points.len() as f64)
        };
        let to = map(1.4, 1.0, 0.6, Point2::new(-0.3, 0.2));
        assert!(close(change.shift, (mean(&to) - mean(&from)).norm()));
        assert_eq!(
            fitted_change(&from, &from[..3]),
            None,
            "points that do not pair"
        );
    }

    /// The transform step lights the obstacle as a marquee's cue until it is
    /// selected, then each grip until its action is ticked, in the card's
    /// order; a tick stays when the obstacle goes back.
    #[test]
    fn the_transform_step_lights_each_grip_until_ticked() {
        let mut state = Playground::default();
        state.start_guide(false);
        settle(&mut state.editor);
        state.guide.step = step_index("Select and transform");
        state.guide.baseline = state.guide_baseline();
        let viewport = viewport();
        let lit = |state: &Playground| state.guide_light(viewport).map(|(_, hint)| hint);
        assert_eq!(
            lit(&state),
            Some(Some("Drag a rectangle over the whole of it."))
        );
        let spans = state
            .editor
            .document
            .model
            .draft
            .geometry
            .curves
            .iter()
            .find(|curve| curve.id == GUIDE_OBSTACLE)
            .unwrap()
            .spans
            .iter()
            .map(|span| TopologySpanTarget::Curve(span.id))
            .collect::<BTreeSet<_>>();
        state.selection = TopologySelection::Spans(spans);
        let (_, center, radius, _, _) = state.transform_gizmo(viewport).unwrap();
        let (grip, _) = state.guide_light(viewport).unwrap();
        assert!(
            grip.contains(center + egui::vec2(-radius, radius) * std::f32::consts::FRAC_1_SQRT_2)
        );
        for (drag, hint) in [(Drag::Move, "Now stretch"), (Drag::Stretch, "Now turn")] {
            gizmo_drag(&mut state, drag);
            settle(&mut state.editor);
            state.guide_update(1.0);
            assert!(
                lit(&state).flatten().unwrap().starts_with(hint),
                "after {drag:?}: {:?}",
                lit(&state)
            );
        }
        // Undone, the move and the stretch stay ticked.
        state.undo();
        state.undo();
        settle(&mut state.editor);
        state.guide_update(2.0);
        assert_eq!(state.guide.ticks, [true, true, false]);
        gizmo_drag(&mut state, Drag::Turn);
        settle(&mut state.editor);
        state.guide_update(3.0);
        assert!(state.guide_step_done());
    }

    /// The Simulation step lights the frequency until it is changed, then
    /// Pulse.
    #[test]
    fn the_simulation_step_lights_the_frequency_then_pulse() {
        let mut state = Playground::default();
        state.start_guide(false);
        settle(&mut state.editor);
        state.guide.step = step_index("Simulation");
        state.guide.baseline = state.guide_baseline();
        state.inspector = Some(InspectorPanel::Simulation);
        let viewport = viewport();
        let frequency = Rect::from_min_size(Pos2::new(900.0, 400.0), egui::vec2(80.0, 20.0));
        let pulse = Rect::from_min_size(Pos2::new(960.0, 370.0), egui::vec2(50.0, 20.0));
        state
            .spotlights
            .record(Spotlight::SourceFrequency, frequency);
        state.spotlights.record(Spotlight::SourcePulse, pulse);
        let lit = |state: &Playground| state.guide_light(viewport).map(|(rect, _)| rect);
        assert_eq!(lit(&state), Some(frequency));
        let mut source = state.editor.document.model.source;
        *source.signal.carrier_mut().2 *= 0.5;
        state.editor.set_point_source(source).unwrap();
        assert_eq!(lit(&state), Some(pulse));
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
        // scene the step is about. The Edit step lights Edit alone first,
        // the wall only once Edit is open, ahead of a folded bar's Panels.
        let edit = &STEPS[step_index("Edit")];
        state
            .spotlights
            .record(Spotlight::Tab(InspectorPanel::Edit), tab);
        assert_eq!(state.guide_spotlight(edit, viewport), Some((tab, None)));
        state.spotlights.clear();
        state.spotlights.record(Spotlight::Panels, panels);
        assert_eq!(
            state.guide_spotlight(edit, viewport),
            Some((panels, PANELS_HINT)),
            "the wall waits for Edit"
        );
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
            .guide_spotlight(&STEPS[step_index("Select and transform")], viewport)
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
        // The region first, where the scene draws it, and its list only
        // once a click has selected it.
        let (region, hint) = state.guide_spotlight(materials, viewport).unwrap();
        assert_eq!(hint, Some("Click inside the round region to select it."));
        let center = state.screen(Point2::new(0.1, -0.45), viewport);
        assert!(region.contains(center), "{region:?}");
        state.select_region(GUIDE_REGION);
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
            Some((combo, Some("Pick the Poynting flow.")))
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
        settle(&mut state.editor);
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
        settle(&mut state.editor);
        state.guide_update(1.0);
        assert!(!state.guide_step_done(), "the early drag counted");
        source.position = source.position + Point2::new(0.0, 0.2);
        state.editor.set_point_source(source).unwrap();
        settle(&mut state.editor);
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
