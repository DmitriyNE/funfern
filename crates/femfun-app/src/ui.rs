use crate::files::{self, FileEvent};
use bevy::platform::time::Instant;
use bevy::prelude::*;
use bevy_egui::{
    EguiContexts,
    egui::{self, Color32, Pos2, Rect, Stroke},
};
use femfun_app::{
    editor::{Acceptance, Editor},
    persistence::{self, LoadCandidate},
};
use femfun_core::*;
use std::sync::{
    Mutex,
    mpsc::{self, Receiver, Sender},
};
const TEAL: Color32 = Color32::from_rgb(91, 220, 194);
const RED: Color32 = Color32::from_rgb(255, 106, 123);
const GOLD: Color32 = Color32::from_rgb(248, 196, 112);
#[derive(Default, PartialEq)]
enum Mode {
    #[default]
    Select,
    Preset,
    Custom,
}
struct Drag {
    id: ObstacleId,
    index: usize,
    offset: Point2,
}
struct Curve {
    id: ObstacleId,
    samples: Vec<Sample>,
}
#[derive(Resource)]
pub struct Playground {
    editor: Editor,
    mode: Mode,
    selection: Option<(ObstacleId, Option<usize>)>,
    custom: Vec<Point2>,
    drag: Option<Drag>,
    panning: bool,
    center: Point2,
    scale: f64,
    fit: bool,
    grid: bool,
    polygon: bool,
    handles: bool,
    reference: bool,
    cache_revision: u64,
    cache_scale: f64,
    cache_accepted: Scene,
    draft_curves: Vec<Curve>,
    accepted_curves: Vec<Curve>,
    sampling_warning: bool,
    message: String,
    sender: Sender<FileEvent>,
    receiver: Mutex<Receiver<FileEvent>>,
    file_busy: bool,
    load: Option<LoadCandidate>,
    frame_ms: f32,
    ready: bool,
    keyboard_captured: bool,
    mesh: Option<TriMesh>,
    mesh_job: Option<MeshingJob>,
    mesh_source: Scene,
    mesh_max_edge: f64,
    mesh_source_max_edge: f64,
    mesh_low_quality: Vec<bool>,
    mesh_error: Option<String>,
    mesh_started: Option<Instant>,
    mesh_build_ms: f64,
    mesh_work_ms: f64,
    mesh_max_slice_ms: f64,
    show_mesh: bool,
    show_mesh_boundary: bool,
}
impl Default for Playground {
    fn default() -> Self {
        let (sender, receiver) = mpsc::channel();
        Self {
            editor: Editor::default(),
            mode: Mode::Select,
            selection: Some((ObstacleId(1), None)),
            custom: vec![],
            drag: None,
            panning: false,
            center: Point2::default(),
            scale: 300.0,
            fit: true,
            grid: true,
            polygon: true,
            handles: true,
            reference: true,
            cache_revision: u64::MAX,
            cache_scale: 0.0,
            cache_accepted: Scene::default(),
            draft_curves: vec![],
            accepted_curves: vec![],
            sampling_warning: false,
            message: String::new(),
            sender,
            receiver: Mutex::new(receiver),
            file_busy: false,
            load: None,
            frame_ms: 0.0,
            ready: false,
            keyboard_captured: false,
            mesh: None,
            mesh_job: None,
            mesh_source: Scene::default(),
            mesh_max_edge: 0.04,
            mesh_source_max_edge: 0.0,
            mesh_low_quality: vec![],
            mesh_error: None,
            mesh_started: None,
            mesh_build_ms: 0.0,
            mesh_work_ms: 0.0,
            mesh_max_slice_ms: 0.0,
            show_mesh: false,
            show_mesh_boundary: true,
        }
    }
}
impl Playground {
    fn screen(&self, p: Point2, r: Rect) -> Pos2 {
        Pos2::new(
            r.center().x + ((p.x - self.center.x) * self.scale).clamp(-1e7, 1e7) as f32,
            r.center().y - ((p.y - self.center.y) * self.scale).clamp(-1e7, 1e7) as f32,
        )
    }
    fn world(&self, p: Pos2, r: Rect) -> Point2 {
        Point2::new(
            self.center.x + (p.x - r.center().x) as f64 / self.scale,
            self.center.y - (p.y - r.center().y) as f64 / self.scale,
        )
    }
    fn clear_transient(&mut self) {
        self.selection = None;
        self.drag = None;
        self.panning = false;
        self.custom.clear();
        self.mode = Mode::Select;
    }
    fn error<T>(&mut self, result: Result<T, String>) -> Option<T> {
        match result {
            Ok(v) => {
                self.message.clear();
                Some(v)
            }
            Err(e) => {
                self.message = e;
                None
            }
        }
    }
    fn finish_custom(&mut self) {
        if self.custom.len() < 4 {
            self.message = "A loop needs at least four control points".into();
            return;
        }
        let spline = PeriodicCubicSpline::uniform(self.custom.clone()).unwrap();
        let result = self.editor.create(spline);
        if let Some(id) = self.error(result) {
            self.selection = Some((id, None));
            self.custom.clear();
            self.mode = Mode::Select;
        }
    }
    fn update_files(&mut self) {
        let events: Vec<_> = self.receiver.lock().unwrap().try_iter().collect();
        for event in events {
            self.file_busy = false;
            match event {
                FileEvent::Loaded(bytes) => match persistence::parse(&bytes) {
                    Ok(load) => {
                        self.load = Some(load);
                        self.message = "Validating scene file…".into()
                    }
                    Err(e) => self.message = e,
                },
                FileEvent::Saved => self.message = "Scene saved".into(),
                FileEvent::Cancelled => {}
                FileEvent::Error(e) => self.message = e,
            }
        }
        if let Some(load) = &mut self.load
            && let Some(result) = load.advance(12_000)
        {
            self.load = None;
            match result {
                Ok(document) => {
                    self.editor.replace_validated(document);
                    self.clear_transient();
                    self.message = "Scene loaded; history cleared".into()
                }
                Err(e) => self.message = e,
            }
        }
    }
    fn refresh_curves(&mut self) {
        if self.cache_revision == self.editor.revision
            && self.cache_scale == self.scale
            && self.cache_accepted == self.editor.document.accepted
        {
            return;
        }
        self.cache_revision = self.editor.revision;
        self.cache_scale = self.scale;
        self.cache_accepted = self.editor.document.accepted.clone();
        self.sampling_warning = false;
        let options = SamplingOptions {
            tolerance: 0.6 / self.scale,
            max_depth: 14,
            max_points: 2048,
        };
        let mut curves = |scene: &Scene| {
            scene
                .obstacles
                .iter()
                .map(|o| {
                    let samples = match sample(&o.spline, options) {
                        Ok(s) => s,
                        Err(_) => {
                            self.sampling_warning = true;
                            vec![]
                        }
                    };
                    Curve { id: o.id, samples }
                })
                .collect()
        };
        self.draft_curves = curves(&self.editor.document.draft);
        self.accepted_curves = curves(&self.editor.document.accepted);
    }

    fn refresh_mesh(&mut self) {
        // A drag can promote many valid draft revisions. Mesh the complete
        // accepted document once the edit transaction ends.
        if self.editor.editing() {
            return;
        }
        if self.mesh_source != self.editor.document.accepted
            || self.mesh_source_max_edge != self.mesh_max_edge
        {
            self.mesh_source = self.editor.document.accepted.clone();
            self.mesh_source_max_edge = self.mesh_max_edge;
            self.mesh_job = Some(MeshingJob::new(
                self.mesh_source.clone(),
                self.editor.revision,
                MeshingOptions {
                    curve_tolerance: (self.mesh_max_edge * 0.02).min(1.5e-3),
                    target_edge_length: self.mesh_max_edge / 1.05,
                    minimum_angle_degrees: 12.0,
                    max_vertices: 50_000,
                    max_triangles: 100_000,
                    max_refinement_steps: 50_000,
                },
            ));
            self.mesh_error = None;
            self.mesh_started = Some(Instant::now());
            self.mesh_build_ms = 0.0;
            self.mesh_work_ms = 0.0;
            self.mesh_max_slice_ms = 0.0;
        }
        // A soft 2 ms deadline plus an operation ceiling. Check between units,
        // including topology preparation and individual edge flips.
        let start = Instant::now();
        let mut result = None;
        let active = self.mesh_job.is_some();
        if let Some(job) = &mut self.mesh_job {
            for _ in 0..100_000 {
                result = job.advance(1);
                if result.is_some() || start.elapsed().as_secs_f64() >= 0.002 {
                    break;
                }
            }
        }
        if let Some(result) = result {
            self.mesh_job = None;
            match result {
                Ok(mesh) => {
                    self.mesh_low_quality = (0..mesh.triangles.len())
                        .map(|i| mesh.triangle_quality(i).unwrap().minimum_angle_degrees < 15.0)
                        .collect();
                    self.mesh = Some(mesh);
                    self.mesh_error = None;
                }
                Err(error) => {
                    self.mesh = None;
                    self.mesh_low_quality.clear();
                    self.mesh_error = Some(error.to_string());
                }
            }
        }
        if active {
            // Include completed-job cleanup and mesh replacement in the slice.
            let elapsed = start.elapsed().as_secs_f64() * 1000.0;
            self.mesh_work_ms += elapsed;
            self.mesh_max_slice_ms = self.mesh_max_slice_ms.max(elapsed);
            self.mesh_build_ms = self
                .mesh_started
                .map_or(0.0, |t| t.elapsed().as_secs_f64() * 1000.0);
        }
    }
    fn panel(&mut self, ui: &mut egui::Ui) {
        ui.add_space(10.0);
        ui.heading(egui::RichText::new("femfun").size(29.0).color(TEAL));
        ui.label(
            egui::RichText::new("GEOMETRY PLAYGROUND")
                .small()
                .color(Color32::GRAY),
        );
        ui.add_space(14.0);
        ui.label("Create an obstacle");
        ui.horizontal(|ui| {
            if ui
                .selectable_label(self.mode == Mode::Select, "Select")
                .clicked()
            {
                self.mode = Mode::Select;
                self.custom.clear();
            }
            if ui
                .selectable_label(self.mode == Mode::Preset, "Rounded")
                .clicked()
            {
                self.mode = Mode::Preset;
                self.custom.clear();
            }
            if ui
                .selectable_label(self.mode == Mode::Custom, "Custom")
                .clicked()
            {
                self.mode = Mode::Custom;
                self.custom.clear();
            }
        });
        ui.small(match self.mode {
            Mode::Select => "Drag handles · double-click a curve to insert",
            Mode::Preset => "Click the viewport to place a radius 0.15 loop",
            Mode::Custom => "Click control points; Enter or first point closes",
        });
        if self.mode == Mode::Custom {
            ui.small(format!(
                "{} / 128 points · Backspace removes · Esc cancels",
                self.custom.len()
            ));
        }
        ui.add_space(10.0);
        ui.separator();
        ui.label(format!(
            "Obstacles  {} / 32",
            self.editor.document.draft.obstacles.len()
        ));
        egui::ScrollArea::vertical()
            .id_salt("obstacles")
            .max_height(135.0)
            .show(ui, |ui| {
                for o in &self.editor.document.draft.obstacles {
                    if ui
                        .selectable_label(
                            self.selection.is_some_and(|s| s.0 == o.id),
                            format!(
                                "Loop {:02}     {} controls",
                                o.id.0,
                                o.spline.controls().len()
                            ),
                        )
                        .clicked()
                    {
                        self.selection = Some((o.id, None));
                    }
                }
            });
        if let Some((id, index)) = self.selection {
            if ui.button("Delete obstacle").clicked() {
                self.editor.delete_obstacle(id);
                self.selection = None;
            }
            if let Some(index) = index
                && let Some(o) = self.editor.obstacle(id)
                && let Some(p) = o.spline.controls().get(index).copied()
            {
                ui.add_space(8.0);
                ui.label(format!("Control {}", index + 1));
                let mut p = p;
                let responses = ui
                    .push_id((id.0, index, "coordinates"), |ui| {
                        ui.horizontal(|ui| {
                            [
                                ui.add(
                                    egui::DragValue::new(&mut p.x)
                                        .speed(0.002)
                                        .range(-1e6..=1e6)
                                        .prefix("x ")
                                        .update_while_editing(false),
                                ),
                                ui.add(
                                    egui::DragValue::new(&mut p.y)
                                        .speed(0.002)
                                        .range(-1e6..=1e6)
                                        .prefix("y ")
                                        .update_while_editing(false),
                                ),
                            ]
                        })
                        .inner
                    })
                    .inner;
                if responses
                    .iter()
                    .any(|r| r.gained_focus() || r.drag_started() || r.changed())
                {
                    self.editor.begin();
                }
                if responses.iter().any(|r| r.changed()) {
                    let result = self.editor.set_point(id, index, p);
                    self.error(result);
                }
                if responses.iter().any(|r| r.lost_focus() || r.drag_stopped()) {
                    self.editor.commit();
                }
                let can_remove = self
                    .editor
                    .obstacle(id)
                    .is_some_and(|o| o.spline.controls().len() > 4);
                if ui
                    .add_enabled(
                        can_remove,
                        egui::Button::new("Remove control · reshapes curve"),
                    )
                    .clicked()
                {
                    let result = self.editor.remove_point(id, index);
                    if self.error(result).is_some() {
                        self.selection = Some((id, None));
                    }
                }
            }
        }
        ui.add_space(8.0);
        ui.separator();
        ui.horizontal(|ui| {
            let (undo, redo) = self.editor.history_len();
            if ui
                .add_enabled(undo > 0, egui::Button::new("Undo"))
                .clicked()
            {
                self.editor.undo();
                self.clear_transient();
            }
            if ui
                .add_enabled(redo > 0, egui::Button::new("Redo"))
                .clicked()
            {
                self.editor.redo();
                self.clear_transient();
            }
            if ui.button("Fit View").clicked() {
                self.fit = true;
            }
        });
        if ui
            .add_enabled(
                self.editor.document.draft != self.editor.document.accepted,
                egui::Button::new("Revert Draft"),
            )
            .clicked()
        {
            self.editor.revert();
            self.clear_transient();
        }
        ui.horizontal(|ui| {
            if ui.button("Save scene").clicked() {
                self.editor.commit();
                match persistence::save(&self.editor.document) {
                    Ok(json) => {
                        files::save(self.sender.clone(), json.into_bytes());
                        self.file_busy = true;
                    }
                    Err(e) => self.message = e,
                }
            }
            if ui.button("Load scene").clicked() {
                self.editor.commit();
                files::load(self.sender.clone());
                self.file_busy = true;
            }
        });
        ui.add_space(8.0);
        ui.separator();
        ui.collapsing("Display", |ui| {
            ui.checkbox(&mut self.grid, "Grid");
            ui.checkbox(&mut self.polygon, "Control polygons");
            ui.checkbox(&mut self.handles, "Handles");
            ui.checkbox(&mut self.reference, "Accepted reference");
            ui.checkbox(&mut self.show_mesh, "Accepted triangle mesh");
            ui.add_enabled_ui(self.show_mesh, |ui| {
                ui.checkbox(&mut self.show_mesh_boundary, "Mesh boundary labels");
            });
        });
        ui.add_space(8.0);
        ui.separator();
        ui.label("Accepted mesh");
        egui::ComboBox::from_id_salt("mesh_resolution")
            .selected_text(format!("Max edge {:.2}", self.mesh_max_edge))
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut self.mesh_max_edge, 0.16, "Preview · h ≤ 0.16");
                ui.selectable_value(&mut self.mesh_max_edge, 0.04, "Finer · h ≤ 0.04");
                ui.selectable_value(&mut self.mesh_max_edge, 0.02, "Fine · h ≤ 0.02");
            });
        ui.small("Wave accuracy still requires a convergence check.");
        if self.editor.editing() && self.mesh_source != self.editor.document.accepted {
            ui.small("Waiting for edit to finish…");
        } else if let Some(job) = &self.mesh_job {
            ui.small(format!("{}…", job.phase()));
        } else if let Some(mesh) = &self.mesh {
            let low_quality = self.mesh_low_quality.iter().filter(|poor| **poor).count();
            ui.small(format!(
                "{} vertices · {} triangles\nmin angle {:.1}° · max edge {:.3}\n{} elements below 15°",
                mesh.vertices.len(),
                mesh.triangles.len(),
                mesh.quality.minimum_angle_degrees,
                mesh.quality.maximum_edge_length,
                low_quality
            ));
        } else if let Some(error) = &self.mesh_error {
            ui.colored_label(RED, error);
        } else {
            ui.small("Preparing…");
        }
        if self.mesh_started.is_some() {
            ui.small(format!(
                "Build {:.0} ms · work {:.1} ms\nlongest mesh slice {:.2} ms (2 ms target)",
                self.mesh_build_ms, self.mesh_work_ms, self.mesh_max_slice_ms
            ));
        }
        ui.add_space(8.0);
        ui.label("Simulation · later milestone");
        ui.add_enabled_ui(false, |ui| {
            ui.horizontal(|ui| {
                let _ = ui.button("Run");
                let _ = ui.button("Step");
                let _ = ui.button("Reset");
            });
        });
        ui.add_space(12.0);
        ui.small("Control points guide the spline; the curve does not pass through them.");
        ui.add_space(6.0);
        ui.small("Pan: middle drag / Space + drag\nZoom: wheel over viewport\nUndo: Ctrl/Cmd + Z · Shift for redo");
        if !self.message.is_empty() {
            ui.add_space(8.0);
            ui.colored_label(GOLD, &self.message);
        }
    }
    fn viewport(&mut self, ui: &mut egui::Ui) -> Rect {
        let (response, painter) =
            ui.allocate_painter(ui.available_size(), egui::Sense::click_and_drag());
        let r = response.rect;
        if self.fit {
            self.center = Point2::default();
            self.scale = (r.width().min(r.height()) as f64 / 2.5).max(20.0);
            self.fit = false;
        }
        let ctx = ui.ctx();
        let pointer = ctx.input(|i| i.pointer.hover_pos());
        let over = response.contains_pointer() && pointer.is_some_and(|p| r.contains(p));
        let typing = self.keyboard_captured || ctx.text_edit_focused();
        let enabled = !self.file_busy && self.load.is_none();
        if enabled {
            if !typing && ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
                if self.drag.take().is_some() || self.editor.editing() {
                    self.editor.cancel();
                } else {
                    self.custom.clear();
                    self.mode = Mode::Select;
                }
                self.panning = false;
            }
            if !typing && ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::Z)) {
                if ctx.input(|i| i.modifiers.shift) {
                    self.editor.redo()
                } else {
                    self.editor.undo()
                }
                self.clear_transient();
            }
            if over && !typing {
                if self.mode == Mode::Custom {
                    if ctx.input(|i| i.key_pressed(egui::Key::Enter)) {
                        self.finish_custom();
                    }
                    if ctx.input(|i| i.key_pressed(egui::Key::Backspace)) {
                        self.custom.pop();
                    }
                } else if ctx.input(|i| i.key_pressed(egui::Key::Delete))
                    && let Some((id, Some(index))) = self.selection
                {
                    let result = self.editor.remove_point(id, index);
                    if self.error(result).is_some() {
                        self.selection = Some((id, None));
                    }
                }
                let p = pointer.unwrap();
                // Use only this frame's wheel events, so a panel's smoothed
                // scroll tail cannot zoom when the cursor enters the viewport.
                let wheel = ctx.input(|i| {
                    i.raw
                        .events
                        .iter()
                        .filter_map(|event| {
                            if let egui::Event::MouseWheel {
                                unit,
                                delta,
                                modifiers,
                                ..
                            } = event
                                && !modifiers.command
                                && !modifiers.ctrl
                            {
                                let multiplier = match unit {
                                    egui::MouseWheelUnit::Point => 1.0,
                                    egui::MouseWheelUnit::Line => 40.0,
                                    egui::MouseWheelUnit::Page => r.height(),
                                };
                                Some(delta.y * multiplier)
                            } else {
                                None
                            }
                        })
                        .sum::<f32>()
                });
                if wheel != 0.0 && self.drag.is_none() && !self.panning {
                    let before = self.world(p, r);
                    self.scale = (self.scale * (wheel as f64 * 0.002).exp()).clamp(20.0, 20_000.0);
                    let after = self.world(p, r);
                    self.center = self.center + before - after;
                }
                let primary = ctx.input(|i| i.pointer.button_pressed(egui::PointerButton::Primary));
                let middle = ctx.input(|i| i.pointer.button_pressed(egui::PointerButton::Middle));
                let space = ctx.input(|i| i.key_down(egui::Key::Space));
                if middle || primary && space {
                    self.panning = true;
                } else if primary && self.mode == Mode::Select {
                    self.refresh_curves();
                    if let Some((id, index)) = self.hit_handle(p, r) {
                        self.selection = Some((id, Some(index)));
                        self.editor.begin();
                        let point = self.editor.obstacle(id).unwrap().spline.controls()[index];
                        self.drag = Some(Drag {
                            id,
                            index,
                            offset: point - self.world(p, r),
                        });
                    } else {
                        self.selection = self.hit_curve(p, r).map(|(id, _)| (id, None));
                    }
                }
                if self.panning {
                    let delta = ctx.input(|i| i.pointer.delta());
                    self.center = self.center
                        + Point2::new(-delta.x as f64 / self.scale, delta.y as f64 / self.scale);
                } else if let Some(drag) = &self.drag
                    && response.dragged_by(egui::PointerButton::Primary)
                {
                    let result =
                        self.editor
                            .set_point(drag.id, drag.index, self.world(p, r) + drag.offset);
                    self.error(result);
                }
                if response.double_clicked()
                    && self.mode == Mode::Select
                    && !space
                    && self.hit_handle(p, r).is_none()
                {
                    self.editor.commit();
                    self.drag = None;
                    if let Some((id, t)) = self.hit_curve(p, r) {
                        let result = self.editor.insert(id, t);
                        if let Some(index) = self.error(result) {
                            self.selection = Some((id, Some(index)));
                        }
                    }
                } else if response.clicked() && !space && !self.panning {
                    match self.mode {
                        Mode::Preset => {
                            let result = self
                                .editor
                                .create(PeriodicCubicSpline::rounded(self.world(p, r), 0.15));
                            if let Some(id) = self.error(result) {
                                self.selection = Some((id, None));
                                self.mode = Mode::Select;
                            }
                        }
                        Mode::Custom => {
                            if self.custom.len() >= 4
                                && self.screen(self.custom[0], r).distance(p) < 10.0
                            {
                                self.finish_custom();
                            } else if self.custom.len() < 128 {
                                self.custom.push(self.world(p, r));
                            } else {
                                self.message = "Maximum 128 control points".into();
                            }
                        }
                        Mode::Select => {}
                    }
                }
            }
        }
        if !ctx.input(|i| i.pointer.primary_down()) {
            if self.drag.take().is_some() {
                self.editor.commit();
            }
            if !ctx.input(|i| i.pointer.button_down(egui::PointerButton::Middle)) {
                self.panning = false;
            }
        }
        self.refresh_curves();
        painter.rect_filled(r, 0.0, Color32::from_rgb(16, 23, 31));
        if self.grid {
            for i in -10..=10 {
                let t = i as f64 / 10.0;
                let color = if i == 0 {
                    Color32::from_rgb(40, 57, 70)
                } else {
                    Color32::from_rgb(27, 39, 49)
                };
                for (a, b) in [
                    (Point2::new(t, -1.0), Point2::new(t, 1.0)),
                    (Point2::new(-1.0, t), Point2::new(1.0, t)),
                ] {
                    painter.line_segment(
                        [self.screen(a, r), self.screen(b, r)],
                        Stroke::new(1.0, color),
                    );
                }
            }
        }
        let domain = [
            Point2::new(-1.0, -1.0),
            Point2::new(1.0, -1.0),
            Point2::new(1.0, 1.0),
            Point2::new(-1.0, 1.0),
            Point2::new(-1.0, -1.0),
        ];
        painter.add(egui::Shape::line(
            domain.map(|p| self.screen(p, r)).to_vec(),
            Stroke::new(1.5, Color32::from_rgb(100, 123, 140)),
        ));
        if self.show_mesh
            && let Some(mesh) = &self.mesh
        {
            for (triangle_index, triangle) in mesh.triangles.iter().enumerate() {
                let low_quality = self.mesh_low_quality[triangle_index];
                let mesh_stroke = Stroke::new(
                    if low_quality { 1.1 } else { 0.65 },
                    if low_quality {
                        Color32::from_rgba_unmultiplied(248, 196, 112, 165)
                    } else {
                        Color32::from_rgba_unmultiplied(78, 123, 151, 105)
                    },
                );
                let positions = triangle
                    .vertices
                    .map(|index| self.screen(mesh.vertices[index].point, r));
                for edge in [[0, 1], [1, 2], [2, 0]] {
                    painter.line_segment([positions[edge[0]], positions[edge[1]]], mesh_stroke);
                }
            }
            if self.show_mesh_boundary {
                for edge in &mesh.boundary_edges {
                    let color = match edge.label {
                        BoundaryLabel::Outer(_) => Color32::from_rgb(142, 161, 175),
                        BoundaryLabel::Obstacle(_) => Color32::from_rgb(119, 155, 255),
                    };
                    painter.line_segment(
                        edge.vertices
                            .map(|index| self.screen(mesh.vertices[index].point, r)),
                        Stroke::new(2.0, color),
                    );
                }
            }
        }
        if self.reference && self.editor.document.draft != self.editor.document.accepted {
            for curve in &self.accepted_curves {
                self.draw_curve(&painter, r, curve, Color32::from_rgb(66, 100, 98), 3.0);
            }
        }
        let color = match self.editor.acceptance {
            Acceptance::Valid => TEAL,
            Acceptance::Pending => GOLD,
            Acceptance::Invalid(_) => RED,
        };
        for curve in &self.draft_curves {
            self.draw_curve(&painter, r, curve, color, 2.0);
        }
        for o in &self.editor.document.draft.obstacles {
            let selected = self.selection.is_some_and(|s| s.0 == o.id);
            let points = o.spline.controls();
            if self.polygon {
                let mut polygon: Vec<_> = points.iter().map(|p| self.screen(*p, r)).collect();
                polygon.push(polygon[0]);
                painter.add(egui::Shape::line(
                    polygon,
                    Stroke::new(
                        1.0,
                        if selected {
                            Color32::from_rgb(91, 110, 127)
                        } else {
                            Color32::from_rgb(46, 66, 80)
                        },
                    ),
                ));
            }
            if self.handles {
                for (i, p) in points.iter().enumerate() {
                    let pos = self.screen(*p, r);
                    let active = self.selection == Some((o.id, Some(i)));
                    painter.circle_filled(
                        pos,
                        if active { 6.0 } else { 4.0 },
                        if active {
                            GOLD
                        } else {
                            Color32::from_rgb(23, 34, 44)
                        },
                    );
                    painter.circle_stroke(
                        pos,
                        if active { 6.0 } else { 4.0 },
                        Stroke::new(
                            1.5,
                            if selected {
                                color
                            } else {
                                Color32::from_rgb(106, 133, 150)
                            },
                        ),
                    );
                }
            }
        }
        if !self.custom.is_empty() {
            let mut points = self.custom.clone();
            if over
                && let Some(p) = pointer
                && points.len() < 128
            {
                points.push(self.world(p, r));
            }
            let polygon: Vec<_> = points.iter().map(|p| self.screen(*p, r)).collect();
            painter.add(egui::Shape::line(polygon, Stroke::new(1.0, GOLD)));
            for (i, p) in self.custom.iter().enumerate() {
                painter.circle_stroke(
                    self.screen(*p, r),
                    if i == 0 { 7.0 } else { 4.0 },
                    Stroke::new(1.5, GOLD),
                );
            }
            if self.custom.len() >= 4
                && let Ok(spline) = PeriodicCubicSpline::uniform(self.custom.clone())
                && let Ok(samples) = sample(
                    &spline,
                    SamplingOptions {
                        tolerance: 0.6 / self.scale,
                        ..Default::default()
                    },
                )
            {
                self.draw_curve(
                    &painter,
                    r,
                    &Curve {
                        id: ObstacleId(0),
                        samples,
                    },
                    GOLD,
                    2.0,
                );
            }
        }
        painter.text(
            r.left_top() + egui::vec2(18.0, 16.0),
            egui::Align2::LEFT_TOP,
            "FIXED DOMAIN  /  [−1, 1]²",
            egui::FontId::monospace(11.0),
            Color32::from_rgb(124, 145, 161),
        );
        if self.sampling_warning {
            painter.text(
                r.left_bottom() + egui::vec2(18.0, -18.0),
                egui::Align2::LEFT_BOTTOM,
                "Render sampling limit reached; zoom out to see curves",
                egui::FontId::proportional(12.0),
                GOLD,
            );
        }
        r
    }
    fn hit_handle(&self, p: Pos2, r: Rect) -> Option<(ObstacleId, usize)> {
        if !self.handles {
            return None;
        }
        self.editor
            .document
            .draft
            .obstacles
            .iter()
            .flat_map(|o| {
                o.spline
                    .controls()
                    .iter()
                    .enumerate()
                    .map(move |(i, c)| (o.id, i, self.screen(*c, r).distance(p)))
            })
            .filter(|x| x.2 <= 10.0)
            .min_by(|a, b| a.2.total_cmp(&b.2))
            .map(|(id, i, _)| (id, i))
    }
    fn hit_curve(&self, p: Pos2, r: Rect) -> Option<(ObstacleId, f64)> {
        let world = self.world(p, r);
        self.draft_curves
            .iter()
            .filter_map(|c| {
                let distance = c
                    .samples
                    .windows(2)
                    .map(|s| point_segment_distance(world, s[0].point, s[1].point))
                    .fold(f64::INFINITY, f64::min);
                (distance * self.scale <= 8.0).then_some((c, distance))
            })
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(c, _)| {
                let spline = &self.editor.obstacle(c.id).unwrap().spline;
                let nearest_knot = spline
                    .knots()
                    .iter()
                    .copied()
                    .map(|t| (t, self.screen(spline.evaluate(t), r).distance(p)))
                    .min_by(|a, b| a.1.total_cmp(&b.1));
                let t = match nearest_knot {
                    Some((t, distance)) if distance <= 4.0 => t,
                    _ => closest_parameter(spline, &c.samples, world),
                };
                (c.id, t)
            })
    }

    fn draw_curve(
        &self,
        painter: &egui::Painter,
        r: Rect,
        curve: &Curve,
        color: Color32,
        width: f32,
    ) {
        if curve.samples.len() > 1 {
            painter.add(egui::Shape::line(
                curve
                    .samples
                    .iter()
                    .map(|s| self.screen(s.point, r))
                    .collect(),
                Stroke::new(width, color),
            ));
        }
    }
}
pub fn frame(mut contexts: EguiContexts, mut state: ResMut<Playground>, time: Res<Time>) -> Result {
    let ctx = contexts.ctx_mut()?;
    if !state.ready {
        ctx.set_visuals(egui::Visuals::dark());
        ctx.options_mut(|o| o.max_passes = 1.try_into().unwrap());
        state.ready = true;
        #[cfg(target_arch = "wasm32")]
        if let Some(element) = web_sys::window()
            .and_then(|w| w.document())
            .and_then(|d| d.get_element_by_id("startup"))
        {
            let _ = element.set_attribute("hidden", "");
        }
    }
    state.frame_ms = state.frame_ms * 0.95 + time.delta_secs() * 1000.0 * 0.05;
    state.update_files();
    let mut root = egui::Ui::new(
        ctx.clone(),
        "root".into(),
        egui::UiBuilder::new()
            .layer_id(egui::LayerId::background())
            .max_rect(ctx.viewport_rect()),
    );
    state.show(&mut root);
    state.editor.validate_frame(12_000);
    state.refresh_mesh();
    Ok(())
}

impl Playground {
    fn show(&mut self, root: &mut egui::Ui) -> Rect {
        // Remember capture before panels can end a text edit this frame.
        self.keyboard_captured = root.ctx().text_edit_focused();
        let state = self;
        egui::Panel::bottom("status")
            .exact_size(29.0)
            .resizable(false)
            .show(root, |ui| {
                ui.horizontal(|ui| {
                    let (text, color) = match &state.editor.acceptance {
                        Acceptance::Valid => ("Accepted · geometry valid".into(), TEAL),
                        Acceptance::Pending => ("Checking draft…".into(), GOLD),
                        Acceptance::Invalid(issue) => {
                            (format!("Draft not accepted · {issue}"), RED)
                        }
                    };
                    ui.colored_label(color, text);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.small(format!(
                            "{:.1} ms  ·  {:.0} px/unit  ·  {}",
                            state.frame_ms,
                            state.scale,
                            if cfg!(target_arch = "wasm32") {
                                "WebGPU"
                            } else {
                                "wgpu"
                            }
                        ));
                    });
                });
            });
        egui::Panel::left("tools")
            .exact_size(280.0)
            .resizable(false)
            .show(root, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    let enabled = !state.file_busy && state.load.is_none();
                    ui.add_enabled_ui(enabled, |ui| state.panel(ui));
                });
            });
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(root, |ui| state.viewport(ui))
            .inner
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Event, Key, Modifiers, PointerButton};
    struct Harness {
        state: Playground,
        ctx: egui::Context,
        rect: Rect,
        time: f64,
        size: egui::Vec2,
        texts: Vec<(String, Rect)>,
    }
    impl Harness {
        fn new() -> Self {
            let ctx = egui::Context::default();
            ctx.options_mut(|o| o.max_passes = 1.try_into().unwrap());
            let mut h = Self {
                state: Playground::default(),
                ctx,
                rect: Rect::NOTHING,
                time: 0.0,
                size: egui::vec2(1280.0, 800.0),
                texts: vec![],
            };
            h.frame(vec![]);
            h.frame(vec![]);
            h
        }
        fn frame(&mut self, events: Vec<Event>) {
            self.time += 1.0 / 60.0;
            let output = self.ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, self.size)),
                    time: Some(self.time),
                    events,
                    focused: true,
                    ..Default::default()
                },
                |ui| self.rect = self.state.show(ui),
            );
            self.texts.clear();
            fn collect(shape: &egui::Shape, texts: &mut Vec<(String, Rect)>) {
                match shape {
                    egui::Shape::Text(t) => texts.push((
                        t.galley.job.text.clone(),
                        Rect::from_min_size(t.pos, t.galley.size()),
                    )),
                    egui::Shape::Vec(shapes) => {
                        for shape in shapes {
                            collect(shape, texts);
                        }
                    }
                    _ => {}
                }
            }
            for shape in &output.shapes {
                collect(&shape.shape, &mut self.texts);
            }
            output.drop_without_applying_deltas();
            self.state.editor.validate_frame(12_000);
        }
        fn point(&self, p: Point2) -> Pos2 {
            self.state.screen(p, self.rect)
        }
        fn move_to(&mut self, p: Pos2) {
            self.frame(vec![Event::PointerMoved(p)]);
        }
        fn button(&mut self, p: Pos2, button: PointerButton, pressed: bool) {
            self.frame(vec![
                Event::PointerMoved(p),
                Event::PointerButton {
                    pos: p,
                    button,
                    pressed,
                    modifiers: Modifiers::NONE,
                },
            ]);
        }
        fn click(&mut self, p: Pos2) {
            self.move_to(p);
            self.button(p, PointerButton::Primary, true);
            self.button(p, PointerButton::Primary, false);
        }
        fn key(&mut self, key: Key, modifiers: Modifiers) {
            self.frame(vec![
                Event::ModifiersChanged(modifiers),
                Event::Key {
                    key,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers,
                },
            ]);
            self.frame(vec![
                Event::Key {
                    key,
                    physical_key: None,
                    pressed: false,
                    repeat: false,
                    modifiers,
                },
                Event::ModifiersChanged(Modifiers::NONE),
            ]);
        }
        fn wheel(&mut self, p: Pos2) {
            self.frame(vec![
                Event::PointerMoved(p),
                Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta: egui::vec2(0.0, 60.0),
                    phase: egui::TouchPhase::Move,
                    modifiers: Modifiers::NONE,
                },
            ]);
        }
        fn settle(&mut self) {
            for _ in 0..130 {
                self.frame(vec![]);
                if self.state.editor.acceptance != Acceptance::Pending {
                    return;
                }
            }
            panic!("did not settle");
        }
    }
    #[test]
    fn pointer_drag_escape_and_one_entry_history() {
        let mut h = Harness::new();
        let before = h.state.editor.document.clone();
        let p = h.point(Point2::new(0.15, 0.0));
        h.click(p);
        assert_eq!(h.state.selection, Some((ObstacleId(1), Some(0))));
        assert_eq!(h.state.editor.history_len(), (0, 0));
        h.button(p, PointerButton::Primary, true);
        let moved = h.point(Point2::new(0.3, 0.1));
        h.move_to(moved);
        h.move_to(moved + egui::vec2(15.0, 0.0));
        h.button(moved, PointerButton::Primary, false);
        h.settle();
        assert_eq!(h.state.editor.history_len(), (1, 0));
        assert_ne!(h.state.editor.document, before);
        h.key(Key::Z, Modifiers::COMMAND);
        assert_eq!(h.state.editor.document, before);
        h.key(Key::Z, Modifiers::COMMAND | Modifiers::SHIFT);
        assert_ne!(h.state.editor.document, before);
        let before = h.state.editor.document.clone();
        let p = h.point(before.draft.obstacles[0].spline.controls()[0]);
        h.button(p, PointerButton::Primary, true);
        h.move_to(p + egui::vec2(25.0, 0.0));
        h.key(Key::Escape, Modifiers::NONE);
        h.button(p, PointerButton::Primary, false);
        assert_eq!(h.state.editor.document, before);
        assert_eq!(h.state.editor.history_len(), (1, 0));
    }
    #[test]
    fn both_creation_workflows_and_custom_cancel() {
        let mut h = Harness::new();
        h.state.mode = Mode::Preset;
        h.click(h.point(Point2::new(0.5, 0.4)));
        assert_eq!(h.state.editor.document.draft.obstacles.len(), 2);
        assert!(h.state.mode == Mode::Select);
        assert_eq!(h.state.editor.history_len().0, 1);
        h.state.mode = Mode::Custom;
        for p in [
            Point2::new(-0.7, -0.4),
            Point2::new(-0.4, -0.4),
            Point2::new(-0.4, -0.7),
            Point2::new(-0.7, -0.7),
        ] {
            h.click(h.point(p));
        }
        assert_eq!(h.state.custom.len(), 4);
        h.key(Key::Enter, Modifiers::NONE);
        assert_eq!(h.state.editor.document.draft.obstacles.len(), 3);
        assert_eq!(h.state.editor.history_len().0, 2);
        h.settle();
        assert_eq!(h.state.editor.acceptance, Acceptance::Valid);
        h.state.mode = Mode::Custom;
        h.click(h.point(Point2::new(0.4, -0.3)));
        h.click(h.point(Point2::new(0.6, -0.3)));
        h.key(Key::Backspace, Modifiers::NONE);
        assert_eq!(h.state.custom.len(), 1);
        h.key(Key::Escape, Modifiers::NONE);
        assert!(h.state.custom.is_empty());
        assert_eq!(h.state.editor.history_len().0, 2);
    }
    #[test]
    fn zoom_is_cursor_centered_and_panel_scroll_does_not_zoom() {
        let mut h = Harness::new();
        let p = h.point(Point2::new(0.4, 0.4));
        h.move_to(p);
        let before = h.state.world(p, h.rect);
        let scale = h.state.scale;
        h.wheel(p);
        assert!(h.state.scale > scale);
        assert!((h.state.world(p, h.rect) - before).norm() < 1e-6);
        for _ in 0..40 {
            h.frame(vec![]);
        }
        let scale = h.state.scale;
        h.wheel(Pos2::new(100.0, 500.0));
        h.move_to(p);
        for _ in 0..20 {
            h.frame(vec![]);
        }
        assert_eq!(h.state.scale, scale);
        assert_eq!(h.state.editor.history_len(), (0, 0));
        let center = h.state.center;
        h.size = egui::vec2(900.0, 600.0);
        h.frame(vec![]);
        assert_eq!(h.state.center, center);
        let p = Point2::new(-0.7, 0.3);
        assert!((h.state.world(h.point(p), h.rect) - p).norm() < 1e-6);
    }
    #[test]
    fn panning_and_panel_capture_never_edit_geometry() {
        let mut h = Harness::new();
        let before = h.state.editor.document.clone();
        let center = h.state.center;
        let p = h.rect.center();
        h.button(p, PointerButton::Middle, true);
        h.move_to(p + egui::vec2(40.0, 20.0));
        h.button(p + egui::vec2(40.0, 20.0), PointerButton::Middle, false);
        assert_ne!(h.state.center, center);
        assert_eq!(h.state.editor.document, before);
        let center = h.state.center;
        let panel = Pos2::new(100.0, 500.0);
        h.button(panel, PointerButton::Primary, true);
        h.move_to(p);
        h.button(p, PointerButton::Primary, false);
        assert_eq!(h.state.center, center);
        assert_eq!(h.state.editor.document, before);
        assert_eq!(h.state.editor.history_len(), (0, 0));
    }
    #[test]
    fn double_click_inserts_once_and_existing_knot_selects() {
        let mut h = Harness::new();
        h.state.handles = false;
        let spline = h.state.editor.document.draft.obstacles[0].spline.clone();
        let p = h.point(spline.evaluate(0.5));
        h.click(p);
        h.click(p);
        assert_eq!(
            h.state.editor.document.draft.obstacles[0]
                .spline
                .controls()
                .len(),
            9
        );
        assert_eq!(h.state.editor.history_len().0, 1);
        h.settle();
        assert_eq!(h.state.editor.acceptance, Acceptance::Valid);
        h.time += 1.0;
        h.click(p);
        h.click(p);
        assert_eq!(
            h.state.editor.document.draft.obstacles[0]
                .spline
                .controls()
                .len(),
            9
        );
        assert_eq!(h.state.editor.history_len().0, 1);
        assert!(h.state.selection.unwrap().1.is_some());
        h.key(Key::Delete, Modifiers::NONE);
        assert_eq!(
            h.state.editor.document.draft.obstacles[0]
                .spline
                .controls()
                .len(),
            8
        );
        assert_eq!(h.state.editor.history_len().0, 2);
    }
    #[test]
    fn numeric_edit_commits_once_and_typing_captures_editor_keys() {
        let mut h = Harness::new();
        h.click(h.point(Point2::new(0.15, 0.0)));
        h.state.mode = Mode::Custom;
        h.state.custom = vec![
            Point2::new(-0.7, 0.4),
            Point2::new(-0.4, 0.4),
            Point2::new(-0.4, 0.7),
            Point2::new(-0.7, 0.7),
        ];
        h.frame(vec![]);
        let x = h
            .texts
            .iter()
            .find(|(text, _)| text.starts_with("x "))
            .unwrap()
            .1
            .center();
        h.click(x);
        assert!(h.ctx.text_edit_focused());
        h.move_to(h.rect.center());
        h.key(Key::Delete, Modifiers::NONE);
        assert_eq!(
            h.state.editor.document.draft.obstacles[0]
                .spline
                .controls()
                .len(),
            8
        );
        h.frame(vec![Event::Text("0.25".into())]);
        h.key(Key::Enter, Modifiers::NONE);
        assert_eq!(h.state.editor.document.draft.obstacles.len(), 1);
        assert_eq!(h.state.custom.len(), 4);
        assert_eq!(
            h.state.editor.document.draft.obstacles[0].spline.controls()[0].x,
            0.25
        );
        assert_eq!(h.state.editor.history_len(), (1, 0));
        h.key(Key::Z, Modifiers::COMMAND);
        assert_eq!(
            h.state.editor.document.draft.obstacles[0].spline.controls()[0].x,
            0.15
        );
    }
    #[test]
    fn space_pan_and_custom_close_by_first_handle() {
        let mut h = Harness::new();
        let center = h.state.center;
        let p = h.rect.center();
        h.move_to(p);
        h.frame(vec![Event::Key {
            key: Key::Space,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        }]);
        h.button(p, PointerButton::Primary, true);
        h.move_to(p + egui::vec2(50.0, 0.0));
        h.button(p + egui::vec2(50.0, 0.0), PointerButton::Primary, false);
        h.frame(vec![Event::Key {
            key: Key::Space,
            physical_key: None,
            pressed: false,
            repeat: false,
            modifiers: Modifiers::NONE,
        }]);
        assert_ne!(center, h.state.center);
        assert_eq!(h.state.editor.history_len(), (0, 0));
        h.state.mode = Mode::Custom;
        let points = [
            Point2::new(-0.7, 0.4),
            Point2::new(-0.4, 0.4),
            Point2::new(-0.4, 0.7),
            Point2::new(-0.7, 0.7),
        ];
        for p in points {
            h.click(h.point(p));
        }
        h.click(h.point(points[0]));
        assert!(h.state.custom.is_empty());
        assert_eq!(h.state.editor.document.draft.obstacles.len(), 2);
        assert_eq!(h.state.editor.history_len(), (1, 0));
    }

    #[test]
    fn accepted_mesh_finishes_cooperatively_and_survives_an_invalid_draft() {
        let mut h = Harness::new();
        for _ in 0..5_000 {
            h.state.refresh_mesh();
            if h.state.mesh.is_some() {
                break;
            }
        }
        let mesh = h.state.mesh.clone().expect("initial accepted mesh");
        assert_eq!(mesh.geometry_revision, 0);
        assert!(mesh.triangles.len() > 10_000);
        assert!(mesh.quality.maximum_edge_length <= 0.04);
        assert_eq!(h.state.mesh_low_quality.len(), mesh.triangles.len());

        h.state.editor.begin();
        h.state
            .editor
            .set_point(ObstacleId(1), 0, Point2::new(8.0, 0.0))
            .unwrap();
        h.state.editor.commit();
        h.settle();
        assert!(matches!(h.state.editor.acceptance, Acceptance::Invalid(_)));
        h.state.refresh_mesh();
        assert_eq!(h.state.mesh.as_ref().unwrap(), &mesh);
        assert!(h.state.mesh_job.is_none());

        // Resolution is part of the mesh request even when the accepted scene
        // and document revision stay unchanged. It does not enter edit history.
        let document = h.state.editor.document.clone();
        let history = h.state.editor.history_len();
        h.state.mesh_max_edge = 0.16;
        for _ in 0..5_000 {
            h.state.refresh_mesh();
            if h.state.mesh_job.is_none() {
                break;
            }
        }
        assert_eq!(h.state.mesh_source_max_edge, 0.16);
        let preview = h.state.mesh.as_ref().unwrap();
        assert!(preview.quality.maximum_edge_length <= 0.16);
        assert!(preview.triangles.len() < mesh.triangles.len() / 4);
        assert_eq!(h.state.editor.document, document);
        assert_eq!(h.state.editor.history_len(), history);
    }
}
