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
use funfern_app::topology_examples::{GUIDE_OBSTACLE, GUIDE_REGION, guide_scene};

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
    /// A region's material choice in the Materials panel.
    RegionMaterial(RegionId),
    /// The source, where the viewport draws it.
    Source,
    /// The obstacle, where the viewport draws it.
    Obstacle,
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
    /// menu lights the menu and says so.
    targets: &'static [(Spotlight, Option<&'static str>)],
    done: fn(&Playground) -> bool,
}

/// A done step waits this long before moving on, so its tick is seen.
const GUIDE_ADVANCE_DELAY: f64 = 0.9;
const GUIDE_CARD_WIDTH: f32 = 380.0;

const PANELS_HINT: Option<&str> = Some("The panel is under Panels in the bar.");

pub(super) static STEPS: [GuideStep; 10] = [
    GuideStep {
        title: "Welcome to funfern",
        text: "The wave runs as soon as its mesh is ready. Run and Pause, Step and Reset \
               sit at the right end of the bar. Pause the wave, then run it again.",
        targets: &[(Spotlight::RunPause, None)],
        done: |state| state.guide.paused_seen && state.wave_running,
    },
    GuideStep {
        title: "The source",
        text: "The circled point is the source; the Simulation panel sets its signal. \
               Drag it somewhere else and watch the field follow.",
        targets: &[(Spotlight::Source, None)],
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
            (Spotlight::Tab(InspectorPanel::Edit), None),
            (Spotlight::Panels, PANELS_HINT),
        ],
        done: |state| {
            state.editor.document.model.draft.outer_boundaries.sides[OuterSide::Right.index()]
                != state.guide.baseline.right_wall
        },
    },
    GuideStep {
        title: "Select and move",
        text: "Drag a rectangle round the round obstacle to select the whole of it. A gizmo \
               appears on the selection, with handles to move, scale and turn it. Drag the \
               gizmo and put the obstacle somewhere else.",
        targets: &[(Spotlight::Obstacle, None)],
        done: |state| {
            state.guide_obstacle_controls() != state.guide.baseline.obstacle
                && !state.editor.editing()
        },
    },
    GuideStep {
        title: "Draw",
        text: "+ Draw opens the palette: circles, rectangles, polygons, splines and lines, \
               as regions, holes or walls. Draw a line across the scene; it becomes a wall.",
        targets: &[(Spotlight::Draw, None)],
        done: |state| {
            state.editor.document.model.draft.geometry.curves.len() > state.guide.baseline.curves
        },
    },
    GuideStep {
        title: "Materials",
        text: "Materials holds the library and says which region has which. Give the round \
               region at the bottom the glass.",
        targets: &[
            (Spotlight::RegionMaterial(GUIDE_REGION), None),
            (
                Spotlight::Tab(InspectorPanel::Materials),
                Some("Open the Materials panel first."),
            ),
            (Spotlight::Panels, PANELS_HINT),
        ],
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
            (Spotlight::Tab(InspectorPanel::View), None),
            (Spotlight::Panels, PANELS_HINT),
        ],
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

    /// The next step, or the end after the last.
    pub(super) fn guide_advance(&mut self) {
        self.guide.done_since = None;
        if self.guide.step + 1 >= STEPS.len() {
            self.finish_guide();
        } else {
            self.guide.step += 1;
            self.guide.paused_seen = false;
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

    /// The rect the step lights this frame and the hint for it, the first
    /// of its targets that was drawn.
    pub(super) fn guide_spotlight(
        &self,
        step: &GuideStep,
        viewport: Rect,
    ) -> Option<(Rect, Option<&'static str>)> {
        step.targets.iter().find_map(|(target, hint)| {
            let rect = match target {
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
                other => self.spotlights.rect(*other),
            };
            rect.map(|rect| (rect, *hint))
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
        let spotlight = self.guide_spotlight(step, viewport);
        // The dimming is paint on a foreground layer, no widget, so clicks
        // go through it to the controls underneath.
        if let Some((rect, _)) = spotlight {
            let screen = ctx.content_rect();
            let hole = rect.expand(6.0).intersect(screen);
            let painter = ctx.layer_painter(egui::LayerId::new(
                egui::Order::Foreground,
                egui::Id::new("guide-spotlight"),
            ));
            let dim = Color32::from_black_alpha(96);
            for shade in [
                Rect::from_min_max(screen.min, Pos2::new(screen.max.x, hole.min.y)),
                Rect::from_min_max(Pos2::new(screen.min.x, hole.max.y), screen.max),
                Rect::from_min_max(
                    Pos2::new(screen.min.x, hole.min.y),
                    Pos2::new(hole.min.x, hole.max.y),
                ),
                Rect::from_min_max(
                    Pos2::new(hole.max.x, hole.min.y),
                    Pos2::new(screen.max.x, hole.max.y),
                ),
            ] {
                if shade.is_positive() {
                    painter.rect_filled(shade, 0.0, dim);
                }
            }
            painter.rect_stroke(hole, 6.0, Stroke::new(2.0, TEAL), egui::StrokeKind::Outside);
        }
        let (index, total) = (self.guide.step + 1, STEPS.len());
        let last = index == total;
        let mut skip = false;
        let mut next = false;
        egui::Window::new("Guided tour")
            .id(egui::Id::new("guide-card"))
            .title_bar(false)
            .resizable(false)
            .collapsible(false)
            .pivot(egui::Align2::CENTER_TOP)
            .fixed_pos(viewport.center_top() + egui::vec2(0.0, 14.0))
            .default_width(GUIDE_CARD_WIDTH)
            .show(ctx, |ui| {
                ui.set_width(GUIDE_CARD_WIDTH.min(viewport.width() - 24.0));
                ui.horizontal(|ui| {
                    ui.strong(step.title);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.weak(format!("{index} / {total}"));
                    });
                });
                ui.label(step.text);
                if let Some((_, Some(hint))) = spotlight {
                    ui.small(hint);
                }
                ui.add_space(4.0);
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
        if skip {
            self.finish_guide();
        } else if next {
            self.guide_advance();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use funfern_app::topology_editor::{OpenCurvePurpose, TopologyProbeTarget};
    use funfern_app::topology_examples::GUIDE_GLASS;
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
