//! Probes end to end: the Probes inspector and its editors, the GPU
//! recorder configuration, and folding readbacks back into the traces.

use crate::wave_gpu::{
    AreaProbeDisplay, AreaProbeInput, CurveProbeDisplay, CurveProbeInput, FarFieldDisplay,
    FarFieldHandoff, FarFieldInput, ProbeDisplay, RecorderContext, RecorderHistory, WaveGpuRequest,
};
use bevy::prelude::*;
use bevy::render::storage::ShaderBuffer;
use bevy_egui::egui::{self};
use funfern_app::document::{ProbeId, ProbeSamplingPreset};
use funfern_app::topology_editor::{TopologyBoundaryProbeTarget, TopologyProbeTarget};
use funfern_app::topology_runtime::{
    PreparedTopology, TopologyProbeCompilation, TopologyProbeStencil,
};
use funfern_app::topology_viewport::{TopologySelection, TopologySpanTarget};
use funfern_core::*;
use std::collections::BTreeSet;
use std::sync::Arc;

use super::*;

impl Playground {
    pub(super) fn selected_boundary_probe_target(&self) -> Option<TopologyBoundaryProbeTarget> {
        let selected = self.selection.spans()?;
        let span_ids = selected
            .iter()
            .filter_map(|target| match target {
                TopologySpanTarget::Curve(span) => Some(*span),
                TopologySpanTarget::Outer(_) => None,
            })
            .collect::<BTreeSet<_>>();
        if span_ids.is_empty() || span_ids.len() != selected.len() {
            return None;
        }
        let mut matching = self
            .editor
            .document
            .model
            .draft
            .geometry
            .curves
            .iter()
            .filter(|curve| curve.spans.iter().any(|span| span_ids.contains(&span.id)));
        let curve = matching.next()?;
        if matching.next().is_some() {
            return None;
        }
        let spans = curve
            .spans
            .iter()
            .filter(|span| span_ids.contains(&span.id))
            .map(|span| span.id)
            .collect::<Vec<_>>();
        (spans.len() == span_ids.len()).then_some(TopologyBoundaryProbeTarget {
            curve: curve.id,
            spans,
            side: self.selected_side,
            reversed: false,
            preset: ProbeSamplingPreset::Medium,
        })
    }
    pub(super) fn probes_panel(&mut self, ui: &mut egui::Ui) {
        ui.heading("Probes");
        ui.horizontal_wrapped(|ui| {
            for (mode, label, hint) in [
                (
                    ProbePlacement::Point,
                    "Point",
                    "Click in the scene to place a point probe",
                ),
                (
                    ProbePlacement::Segment { start: None },
                    "Line",
                    "Click the line's start, then its end",
                ),
                (
                    ProbePlacement::Disk { center: None },
                    "Disk",
                    "Click the disk's centre, then a point on its rim",
                ),
                (
                    ProbePlacement::Region,
                    "Region",
                    "Click a subdomain to probe the whole of it",
                ),
            ] {
                let selected = self.probe_mode.is_some_and(|active| {
                    std::mem::discriminant(&active) == std::mem::discriminant(&mode)
                });
                if ui
                    .add(egui::Button::new(format!("+ {label}")).selected(selected))
                    .on_hover_text(hint)
                    .clicked()
                {
                    self.probe_mode = (!selected).then_some(mode);
                    self.pulse_mode = false;
                }
            }
        });
        let boundary_target = self.selected_boundary_probe_target();
        if ui
            .add_enabled(
                boundary_target.is_some(),
                egui::Button::new("+ Selected boundary"),
            )
            .on_hover_text("Create a boundary probe from one span selection on one curve")
            .clicked()
            && let Some(target) = boundary_target
        {
            match self.editor.create_probe(
                format!("Boundary {}", self.editor.document.model.probes.len() + 1),
                [248, 196, 112],
                TopologyProbeTarget::Boundary(target),
            ) {
                Ok(id) => {
                    self.selected_probe = Some(id);
                    self.probe_windows.insert(id);
                    self.notify("Boundary probe added");
                }
                Err(error) => self.notify(error),
            }
        }
        ui.add(
            egui::Slider::new(&mut self.probe_history_seconds, 2.0..=60.0)
                .logarithmic(true)
                .text("History (sim s)"),
        )
        .on_hover_text("Longest time window a readout can show");
        ui.separator();
        let probes = self.editor.document.model.probes.clone();
        for mut probe in probes {
            ui.horizontal(|ui| {
                let open = self.probe_windows.contains(&probe.id);
                if ui
                    .selectable_label(self.selected_probe == Some(probe.id), &probe.name)
                    .clicked()
                {
                    self.selected_probe = Some(probe.id);
                    self.selection = TopologySelection::None;
                }
                if ui
                    .add(egui::Button::new("Plot").selected(open))
                    .on_hover_text("Show or hide this probe's readout window")
                    .clicked()
                {
                    if open {
                        self.probe_windows.remove(&probe.id);
                    } else {
                        self.probe_windows.insert(probe.id);
                    }
                }
                if ui
                    .checkbox(&mut probe.enabled, "Live")
                    .on_hover_text("Record samples from the solver")
                    .changed()
                    && let Err(error) = self.editor.update_probe(probe.clone())
                {
                    self.notify(error);
                }
                if ui.small_button("×").on_hover_text("Delete").clicked() {
                    if let Err(error) = self.editor.delete_probe(probe.id) {
                        self.notify(error)
                    } else {
                        self.probe_windows.remove(&probe.id);
                        if self.selected_probe == Some(probe.id) {
                            self.selected_probe = None;
                        }
                    }
                }
            });
            if let Some(status) = self.probe_status.get(&probe.id) {
                ui.small(egui::RichText::new(status).color(GOLD));
            }
        }
        if let Some(id) = self.selected_probe {
            self.selected_probe_editor(ui, id);
        }
        ui.separator();
        let mut far = self.editor.document.model.far_field;
        if ui.checkbox(&mut far.enabled, "Far field").changed() {
            if let Err(error) = self.editor.set_far_field(far) {
                self.notify(error)
            }
        }
        if ui
            .add_enabled(far.enabled, egui::Button::new("Open far-field readout"))
            .clicked()
        {
            self.far_field_window = true;
        }
        if ui
            .add(
                egui::DragValue::new(&mut far.inset)
                    .speed(0.005)
                    .range(0.001..=10.0)
                    .prefix("Inset "),
            )
            .changed()
        {
            if let Err(error) = self.editor.set_far_field(far) {
                self.notify(error)
            }
        }
        if let Some(recorded) = self.far_field_recording() {
            ui.small(
                egui::RichText::new(format!(
                    "Recording the delay window · {:.0}%",
                    recorded * 100.0
                ))
                .color(GOLD),
            )
            .on_hover_text(
                "Every observation angle reads the contour at its own retarded time, \
                 so the recorder reports nothing until it holds a whole window. \
                 A remesh keeps what it has; moving the contour starts it again.",
            );
        }
    }
    /// Name, color, and target settings for the selected probe. The name is
    /// committed when the field loses focus so every keystroke is not a
    /// separate document revision.
    pub(super) fn selected_probe_editor(&mut self, ui: &mut egui::Ui, id: ProbeId) {
        let Some(mut probe) = self
            .editor
            .document
            .model
            .probes
            .iter()
            .find(|probe| probe.id == id)
            .cloned()
        else {
            self.selected_probe = None;
            self.probe_name_edit = None;
            return;
        };
        ui.separator();
        ui.label(match probe.target {
            TopologyProbeTarget::Point(_) => "Selected point probe",
            TopologyProbeTarget::Segment { .. } => "Selected line probe",
            TopologyProbeTarget::Boundary(_) => "Selected boundary probe",
            TopologyProbeTarget::AreaDisk { .. } => "Selected disk probe",
            TopologyProbeTarget::AreaRegion(_) => "Selected region probe",
        });
        if !matches!(self.probe_name_edit.as_ref(), Some((candidate, _)) if *candidate == id) {
            self.probe_name_edit = Some((id, probe.name.clone()));
        }
        let mut commit_name = None;
        if let Some((_, name)) = self.probe_name_edit.as_mut() {
            let response = ui.add(egui::TextEdit::singleline(name).hint_text("Probe name"));
            if (response.lost_focus()
                || response
                    .ctx
                    .input(|input| input.key_pressed(egui::Key::Enter)))
                && !name.trim().is_empty()
                && name.len() <= 64
            {
                commit_name = Some(name.trim().to_owned());
            }
        }
        if let Some(name) = commit_name
            && name != probe.name
        {
            probe.name = name;
            if let Err(error) = self.editor.update_probe(probe.clone()) {
                self.notify(error);
            }
        }
        ui.horizontal(|ui| {
            ui.label("Color");
            if ui.color_edit_button_srgb(&mut probe.color).changed()
                && let Err(error) = self.editor.update_probe(probe.clone())
            {
                self.notify(error);
            }
        });
        let mut changed = false;
        match &mut probe.target {
            TopologyProbeTarget::Segment { start, end, preset } => {
                changed |= sampling_preset_picker(ui, id, preset);
                if ui
                    .button("Swap ends")
                    .on_hover_text("Reverse the arclength axis and the normal flux sign")
                    .clicked()
                {
                    std::mem::swap(start, end);
                    changed = true;
                }
            }
            TopologyProbeTarget::Boundary(target) => {
                changed |= sampling_preset_picker(ui, id, &mut target.preset);
                let mut side = target.side;
                ui.horizontal(|ui| {
                    ui.label("Trace side").on_hover_text(
                        "Which of the span's two traces is read. In the scene the arrow \
                         arrives at the marker from that side, along the way positive flux \
                         points",
                    );
                    for (value, label) in [
                        (CurveTraceSide::Left, "Left"),
                        (CurveTraceSide::Right, "Right"),
                    ] {
                        if ui.selectable_label(side == value, label).clicked() {
                            side = value;
                        }
                    }
                });
                if side != target.side {
                    target.side = side;
                    changed = true;
                }
                let mut reversed = target.reversed;
                if ui
                    .checkbox(&mut reversed, "Reverse direction")
                    .on_hover_text(
                        "Sample the path against increasing curve parameter. The second \
                         arrow in the scene follows it",
                    )
                    .changed()
                {
                    target.reversed = reversed;
                    changed = true;
                }
                ui.small(format!("{} spans", target.spans.len()));
            }
            TopologyProbeTarget::AreaDisk { radius, .. } => {
                changed |= ui
                    .add(
                        egui::DragValue::new(radius)
                            .speed(0.005)
                            .range(0.001..=10.0)
                            .prefix("Radius "),
                    )
                    .changed();
            }
            TopologyProbeTarget::Point(_) | TopologyProbeTarget::AreaRegion(_) => {}
        }
        if changed && let Err(error) = self.editor.update_probe(probe) {
            self.notify(error);
        }
        if let Some((length, closed)) = self.probe_metrics.get(&id).copied()
            && length > 0.0
        {
            ui.small(format!(
                "Path length {length:.4}{}",
                if closed { " · closed" } else { "" }
            ));
        }
        if ui.button("Clear recorded samples").clicked() {
            self.clear_probe_trace(id);
        }
    }
    pub(super) fn configure_probes(
        &mut self,
        request: &mut WaveGpuRequest,
        assets: &mut Assets<ShaderBuffer>,
        commands: &mut Commands,
        active: &Arc<PreparedTopology>,
        source: RecorderSource<'_>,
    ) {
        let mut points = vec![];
        let mut curves = vec![];
        let mut areas = vec![];
        for compiled in active.probes.iter() {
            let definition = self
                .editor
                .document
                .model
                .probes
                .iter()
                .find(|probe| probe.id == compiled.id);
            let Some(definition) = definition else {
                continue;
            };
            match &compiled.result {
                TopologyProbeCompilation::Ready(stencil) => match stencil.as_ref() {
                    TopologyProbeStencil::Point(stencil) => {
                        points.push((compiled.id.0, Some(*stencil)))
                    }
                    TopologyProbeStencil::Segment(stencils) => {
                        if let TopologyProbeTarget::Segment { start, end, preset } =
                            definition.target
                        {
                            let count = stencils.len();
                            curves.push(CurveProbeInput {
                                id: compiled.id.0,
                                sample_rate: preset.sample_rate(),
                                samples: stencils
                                    .iter()
                                    .enumerate()
                                    .map(|(index, stencil)| {
                                        Some((
                                            *stencil,
                                            start.lerp(
                                                end,
                                                index as f64 / (count - 1).max(1) as f64,
                                            ),
                                        ))
                                    })
                                    .collect(),
                            });
                        }
                    }
                    TopologyProbeStencil::Boundary(stencils) => {
                        let rate = match &definition.target {
                            TopologyProbeTarget::Boundary(target) => target.preset.sample_rate(),
                            _ => 60.0,
                        };
                        curves.push(CurveProbeInput {
                            id: compiled.id.0,
                            sample_rate: rate,
                            samples: stencils
                                .iter()
                                .map(|sample| Some((sample.stencil, sample.point)))
                                .collect(),
                        });
                    }
                    TopologyProbeStencil::Area(stencil) => areas.push(AreaProbeInput {
                        id: compiled.id.0,
                        stencil: Some(stencil.clone()),
                    }),
                },
                TopologyProbeCompilation::Disabled => {}
                TopologyProbeCompilation::Failed(error) => {
                    self.message = format!("Probe {}: {error}", definition.name)
                }
            }
        }
        let dt = self.solver_time_step();
        let physics = active.bundle.authored.physics;
        if self
            .probe_history_physics
            .is_some_and(|previous| previous != physics)
        {
            self.restart_probe_traces();
        }
        self.probe_history_physics = Some(physics);
        // Every recorder writes a time series on the solver's own clock, and a
        // transfer carries that clock across. So a new mesh over the same
        // recorders takes over the rings they were filling, and only a clock
        // that restarted starts them again. Without that the samples the GPU
        // wrote since the last readback went with the buffers - two to four of
        // them at 120 Hz, on every adaptation.
        let restarted = std::mem::take(&mut self.probe_clock_restarted);
        let history = if restarted {
            RecorderHistory::Restart
        } else {
            RecorderHistory::Keep
        };
        let context = RecorderContext {
            time_step: dt,
            physics,
            history,
        };
        let result = source
            .point_probes(request, assets, commands, &points, 120.0, context)
            .and_then(|()| source.curve_probes(request, assets, commands, &curves, context))
            .and_then(|()| source.area_probes(request, assets, commands, &areas, 60.0, context));
        if let Err(error) = result {
            self.message = error;
        }
        let far = active
            .far_field
            .as_ref()
            .and_then(|result| result.as_ref().ok())
            .map(|stencil| FarFieldInput {
                samples: stencil.samples.clone(),
                wave_speed: stencil.wave_speed,
                sample_spacing: stencil.sample_spacing,
                delay_margin: stencil.delay_margin,
            });
        // The far field records at fixed world points rather than at a probe,
        // so a contour that moved starts its delay window again even when the
        // clock did not.
        match request.update_canonical_far_field(assets, commands, far.as_ref(), dt, history) {
            Ok(FarFieldHandoff::Restarted) => {
                self.far_field_trace = FarFieldTrace::default();
                self.far_field_recording_from = Some(self.simulated_time());
            }
            Ok(FarFieldHandoff::Off) => self.far_field_recording_from = None,
            Ok(FarFieldHandoff::Kept) => {}
            Err(error) => {
                self.far_field_recording_from = None;
                self.message = error;
            }
        }
        // Recorded whatever happened: a rejected recorder setting is rejected
        // the same way every frame, and the readback filter below keys on these
        // revisions, so what the GPU actually holds is what is written down.
        // A restarted clock disowns the run before it; anything else leaves the
        // last upload addressable, for the readback still on its way here.
        self.probe_upload_previous = (!restarted).then_some(self.probe_upload).flatten();
        self.probe_upload = Some(ProbeUpload {
            token: active.bundle.token,
            generation: request.generation(),
            revision: request.probe_revision(),
            curve_revision: request.curve_probe_revision(),
            area_revision: request.area_probe_revision(),
            far_field_revision: request.far_field_revision(),
        });
    }
    /// The simulation clock is starting over at zero. Ingestion only takes a
    /// record newer than the trace's last one, so a trace carried across the
    /// restart would refuse the entire new run — which is what made a reset
    /// look like it had frozen the probes until the traces were cleared by hand.
    pub(super) fn restart_probe_traces(&mut self) {
        self.probe_traces.clear();
        self.curve_probe_traces.clear();
        self.area_probe_traces.clear();
        self.far_field_trace = FarFieldTrace::default();
        self.probe_clock_restarted = true;
        self.probe_upload_previous = None;
    }
    /// The step the GPU is actually running at.
    /// Whether the resident grid filter actually runs: asked for, and admitted
    /// by the generation. Anything timed against its commits - the lane flip
    /// it leaves at each cadence step - follows this rather than the setting.
    pub(super) fn grid_filter_running(&self) -> bool {
        self.editor.document.presentation.grid_scale_filter && !self.grid_filter_refused
    }

    pub(super) fn solver_time_step(&self) -> f64 {
        if self.uploaded_time_step > 0.0 {
            return self.uploaded_time_step;
        }
        // The generation's own step, which for a driven one is the tighter step
        // its coefficient trajectory demands rather than what its authored
        // coefficients allow. Reading the base operator's here would name a
        // step the running solver never takes.
        self.runtime
            .active()
            .map_or(0.0, |active| active.recommended_time_step())
    }

    /// Republishes when the speed ceiling wants a different step from the one
    /// the solver is running.
    ///
    /// The scene is unchanged, so the preparation reuses the plan, the mesh and
    /// the operator and the field crosses on the identity transfer — the same
    /// path an adaptation handoff takes, which already changes the step every
    /// time it runs. Clearing the requested revision is the lever a document
    /// load pulls.
    pub(super) fn retime_for_speed(&mut self) {
        if self.uploaded_time_step <= 0.0
            || self.uploading.is_some()
            || self.preparation_in_progress()
            || self.runtime.ready().is_some()
        {
            return;
        }
        let Some(active) = self.runtime.active() else {
            return;
        };
        // The step the generation actually runs at, which on a driven medium
        // is the tighter one its coefficient trajectory demands rather than the
        // base operator's. Comparing against the base asked for a step the
        // upload would never choose, so every frame cleared the requested
        // revision and prepared the whole generation again - the phase label
        // churned, adaptation never got a settled generation to analyse, and
        // the frame rate went with it.
        let wanted = paced_time_step(
            active.recommended_time_step(),
            self.editor.document.presentation.simulation_speed,
        );
        if (wanted / self.uploaded_time_step - 1.0).abs() > TIME_STEP_HYSTERESIS {
            self.requested_revision = None;
        }
    }

    pub(super) fn simulated_time(&self) -> f64 {
        self.sim_time_offset + self.completed_steps as f64 * self.solver_time_step()
    }
    /// How much of the delay window the far-field recorder holds, once it is
    /// running and has not filled it yet. Nothing can be projected before it is
    /// whole, and the empty plot says nothing on its own.
    pub(super) fn far_field_recording(&self) -> Option<f64> {
        let history = self
            .runtime
            .active()?
            .far_field
            .as_ref()?
            .as_ref()
            .ok()?
            .history_seconds();
        let from = self.far_field_recording_from?;
        let recorded = (self.simulated_time() - from) / history;
        (history > 0.0 && recorded < 1.0).then(|| recorded.clamp(0.0, 1.0))
    }

    /// A readback carries the generation and the revision it was recorded
    /// against. Anything else is a ring the app no longer owns — the tail of the
    /// run before a reset, or of the probe set before an edit — and its times
    /// belong to a clock the traces have left behind.
    pub(super) fn readback_is_current(
        &self,
        generation: u64,
        revision: u64,
        recorded: impl Fn(&ProbeUpload) -> u64,
    ) -> bool {
        [&self.probe_upload, &self.probe_upload_previous]
            .into_iter()
            .flatten()
            .any(|upload| upload.generation == generation && recorded(upload) == revision)
    }
    /// Every recorder stamps its samples with the solver's own clock, which the
    /// transfer carries into the buffers that replace it. That clock is already
    /// the app's, and adding `sim_time_offset` to it counted the run so far a
    /// second time — a jump the width of the previous mesh's whole lifetime at
    /// every handoff, which is what put the holes in these traces.
    pub(super) fn ingest_probes(&mut self, display: &ProbeDisplay) {
        if display.readbacks == self.probe_readback
            || !self.readback_is_current(display.generation, display.revision, |upload| {
                upload.revision
            })
        {
            return;
        }
        self.probe_readback = display.readbacks;
        for record in &display.records {
            let trace = self
                .probe_traces
                .entry(ProbeId(record.probe_id))
                .or_default();
            let record = *record;
            if record.time > trace.last_time {
                trace.last_time = record.time;
                trace.samples.push_back(record);
                while trace.samples.len() > 4096 {
                    trace.samples.pop_front();
                }
            }
        }
    }
    pub(super) fn ingest_spatial_probes(
        &mut self,
        curves: &CurveProbeDisplay,
        areas: &AreaProbeDisplay,
        far: &FarFieldDisplay,
    ) {
        if curves.readbacks != self.curve_probe_readback
            && self.readback_is_current(curves.generation, curves.revision, |upload| {
                upload.curve_revision
            })
        {
            self.curve_probe_readback = curves.readbacks;
            for record in &curves.records {
                let trace = self
                    .curve_probe_traces
                    .entry(ProbeId(record.probe_id))
                    .or_default();
                let record = record.clone();
                if record.time > trace.last_time {
                    trace.last_time = record.time;
                    trace.records.push_back(record);
                    while trace.records.len() > CURVE_TRACE_FRAMES {
                        trace.records.pop_front();
                    }
                }
            }
        }
        if areas.readbacks != self.area_probe_readback
            && self.readback_is_current(areas.generation, areas.revision, |upload| {
                upload.area_revision
            })
        {
            self.area_probe_readback = areas.readbacks;
            for record in &areas.records {
                let trace = self
                    .area_probe_traces
                    .entry(ProbeId(record.probe_id))
                    .or_default();
                let record = *record;
                if record.time > trace.last_time {
                    trace.last_time = record.time;
                    trace.records.push_back(record);
                    while trace.records.len() > 4096 {
                        trace.records.pop_front();
                    }
                }
            }
        }
        if far.readbacks != self.far_field_readback
            && self.readback_is_current(far.generation, far.revision, |upload| {
                upload.far_field_revision
            })
        {
            self.far_field_readback = far.readbacks;
            for record in &far.records {
                // Stamped on the app's clock by the recorder, which has to hold
                // one across the mesh swaps its ring survives.
                let record = record.clone();
                if record.time > self.far_field_trace.last_time {
                    self.far_field_trace.last_time = record.time;
                    self.far_field_trace.records.push_back(record);
                    while self.far_field_trace.records.len() > 512 {
                        self.far_field_trace.records.pop_front();
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wave_gpu::PointProbeRecord;

    fn probe_upload(token: TopologyToken, generation: u64, revision: u64) -> ProbeUpload {
        ProbeUpload {
            token,
            generation,
            revision,
            curve_revision: revision,
            area_revision: revision,
            far_field_revision: revision,
        }
    }

    /// A driven plan records nothing through a stencil that does not address
    /// its law tables, so a recorder is only built once the generation and the
    /// installed plan agree about whether the medium is driven.
    #[test]
    fn a_recorder_waits_for_the_plan_to_agree_with_the_generation() {
        assert_eq!(recorder_pairing::<(), ()>(None, None), Some(None));
        assert_eq!(recorder_pairing(Some('t'), Some(3)), Some(Some(('t', 3))));
        // A handoff between an inert and a driven generation, either way round.
        assert_eq!(recorder_pairing::<char, u8>(Some('t'), None), None);
        assert_eq!(recorder_pairing::<char, u8>(None, Some(3)), None);
    }

    #[test]
    fn replacing_the_wave_buffers_asks_for_the_probe_buffers_again() {
        let token = TopologyToken {
            document_revision: 4,
            topology_revision: 3,
            mesh_generation: 2,
        };
        let upload = probe_upload(token, 7, 1);
        assert!(probes_need_upload(None, token, 7));
        assert!(!probes_need_upload(Some(upload), token, 7));

        // A reset keeps the topology and replaces the wave buffers, and every
        // probe buffer goes with them. Nothing else says so.
        assert!(probes_need_upload(Some(upload), token, 8));

        // A commit that reuses the buffers still moves the stencils.
        assert!(probes_need_upload(
            Some(upload),
            TopologyToken {
                mesh_generation: 3,
                ..token
            },
            7
        ));
    }

    #[test]
    fn a_restarted_run_records_from_its_own_clock() {
        let sample = |time: f64| PointProbeRecord {
            probe_id: 1,
            time,
            ..PointProbeRecord::default()
        };
        let mut state = Playground::default();
        let token = TopologyToken {
            document_revision: 1,
            topology_revision: 1,
            mesh_generation: 1,
        };
        state.probe_upload = Some(probe_upload(token, 1, 1));
        // A recorder stamps the solver's clock, which the transfer carries from
        // one generation into the next. `sim_time_offset` turns this
        // generation's step count into that clock; adding it to a time already
        // on it counts the run so far a second time.
        state.sim_time_offset = 4.0;
        state.ingest_probes(&ProbeDisplay {
            generation: 1,
            revision: 1,
            records: vec![sample(4.5), sample(5.0)],
            readbacks: 1,
        });
        let samples = |state: &Playground| -> Vec<f64> {
            state
                .probe_traces
                .get(&ProbeId(1))
                .map(|trace| trace.samples.iter().map(|sample| sample.time).collect())
                .unwrap_or_default()
        };
        assert_eq!(samples(&state), vec![4.5, 5.0]);

        // The run restarts: new buffers, new probes, and a clock back at zero.
        // With the previous run's samples still in the trace every record of the
        // new one sits below its high-water mark and is dropped — the freeze
        // that only Clear could undo.
        state.sim_time_offset = 0.0;
        state.probe_upload = Some(probe_upload(token, 2, 2));
        state.ingest_probes(&ProbeDisplay {
            generation: 2,
            revision: 2,
            records: vec![sample(0.25)],
            readbacks: 2,
        });
        assert_eq!(samples(&state), vec![4.5, 5.0]);

        state.restart_probe_traces();

        // A readback still in flight from the run that ended carries times from
        // a clock the trace has left behind, and would land ahead of everything
        // the new run is about to record.
        state.ingest_probes(&ProbeDisplay {
            generation: 1,
            revision: 1,
            records: vec![sample(1.5)],
            readbacks: 3,
        });
        assert_eq!(samples(&state), Vec::<f64>::new());

        state.ingest_probes(&ProbeDisplay {
            generation: 2,
            revision: 2,
            records: vec![sample(0.25)],
            readbacks: 4,
        });
        assert_eq!(samples(&state), vec![0.25]);
    }
}
