//! What the integration tests that run the app on the CPU share: the
//! editor settled and a document prepared as the app prepares it, a run
//! stepped on a generation and the pulse that strikes it, what a point
//! probe reads of it, and the field carried across a handoff as the app
//! carries it.

// Each test file uses its own part of this.
#![allow(dead_code)]

use funfern_app::canonical_gpu::CanonicalGpuRuntimeTransfer;
use funfern_app::topology_editor::{TopologyAcceptance, TopologyDocument, TopologyEditor};
use funfern_app::topology_runtime::{PreparedCanonicalTransfer, PreparedTopology, TopologyRuntime};
use funfern_core::{
    CanonicalHandoffMaps, CanonicalPointStencil, CanonicalStationaryFluxJob,
    CanonicalTemporalWaveState, CanonicalWaveState, DomainRect, GRID_SCALE_FILTER_STRENGTH,
    MeshingOptions, Point2, QuadraticPointStencil,
};
use std::sync::Arc;

pub fn settle(editor: &mut TopologyEditor) {
    for _ in 0..100_000 {
        editor.validate_frame(64);
        if editor.acceptance != TopologyAcceptance::Pending {
            return;
        }
    }
    panic!("validation did not finish");
}

/// Runs the runtime's preparation to its end and publishes it.
pub fn finish(runtime: &mut TopologyRuntime) -> Result<Arc<PreparedTopology>, String> {
    for _ in 0..200_000_000 {
        if let Some(outcome) = runtime.advance(64) {
            let token = outcome.map_err(|error| error.to_string())?;
            return runtime.commit_ready(token);
        }
    }
    Err("preparation did not finish".into())
}

#[derive(Clone)]
pub enum Run {
    Fixed(CanonicalWaveState),
    Temporal(CanonicalTemporalWaveState),
}

/// A smooth pulse of the primary field: a Gaussian `height` high and
/// `width` wide about `centre`.
#[derive(Clone, Copy, Debug)]
pub struct Pulse {
    pub centre: Point2,
    pub width: f64,
    pub height: f64,
}

impl Pulse {
    /// Centred off the domain's middle, where few scenes put anything,
    /// `strength` times its usual height.
    pub fn off_centre(domain: DomainRect, strength: f64) -> Self {
        Self {
            centre: Point2::new(
                domain.min_x + 0.37 * domain.width(),
                domain.min_y + 0.41 * domain.height(),
            ),
            width: 0.15 * domain.width().min(domain.height()),
            height: 0.1 * strength,
        }
    }

    pub fn at(self, point: Point2) -> f64 {
        let offset = point - self.centre;
        self.height * (-offset.dot(offset) / (self.width * self.width)).exp()
    }
}

impl Run {
    /// At rest, then struck by the pulse off the domain's middle,
    /// `strength` times its usual height.
    pub fn struck(
        prepared: &PreparedTopology,
        time_step: f64,
        strength: f64,
    ) -> Result<Self, String> {
        let domain = prepared.bundle.authored.geometry.domain;
        Self::struck_by(prepared, time_step, Pulse::off_centre(domain, strength))
    }

    /// At rest, then struck by `pulse`.
    pub fn struck_by(
        prepared: &PreparedTopology,
        time_step: f64,
        pulse: Pulse,
    ) -> Result<Self, String> {
        let increment = prepared
            .canonical_operator
            .node_points()
            .iter()
            .map(|point| pulse.at(*point))
            .collect::<Vec<_>>();
        let forcing = &prepared.canonical_forcing;
        match &prepared.canonical_temporal_operator {
            Some(temporal) => {
                let mut state = CanonicalTemporalWaveState::zero(temporal, time_step)
                    .map_err(|error| error.to_string())?;
                state
                    .apply_primary_pulse(temporal, forcing, &increment)
                    .map_err(|error| error.to_string())?;
                Ok(Run::Temporal(state))
            }
            None => {
                let mut state = CanonicalWaveState::zero(&prepared.canonical_operator, time_step)
                    .map_err(|error| error.to_string())?;
                state
                    .apply_primary_pulse(&prepared.canonical_operator, forcing, &increment)
                    .map_err(|error| error.to_string())?;
                Ok(Run::Fixed(state))
            }
        }
    }

    /// Struck by `pulse` where it stands, as the app strikes a running field.
    pub fn strike(&mut self, prepared: &PreparedTopology, pulse: Pulse) -> Result<(), String> {
        let increment = prepared
            .canonical_operator
            .node_points()
            .iter()
            .map(|point| pulse.at(*point))
            .collect::<Vec<_>>();
        let forcing = &prepared.canonical_forcing;
        match (self, &prepared.canonical_temporal_operator) {
            (Run::Fixed(state), _) => state
                .apply_primary_pulse(&prepared.canonical_operator, forcing, &increment)
                .map(|_| ()),
            (Run::Temporal(state), Some(temporal)) => state
                .apply_primary_pulse(temporal, forcing, &increment)
                .map(|_| ()),
            (Run::Temporal(_), None) => unreachable!("a temporal state on a fixed generation"),
        }
        .map_err(|error| error.to_string())
    }

    pub fn step(&mut self, prepared: &PreparedTopology) -> Result<(), String> {
        let forcing = &prepared.canonical_forcing;
        match (self, &prepared.canonical_temporal_operator) {
            (Run::Fixed(state), _) => state
                .step_with_forcing(&prepared.canonical_operator, forcing)
                .map(|_| ()),
            (Run::Temporal(state), Some(temporal)) => {
                state.step_with_forcing(temporal, forcing).map(|_| ())
            }
            (Run::Temporal(_), None) => unreachable!("a temporal state on a fixed generation"),
        }
        .map_err(|error| error.to_string())
    }

    /// The grid-scale filter applied once, as the app applies it every
    /// [`funfern_core::GRID_SCALE_FILTER_CADENCE`] steps with the filter on,
    /// as a new document has it.
    pub fn filter(&mut self, prepared: &PreparedTopology) -> Result<(), String> {
        let forcing = &prepared.canonical_forcing;
        match (self, &prepared.canonical_temporal_operator) {
            (Run::Fixed(state), _) => state
                .apply_grid_filter(
                    &prepared.canonical_operator,
                    forcing,
                    GRID_SCALE_FILTER_STRENGTH,
                )
                .map(|_| ()),
            (Run::Temporal(state), Some(temporal)) => state
                .apply_grid_filter_with_forcing(temporal, forcing, GRID_SCALE_FILTER_STRENGTH)
                .map(|_| ()),
            (Run::Temporal(_), None) => unreachable!("a temporal state on a fixed generation"),
        }
        .map_err(|error| error.to_string())
    }

    pub fn primary(&self) -> &[f64] {
        match self {
            Run::Fixed(state) => state.primary_flux(),
            Run::Temporal(state) => state.primary_flux(),
        }
    }

    pub fn complementary(&self) -> &[Point2] {
        match self {
            Run::Fixed(state) => state.complementary_flux(),
            Run::Temporal(state) => state.complementary_flux(),
        }
    }

    pub fn integrated(&self) -> &[f64] {
        match self {
            Run::Fixed(_) => &[],
            Run::Temporal(state) => state.integrated_field(),
        }
    }

    pub fn energy(&self, prepared: &PreparedTopology) -> Result<f64, String> {
        match (self, &prepared.canonical_temporal_operator) {
            (Run::Fixed(state), _) => state.energy(&prepared.canonical_operator),
            (Run::Temporal(state), Some(temporal)) => state.energy(temporal),
            (Run::Temporal(_), None) => unreachable!("a temporal state on a fixed generation"),
        }
        .map_err(|error| error.to_string())
    }

    pub fn bits(&self) -> Vec<u64> {
        self.primary()
            .iter()
            .chain(self.integrated())
            .map(|value| value.to_bits())
            .chain(
                self.complementary()
                    .iter()
                    .flat_map(|value| [value.x.to_bits(), value.y.to_bits()]),
            )
            .collect()
    }
}

/// `document` opened, validated to acceptance and prepared the way the app
/// prepares it, at its own edge, and published: the editor, the runtime that
/// holds it and the generation.
pub fn prepare(
    document: &TopologyDocument,
) -> Result<(TopologyEditor, TopologyRuntime, Arc<PreparedTopology>), String> {
    prepare_at(document, document.presentation.mesh_edge)
}

/// The meshing options the tests prepare `domain` with at the mesh edge
/// `edge`.
pub fn meshing(domain: DomainRect, edge: f64) -> MeshingOptions {
    MeshingOptions {
        target_edge_length: edge,
        curve_tolerance: (edge * 0.02).min(5.0e-4),
        ..MeshingOptions::default()
    }
    .sized_for_area(domain.width() * domain.height())
}

/// [`prepare`] at the mesh edge `edge`.
pub fn prepare_at(
    document: &TopologyDocument,
    edge: f64,
) -> Result<(TopologyEditor, TopologyRuntime, Arc<PreparedTopology>), String> {
    let editor = accepted(document)?;
    let domain = editor.document.model.accepted.geometry.domain;
    prepare_with(editor, meshing(domain, edge))
}

/// `document` opened and validated to acceptance.
pub fn accepted(document: &TopologyDocument) -> Result<TopologyEditor, String> {
    let mut editor = TopologyEditor::from_document(document.clone())
        .map_err(|error| format!("does not load: {error}"))?;
    settle(&mut editor);
    if let TopologyAcceptance::Invalid(issue) = &editor.acceptance {
        return Err(format!("refused at acceptance: {issue}"));
    }
    Ok(editor)
}

/// `editor`'s document, accepted, prepared at `options` and published.
pub fn prepare_with(
    editor: TopologyEditor,
    options: MeshingOptions,
) -> Result<(TopologyEditor, TopologyRuntime, Arc<PreparedTopology>), String> {
    let mut runtime = TopologyRuntime::default();
    runtime.request(
        editor.revision,
        &editor.document,
        editor.compiled_accepted.clone(),
        options,
        true,
    )?;
    let prepared = finish(&mut runtime)?;
    Ok((editor, runtime, prepared))
}

/// `work` done on every item on all threads, the results in the items' order.
pub fn in_parallel<T: Sync, R: Send>(items: &[T], work: impl Fn(&T) -> R + Sync) -> Vec<R> {
    let workers = std::thread::available_parallelism()
        .map(|count| count.get())
        .unwrap_or(4)
        .min(items.len().max(1));
    let work = &work;
    std::thread::scope(|scope| {
        let handles = (0..workers)
            .map(|worker| {
                scope.spawn(move || {
                    items
                        .iter()
                        .enumerate()
                        .skip(worker)
                        .step_by(workers)
                        .map(|(index, item)| (index, work(item)))
                        .collect::<Vec<_>>()
                })
            })
            .collect::<Vec<_>>();
        let mut outcomes = handles
            .into_iter()
            .flat_map(|handle| handle.join().expect("a worker"))
            .collect::<Vec<_>>();
        outcomes.sort_by_key(|(index, _)| *index);
        outcomes.into_iter().map(|(_, outcome)| outcome).collect()
    })
}

/// The stencil of a point probe at `point`, `None` where no element takes
/// it.
pub fn stencil_at(prepared: &PreparedTopology, point: Point2) -> Option<QuadraticPointStencil> {
    QuadraticPointStencil::build_topology(
        &prepared.mesh,
        &prepared.operator,
        &prepared.bundle.plan,
        prepared.fixed_model(),
        point,
    )
    .ok()
}

/// What a point probe reads at `point`: the field and the complementary
/// field. `None` where no element takes the point.
pub fn probe_at(prepared: &PreparedTopology, run: &Run, point: Point2) -> Option<(f64, Point2)> {
    let reading = CanonicalPointStencil::from_quadratic(
        stencil_at(prepared, point)?,
        &prepared.canonical_operator,
    )
    .ok()?
    .sample(run.primary(), run.primary(), run.complementary())
    .ok()?;
    Some((reading.primary, reading.complementary))
}

/// `run` on `prepared` stepped `steps` times.
pub fn advance(run: &mut Run, prepared: &PreparedTopology, steps: u64) -> Result<(), String> {
    for step in 0..steps {
        run.step(prepared)
            .map_err(|error| format!("step {step}: {error}"))?;
    }
    Ok(())
}

/// `run` on `source` carried onto `target` through `maps` as the app carries
/// the running field: each component's total kept where the runtime's
/// transfer keeps it, the core's handoff doing what the device's transfer
/// table does, stepped on at `time_step`.
pub fn handed(
    (source, run): (&PreparedTopology, &Run),
    target: &PreparedTopology,
    maps: &PreparedCanonicalTransfer,
    time_step: f64,
) -> Result<Run, String> {
    let runtime = CanonicalGpuRuntimeTransfer::from_primary_transfer(
        &source.canonical_operator,
        &target.canonical_operator,
        &source.canonical_forcing,
        &target.canonical_forcing,
        &maps.primary,
        [0; 4],
    )
    .map_err(|error| format!("{error:?}"))?;
    // The app's `corrects_component_totals`.
    let runtime = if target.carries_totals
        && target.mesh.geometry_revision == source.mesh.geometry_revision
    {
        runtime
    } else {
        runtime.without_total_correction()
    };
    let desired = runtime.desired_totals(&source.canonical_operator, run.primary());
    let maps = CanonicalHandoffMaps {
        primary: &maps.primary,
        complementary: &maps.complementary,
        thin_gap: &maps.thin_gap,
        outgoing: &maps.outgoing,
    };
    let forcing = &target.canonical_forcing;
    match (
        run,
        &source.canonical_temporal_operator,
        &target.canonical_temporal_operator,
    ) {
        (Run::Fixed(state), None, None) => state
            .handed_over(
                &source.canonical_operator,
                &target.canonical_operator,
                forcing,
                maps,
                &desired,
                time_step,
            )
            .map(|(state, _)| Run::Fixed(state)),
        (Run::Temporal(state), Some(from), Some(to)) => state
            .handed_over(from, to, forcing, maps, &desired, time_step)
            .map(|handed| Run::Temporal(handed.state)),
        _ => return Err("the run and the generations disagree on a drive".into()),
    }
    .map_err(|error| error.to_string())
}

/// A study's look at a carry that failed: each piece of [`handed`] applied
/// on its own, and what each answers.
pub fn diagnose_carry(
    (source, run): (&PreparedTopology, &Run),
    target: &PreparedTopology,
    maps: &PreparedCanonicalTransfer,
) -> String {
    let Run::Fixed(state) = run else {
        return "a driven run".into();
    };
    let finite = |values: &[f64]| values.iter().all(|value| value.is_finite());
    let mut report = Vec::new();
    let runtime = match CanonicalGpuRuntimeTransfer::from_primary_transfer(
        &source.canonical_operator,
        &target.canonical_operator,
        &source.canonical_forcing,
        &target.canonical_forcing,
        &maps.primary,
        [0; 4],
    ) {
        Ok(runtime) => runtime,
        Err(error) => return format!("the runtime transfer: {error:?}"),
    };
    let desired = runtime.desired_totals(&source.canonical_operator, run.primary());
    report.push(format!("desired totals {desired:?}"));
    let prescribed = vec![false; target.canonical_operator.degrees_of_freedom()];
    for (name, totals) in [
        ("with totals", desired.clone()),
        ("without", vec![None; desired.len()]),
    ] {
        match maps.primary.transfer(run.primary(), &totals, &prescribed) {
            Ok((flux, _)) => report.push(format!(
                "primary {name}: finite {}, largest {:.3e}",
                finite(&flux),
                flux.iter().fold(0.0_f64, |m, v| m.max(v.abs()))
            )),
            Err(error) => report.push(format!("primary {name}: {error}")),
        }
    }
    match maps.complementary.transfer(run.complementary()) {
        Ok((flux, _)) => report.push(format!(
            "complementary: finite {}",
            flux.iter()
                .all(|value| value.x.is_finite() && value.y.is_finite())
        )),
        Err(error) => report.push(format!("complementary: {error}")),
    }
    // The walls' memory is the state's own; what is left of the carry
    // after these is its map and the target's assembly of the fields.
    let _ = state;
    report.join("; ")
}

/// Takes away the stationary flux a handoff left on `run`, as the app does
/// once it has solved for it from the new generation's first readback. At
/// once here: the part is inert and does not change on its generation, so
/// the app's later subtraction comes to the same. Answers with the share of
/// the run's energy it took. A driven run, or one whose flux loses energy,
/// the app leaves as it is, and so does this.
pub fn cleaned(run: &mut Run, prepared: &PreparedTopology) -> Result<f64, String> {
    let operator = &prepared.canonical_operator;
    let Run::Fixed(state) = run else {
        return Ok(0.0);
    };
    if operator
        .complementary_loss_rate()
        .iter()
        .any(|rate| *rate != 0.0)
    {
        return Ok(0.0);
    }
    let mut job =
        CanonicalStationaryFluxJob::new(operator.clone(), state.complementary_flux().to_vec())
            .map_err(|error| error.to_string())?;
    let found = loop {
        if let Some(found) = job.advance(1 << 16) {
            break found.map_err(|error| error.to_string())?;
        }
    };
    // The app's floor: what the solve leaves of a flux with none.
    if found.energy <= 1.0e-9 * found.flux_energy {
        return Ok(0.0);
    }
    let energy = state.energy(operator).map_err(|error| error.to_string())?;
    let increment = found
        .stationary
        .iter()
        .map(|value| *value * -1.0)
        .collect::<Vec<_>>();
    state
        .apply_complementary_increment(operator, &increment)
        .map_err(|error| error.to_string())?;
    Ok(found.energy / energy)
}

/// Runs the runtime's preparation to its end and publishes it, with the maps
/// it prepared to carry the running field over, taken before the generation
/// is published.
pub fn finish_with_maps(
    runtime: &mut TopologyRuntime,
) -> Result<(Arc<PreparedTopology>, Arc<PreparedCanonicalTransfer>), String> {
    for _ in 0..200_000_000 {
        if let Some(outcome) = runtime.advance(64) {
            let token = outcome.map_err(|error| error.to_string())?;
            let maps = runtime
                .ready()
                .and_then(|ready| ready.canonical_transfer.clone())
                .ok_or("the preparation made no maps to carry the field over")?;
            return Ok((runtime.commit_ready(token)?, maps));
        }
    }
    Err("preparation did not finish".into())
}
