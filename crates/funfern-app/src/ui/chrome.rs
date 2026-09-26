//! The chrome around the viewport: the top bar and the side panel that hosts
//! whichever inspector is open.

use crate::files::{self};
use bevy::prelude::*;
use bevy_egui::egui::{self};
use funfern_app::topology_viewport::{TopologySelection, TopologySpanTarget};
use funfern_core::*;

use super::*;

/// How far the top bar has folded to fit its width, least first. Each step
/// keeps what the ones before it did, and the bar takes the first that fits,
/// measured in its own fonts, so no width is hard-coded.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum ToolbarFold {
    /// Every label, and the inspector tabs in the bar.
    Full,
    /// The tabs in a Panels menu.
    Tabs,
    /// Undo, Redo, Fit view and the three run controls as icons.
    Icons,
    /// Redo, Fit view, Step and Reset in a … menu at the right end.
    Overflow,
    /// + Draw as +, and Panels as ☰.
    Compact,
    /// Examples as its picture alone.
    Minimal,
}

impl ToolbarFold {
    pub(super) const ALL: [Self; 6] = [
        Self::Full,
        Self::Tabs,
        Self::Icons,
        Self::Overflow,
        Self::Compact,
        Self::Minimal,
    ];

    /// The least folded bar that fits `width` in `ui`'s fonts, or the most
    /// folded when none does.
    pub(super) fn fitting(ui: &egui::Ui, width: f32) -> Self {
        Self::ALL
            .into_iter()
            .find(|fold| fold.width(ui) <= width)
            .unwrap_or(Self::Minimal)
    }

    /// The width the bar takes at this fold with its two ends pushed
    /// together. An item with two labels is counted at the wider, so Run
    /// turning into Pause never changes the fold.
    pub(super) fn width(self, ui: &egui::Ui) -> f32 {
        let (left, right) = self.items();
        let font = egui::TextStyle::Button.resolve(ui.style());
        let padding = 2.0 * ui.spacing().button_padding.x;
        let text = |label: &str| {
            ui.ctx().fonts_mut(|fonts| {
                fonts
                    .layout_no_wrap(label.to_owned(), font.clone(), Color32::WHITE)
                    .size()
                    .x
            })
        };
        let buttons = left
            .iter()
            .chain(&right)
            .map(|item| {
                item.labels(self)
                    .iter()
                    .map(|label| text(label))
                    .fold(0.0, f32::max)
                    + padding
            })
            .sum::<f32>();
        buttons + ui.spacing().item_spacing.x * (left.len() + right.len() - 1) as f32
    }

    /// What the bar holds at this fold: the items from its left end, and
    /// those at its right end, left to right.
    fn items(self) -> (Vec<ToolbarItem>, Vec<ToolbarItem>) {
        use ToolbarItem::*;
        let overflow = self >= Self::Overflow;
        let mut left = vec![Undo];
        if !overflow {
            left.push(Redo);
        }
        left.extend([File, Examples]);
        if !overflow {
            left.push(FitView);
        }
        if self == Self::Full {
            left.extend(InspectorPanel::ALL.map(Tab));
        } else {
            left.push(Panels);
        }
        left.push(Draw);
        let right = if overflow {
            vec![RunPause, More]
        } else {
            vec![RunPause, Step, Reset]
        };
        (left, right)
    }
}

/// One control in the top bar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ToolbarItem {
    Undo,
    Redo,
    File,
    Examples,
    FitView,
    Tab(InspectorPanel),
    Panels,
    Draw,
    RunPause,
    Step,
    Reset,
    More,
}

impl ToolbarItem {
    /// What it shows at `fold`. Run/Pause has both, Run first.
    fn labels(self, fold: ToolbarFold) -> &'static [&'static str] {
        let icons = fold >= ToolbarFold::Icons;
        let compact = fold >= ToolbarFold::Compact;
        match self {
            Self::Undo if icons => &["⟲"],
            Self::Undo => &["Undo"],
            Self::Redo if icons => &["⟳"],
            Self::Redo => &["Redo"],
            Self::File => &["File"],
            Self::Examples if fold >= ToolbarFold::Minimal => &["🖼"],
            Self::Examples => &["🖼 Examples"],
            Self::FitView if icons => &["⛶"],
            Self::FitView => &["Fit view"],
            Self::Tab(panel) => match panel {
                InspectorPanel::Edit => &["Edit"],
                InspectorPanel::View => &["View"],
                InspectorPanel::Simulation => &["Simulation"],
                InspectorPanel::Materials => &["Materials"],
                InspectorPanel::Probes => &["Probes"],
            },
            Self::Panels if compact => &["☰"],
            Self::Panels => &["Panels"],
            Self::Draw if compact => &["+"],
            Self::Draw => &["+ Draw"],
            Self::RunPause if icons => &["⏵", "⏸"],
            Self::RunPause => &["Run", "Pause"],
            Self::Step if icons => &["⏭"],
            Self::Step => &["Step"],
            Self::Reset if icons => &["⏮"],
            Self::Reset => &["Reset"],
            Self::More => &["…"],
        }
    }
}

/// Below this width the inspector floats over the viewport rather than
/// docking beside it.
pub(super) const FLOATING_INSPECTOR_BELOW: f32 = 700.0;

/// Where the top bar's two ends landed, for the tests that hold them apart.
#[derive(Clone, Copy, Debug)]
pub(super) struct ToolbarFit {
    pub(super) fold: ToolbarFold,
    pub(super) bar: egui::Rect,
    pub(super) left: egui::Rect,
    pub(super) right: egui::Rect,
}

impl Playground {
    pub(super) fn top_bar(&mut self, root: &mut egui::Ui) -> ToolbarFit {
        let mut fit = ToolbarFit {
            fold: ToolbarFold::Full,
            bar: egui::Rect::NOTHING,
            left: egui::Rect::NOTHING,
            right: egui::Rect::NOTHING,
        };
        egui::Panel::top("top").exact_size(42.0).show(root, |ui| {
            ui.horizontal(|ui| {
                let fold = ToolbarFold::fitting(ui, ui.available_width());
                let (left, right) = fold.items();
                fit.fold = fold;
                fit.bar = ui.max_rect();
                for item in left {
                    self.toolbar_item(ui, item, fold);
                }
                fit.left = ui.min_rect();
                fit.right = ui
                    .with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        for item in right.into_iter().rev() {
                            self.toolbar_item(ui, item, fold);
                        }
                    })
                    .response
                    .rect;
            });
        });
        // The palette stays up across draws - one primitive after another is the
        // usual way it is used - so it closes only from its own button or the
        // toolbar toggle, and it floats where it was last dragged.
        if self.draw_open && !self.capturing() {
            let ctx = root.ctx().clone();
            let mut open = true;
            egui::Window::new("Draw")
                .open(&mut open)
                .collapsible(false)
                .resizable(false)
                .default_pos([300.0, 42.0])
                .show(&ctx, |ui| {
                    ui.label("Closed curve");
                    ui.horizontal(|ui| {
                        ui.radio_value(
                            &mut self.closed_purpose,
                            ClosedPurpose::Subdomain,
                            "Subdomain",
                        );
                        ui.radio_value(&mut self.closed_purpose, ClosedPurpose::Hole, "Hole");
                    });
                    if self.closed_purpose == ClosedPurpose::Subdomain {
                        let selected = self.resolved_material_selection();
                        let materials = self
                            .editor
                            .document
                            .model
                            .draft
                            .materials
                            .iter()
                            .map(|material| (material.id, material.name.clone()))
                            .collect::<Vec<_>>();
                        let selected_name = materials
                            .iter()
                            .find(|(id, _)| *id == selected)
                            .map_or("Missing", |(_, name)| name.as_str());
                        egui::ComboBox::from_label("Material")
                            .selected_text(selected_name)
                            .show_ui(ui, |ui| {
                                for (id, name) in &materials {
                                    ui.selectable_value(&mut self.material_selection, *id, name);
                                }
                            });
                    }
                    ui.horizontal(|ui| {
                        for (tool, label) in [
                            (DrawTool::Circle, "Circle"),
                            (DrawTool::Rectangle, "Rectangle"),
                            (DrawTool::Polygon, "Polygon"),
                            (DrawTool::ClosedSpline, "Spline"),
                        ] {
                            if ui.button(label).clicked() {
                                self.begin_draw(tool);
                            }
                        }
                    });
                    ui.separator();
                    ui.label("Open curve");
                    ui.horizontal(|ui| {
                        ui.radio_value(
                            &mut self.open_purpose,
                            OpenPurpose::Separator,
                            "Subdomain separator",
                        );
                        ui.radio_value(&mut self.open_purpose, OpenPurpose::Baffle, "BC baffle");
                    });
                    ui.horizontal(|ui| {
                        if ui.button("Polyline").clicked() {
                            self.begin_draw(DrawTool::Polyline);
                        }
                        if ui.button("Spline").clicked() {
                            self.begin_draw(DrawTool::OpenSpline);
                        }
                    });
                });
            self.draw_open = open;
        }
        fit
    }

    /// One control of the top bar as `fold` shows it. An icon names what it
    /// stands for on hover.
    fn toolbar_item(&mut self, ui: &mut egui::Ui, item: ToolbarItem, fold: ToolbarFold) {
        let labels = item.labels(fold);
        let label = labels[usize::from(item == ToolbarItem::RunPause && self.wave_running)];
        let icon = fold >= ToolbarFold::Icons;
        let named = |response: egui::Response, name: &str| {
            if icon {
                response.on_hover_text(name)
            } else {
                response
            }
        };
        match item {
            ToolbarItem::Undo => {
                let shortcut = ui.ctx().format_shortcut(&session::UNDO_SHORTCUT);
                if ui
                    .button(label)
                    .on_hover_text(format!("Undo ({shortcut})"))
                    .clicked()
                {
                    self.undo();
                }
            }
            ToolbarItem::Redo => {
                let shortcut = ui.ctx().format_shortcut(&session::REDO_SHORTCUT);
                if ui
                    .button(label)
                    .on_hover_text(format!("Redo ({shortcut})"))
                    .clicked()
                {
                    self.redo();
                }
            }
            ToolbarItem::File => {
                ui.menu_button(label, |ui| self.file_menu(ui));
            }
            // A button like its neighbours, set a little apart by its
            // picture, rather than a panel tab: it opens a window, as
            // + Draw does.
            ToolbarItem::Examples => {
                if ui
                    .button(label)
                    .on_hover_text("Ready-to-run scenes")
                    .clicked()
                {
                    self.examples_open = !self.examples_open;
                }
            }
            ToolbarItem::FitView => {
                if named(ui.button(label), "Fit view").clicked() {
                    self.fit = true;
                }
            }
            ToolbarItem::Tab(panel) => {
                let selected = self.inspector == Some(panel);
                if ui.selectable_label(selected, label).clicked() {
                    self.inspector = (!selected).then_some(panel);
                }
            }
            ToolbarItem::Panels => {
                let menu = ui.menu_button(label, |ui| {
                    for panel in InspectorPanel::ALL {
                        let selected = self.inspector == Some(panel);
                        if ui.selectable_label(selected, panel.title()).clicked() {
                            self.inspector = (!selected).then_some(panel);
                        }
                    }
                });
                if fold >= ToolbarFold::Compact {
                    menu.response.on_hover_text("Panels");
                }
            }
            ToolbarItem::Draw => {
                let button = ui.button(label);
                let button = if fold >= ToolbarFold::Compact {
                    button.on_hover_text("Draw")
                } else {
                    button
                };
                if button.clicked() {
                    self.draw_open = !self.draw_open;
                }
            }
            ToolbarItem::RunPause => {
                let name = if self.wave_running { "Pause" } else { "Run" };
                if named(ui.button(label), name).clicked() {
                    self.wave_running = !self.wave_running;
                }
            }
            ToolbarItem::Step => {
                if named(ui.button(label), "Step").clicked() {
                    self.wave_step = true;
                }
            }
            ToolbarItem::Reset => {
                if named(ui.button(label), "Reset").clicked() {
                    self.reset_requested = true;
                }
            }
            // What the narrowest bars have no room for, by name. Step keeps
            // the menu open, since stepping is done a step at a time.
            ToolbarItem::More => {
                ui.menu_button(label, |ui| {
                    let redo = ui.ctx().format_shortcut(&session::REDO_SHORTCUT);
                    if ui
                        .add(egui::Button::new("Redo").shortcut_text(redo))
                        .clicked()
                    {
                        self.redo();
                        ui.close();
                    }
                    if ui.button("Fit view").clicked() {
                        self.fit = true;
                        ui.close();
                    }
                    ui.separator();
                    if ui.button("Step").clicked() {
                        self.wave_step = true;
                    }
                    if ui.button("Reset").clicked() {
                        self.reset_requested = true;
                        ui.close();
                    }
                });
            }
        }
    }

    fn file_menu(&mut self, ui: &mut egui::Ui) {
        if ui.button("New").clicked() {
            self.new_scene();
            ui.close();
        }
        if ui.button("Examples…").clicked() {
            self.examples_open = true;
            ui.close();
        }
        ui.separator();
        if ui.button("Open…").clicked() {
            self.file_busy = true;
            files::load(self.sender.clone());
            ui.close();
        }
        if ui.button("Save…").clicked() {
            self.save_scene();
            ui.close();
        }
        if ui.button("Copy scene link").clicked() {
            self.copy_link(ui.ctx());
            ui.close();
        }
        let capture_ready = self.snapshot_state == SnapshotState::Idle
            && self.recording_state == RecordingState::Idle;
        if ui
            .add_enabled(capture_ready, egui::Button::new("Export viewport PNG"))
            .clicked()
        {
            self.export_viewport_png();
            ui.close();
        }
        let recording_label = if matches!(
            self.recording_state,
            RecordingState::Starting | RecordingState::Recording
        ) {
            "Stop recording"
        } else if self.recording_state != RecordingState::Idle {
            "Preparing recording…"
        } else {
            "Record viewport"
        };
        if ui
            .add_enabled(
                capture_ready
                    || matches!(
                        self.recording_state,
                        RecordingState::Starting | RecordingState::Recording
                    ),
                egui::Button::new(recording_label),
            )
            .clicked()
        {
            if self.recording_state == RecordingState::Idle {
                self.request_video_recording();
            } else {
                self.stop_video_recording();
            }
            ui.close();
        }
    }

    /// A launch on a narrow screen opens no inspector: there it would float
    /// over the scene, and over its card, and the scene is what a launch is
    /// for. A panel is a tap away in the top bar.
    pub(super) fn fit_inspector_to_screen(&mut self, width: f32) {
        if width < FLOATING_INSPECTOR_BELOW {
            self.inspector = None;
        }
    }

    pub(super) fn side_panel(&mut self, root: &mut egui::Ui) {
        let Some(panel) = self.inspector else { return };
        let title = panel.title();
        // On a narrow layout the inspector floats over the viewport instead of
        // docking beside it, which would put a panel inside the capture crop.
        if self.capturing() && root.available_width() < FLOATING_INSPECTOR_BELOW {
            return;
        }
        if root.available_width() < FLOATING_INSPECTOR_BELOW {
            let mut open = true;
            let maximum_height = (root.ctx().viewport_rect().height() - 54.0).max(96.0);
            egui::Window::new(title)
                .id(egui::Id::new("mobile-inspector"))
                .open(&mut open)
                .default_width(280.0)
                .max_height(maximum_height)
                .anchor(egui::Align2::RIGHT_TOP, [-6.0, 48.0])
                .show(root.ctx(), |ui| {
                    egui::ScrollArea::vertical()
                        .id_salt(("mobile-inspector-scroll", title))
                        .show(ui, |ui| self.inspector_contents(ui, panel));
                });
            if !open {
                self.inspector = None;
            }
            return;
        }
        egui::Panel::right("inspector")
            .default_size(292.0)
            .show(root, |ui| {
                egui::ScrollArea::vertical()
                    .id_salt(("inspector-scroll", title))
                    .auto_shrink([false, false])
                    .show(ui, |ui| self.inspector_contents(ui, panel));
            });
    }
    pub(super) fn inspector_contents(&mut self, ui: &mut egui::Ui, panel: InspectorPanel) {
        match panel {
            InspectorPanel::Edit => self.edit_panel(ui),
            InspectorPanel::View => self.view_panel(ui),
            InspectorPanel::Simulation => self.simulation_panel(ui),
            InspectorPanel::Materials => self.materials_panel(ui),
            InspectorPanel::Probes => self.probes_panel(ui),
        }
    }
    pub(super) fn edit_panel(&mut self, ui: &mut egui::Ui) {
        ui.heading("Edit");
        ui.collapsing("Features", |ui| {
            if ui.selectable_label(matches!(self.selection, TopologySelection::Spans(ref s) if s.iter().all(|v| matches!(v, TopologySpanTarget::Outer(_)))), "Outer boundary").clicked() {
                self.selection = TopologySelection::Spans(OuterSide::ALL.into_iter().map(TopologySpanTarget::Outer).collect());
            }
            let curves = self.editor.document.model.draft.geometry.curves.iter().map(|curve| (curve.id, curve.spline.is_open(), curve.spans.iter().map(|span| span.id).collect::<Vec<_>>())).collect::<Vec<_>>();
            for (curve, open, spans) in curves {
                let selected = matches!(&self.selection, TopologySelection::Spans(selection) if spans.iter().all(|span| selection.contains(&TopologySpanTarget::Curve(*span))));
                if ui.selectable_label(selected, format!("{} {}", if open { "Open curve" } else { "Closed curve" }, curve.0)).clicked() {
                    self.selection = TopologySelection::Spans(spans.into_iter().map(TopologySpanTarget::Curve).collect());
                }
            }
        });
        ui.separator();
        match self.selection.clone() {
            TopologySelection::None => {
                ui.weak("Select a control, junction, span, or face");
            }
            TopologySelection::Handle(handle) => self.handle_inspector(ui, handle),
            TopologySelection::Spans(spans) => self.span_inspector(ui, spans),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The top bar laid out alone on a screen `width` wide, in the app's look.
    fn bar_at(state: &mut Playground, context: &egui::Context, width: f32) -> ToolbarFit {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(width, 800.0),
            )),
            ..egui::RawInput::default()
        };
        let mut fit = None;
        let _ = context.run_ui(input, |ui| fit = Some(state.top_bar(ui)));
        fit.unwrap()
    }

    /// From a phone held upright to a wide desktop, the bar's two ends never
    /// meet and nothing leaves it, and it only ever folds further as the
    /// screen narrows.
    #[test]
    fn the_top_bar_fits_from_a_phone_to_a_desktop() {
        let mut state = Playground::default();
        let context = egui::Context::default();
        theme::apply(&context);
        let spacing = 8.0;
        let mut wider_fold = ToolbarFold::Full;
        for width in (320..=1600).rev().step_by(4) {
            let fit = bar_at(&mut state, &context, width as f32);
            assert!(
                fit.fold >= wider_fold,
                "unfolded to {:?} at {width}",
                fit.fold
            );
            wider_fold = fit.fold;
            assert!(
                fit.left.right() + spacing <= fit.right.left() + 0.5,
                "the ends meet at {width} ({:?}): {:?} against {:?}",
                fit.fold,
                fit.left,
                fit.right
            );
            assert!(fit.left.left() >= fit.bar.left() - 0.5, "{width}");
            assert!(fit.right.right() <= fit.bar.right() + 0.5, "{width}");
        }
        assert_eq!(bar_at(&mut state, &context, 1280.0).fold, ToolbarFold::Full);
        assert_eq!(bar_at(&mut state, &context, 700.0).fold, ToolbarFold::Tabs);
        assert!(bar_at(&mut state, &context, 360.0).fold <= ToolbarFold::Compact);
    }

    /// Pressing Run turns it into Pause, which is wider; the fold was chosen
    /// for the wider already, so nothing else moves.
    #[test]
    fn running_or_pausing_keeps_the_fold() {
        let mut state = Playground::default();
        let context = egui::Context::default();
        theme::apply(&context);
        for width in (320..=1600).step_by(4) {
            state.wave_running = false;
            let paused = bar_at(&mut state, &context, width as f32).fold;
            state.wave_running = true;
            assert_eq!(bar_at(&mut state, &context, width as f32).fold, paused);
        }
    }

    #[test]
    fn a_narrow_screen_launches_without_an_inspector() {
        let mut phone = Playground::default();
        phone.fit_inspector_to_screen(390.0);
        assert_eq!(phone.inspector, None);
        let mut desktop = Playground::default();
        desktop.fit_inspector_to_screen(1280.0);
        assert_eq!(desktop.inspector, Some(InspectorPanel::Edit));
    }

    #[test]
    fn every_toolbar_label_is_in_the_fonts() {
        let context = egui::Context::default();
        let _ = context.run_ui(egui::RawInput::default(), |_| {});
        let font = egui::TextStyle::Button.resolve(&context.global_style());
        for fold in ToolbarFold::ALL {
            let (left, right) = fold.items();
            for item in left.into_iter().chain(right) {
                for label in item.labels(fold) {
                    assert!(
                        context.fonts_mut(|fonts| fonts.has_glyphs(&font, label)),
                        "{label} ({item:?} at {fold:?})"
                    );
                }
            }
        }
    }
}
