//! The chrome around the viewport: the top bar and the side panel that hosts
//! whichever inspector is open.

use crate::files::{self};
use bevy::prelude::*;
use bevy_egui::egui::{self};
use funfern_app::topology_viewport::{TopologySelection, TopologySpanTarget};
use funfern_core::*;

use super::*;

impl Playground {
    pub(super) fn top_bar(&mut self, root: &mut egui::Ui) {
        let fold_panels = root.available_width() < 1080.0;
        egui::Panel::top("top").exact_size(42.0).show(root, |ui| {
            ui.horizontal(|ui| {
                if ui.button("Undo").clicked() {
                    self.undo();
                }
                if ui.button("Redo").clicked() {
                    self.redo();
                }
                ui.menu_button("File", |ui| {
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
                });
                if ui.button("Fit view").clicked() {
                    self.fit = true;
                }
                let panels = [
                    (InspectorPanel::Edit, "Edit"),
                    (InspectorPanel::View, "View"),
                    (InspectorPanel::Simulation, "Simulation"),
                    (InspectorPanel::Materials, "Materials"),
                    (InspectorPanel::Probes, "Probes"),
                ];
                if fold_panels {
                    ui.menu_button("Panels", |ui| {
                        for (panel, label) in panels {
                            let selected = self.inspector == Some(panel);
                            if ui.selectable_label(selected, label).clicked() {
                                self.inspector = (!selected).then_some(panel);
                            }
                        }
                    });
                } else {
                    for (panel, label) in panels {
                        let selected = self.inspector == Some(panel);
                        if ui.selectable_label(selected, label).clicked() {
                            self.inspector = (!selected).then_some(panel);
                        }
                    }
                }
                if ui.button("+ Draw").clicked() {
                    self.draw_open = !self.draw_open;
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Reset").clicked() {
                        self.reset_requested = true;
                    }
                    if ui.button("Step").clicked() {
                        self.wave_step = true;
                    }
                    if ui
                        .button(if self.wave_running { "Pause" } else { "Run" })
                        .clicked()
                    {
                        self.wave_running = !self.wave_running;
                    }
                });
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
    }
    pub(super) fn side_panel(&mut self, root: &mut egui::Ui) {
        let Some(panel) = self.inspector else { return };
        let title = match panel {
            InspectorPanel::Edit => "Edit",
            InspectorPanel::View => "View",
            InspectorPanel::Simulation => "Simulation",
            InspectorPanel::Materials => "Materials",
            InspectorPanel::Probes => "Probes",
        };
        // On a narrow layout the inspector floats over the viewport instead of
        // docking beside it, which would put a panel inside the capture crop.
        if self.capturing() && root.available_width() < 700.0 {
            return;
        }
        if root.available_width() < 700.0 {
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
