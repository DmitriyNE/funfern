//! Production f32 layout and execution path for the canonical integrated-flux
//! solver. Generation acceptance owns state, clock, physical histories and
//! live events atomically; synchronized consumers record only accepted steps.

use std::{
    borrow::Cow,
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicU32, AtomicU64, Ordering},
    },
};

use bevy::{
    asset::{embedded_asset, load_embedded_asset},
    core_pipeline::schedule::camera_driver,
    math::{UVec4, Vec4},
    prelude::*,
    render::{
        Render, RenderApp, RenderStartup, RenderSystems,
        extract_resource::{ExtractResource, ExtractResourcePlugin},
        gpu_readback::{Readback, ReadbackComplete},
        render_asset::RenderAssets,
        render_resource::{
            BindGroup, BindGroupEntries, BindGroupLayoutDescriptor, BindGroupLayoutEntries,
            CachedComputePipelineId, CachedPipelineState, ComputePassDescriptor,
            ComputePipelineDescriptor, PipelineCache, ShaderStages, ShaderType,
            binding_types::storage_buffer,
        },
        renderer::{RenderContext, RenderDevice, RenderGraph},
        storage::{GpuShaderBuffer, ShaderBuffer},
    },
};
use funfern_core::{
    CanonicalAuxiliaryState, CanonicalForcing, CanonicalOutgoingHistoryTransferMap,
    CanonicalOutgoingMidpointFactor, CanonicalOutgoingNormalizedTransfer,
    CanonicalPrimaryTransferMap, CanonicalRateDrive, CanonicalThinGapHistoryTransferMap,
    CanonicalVectorTransferMap, CanonicalWaveOperator, CanonicalWaveState,
    GRID_SCALE_FILTER_CADENCE, Point2, QuadraticWaveOperator, TimeSignal, WaveError,
};

use crate::wave_gpu::{
    AreaProbeBindGroup, CurveProbeBindGroup, FAR_FIELD_CONTOUR_POINTS, FAR_FIELD_DIRECTIONS,
    FarFieldBindGroup, ProbeBindGroup, WaveGpuRequest, WavePipeline, probe_sample_due,
};

// `ShaderBuffer::from(T)` serializes into an owned byte vector and then copies
// that vector once more through `ShaderBuffer::new`. Canonical generations can
// exceed 80 MiB, so doing that for every handoff creates a visible main-thread
// pause. `set_data` keeps the serializer's owned vector directly.
macro_rules! add_shader_buffer {
    ($assets:expr, $value:expr) => {{
        let mut buffer = ShaderBuffer::default();
        buffer.set_data($value);
        $assets.add(buffer)
    }};
}

pub const CANONICAL_GPU_LAYOUT_VERSION: u32 = 2;
pub const CANONICAL_GPU_STORAGE_BINDINGS: usize = 8;
pub const CANONICAL_GPU_WORKGROUP_SIZE: u32 = 128;
pub const CANONICAL_GPU_MAX_TRACE: usize = 1024;
const NO_INDEX: u32 = u32::MAX;
const FORCE_KIND_GAP: u32 = 1;
const MODE_WORDS: usize = 12;
const EVENT_NONE: u32 = 0;
const EVENT_PRIMARY_PULSE: u32 = 1;
const EVENT_GRID_FILTER: u32 = 2;
const EVENT_LINEAR_LAW_PATCH: u32 = 3;
const EVENT_MAINTENANCE: u32 = 4;
const EVENT_SOURCE_PATCH: u32 = 5;
const RESIDENT_FILTER_DISPATCHES: u64 = 7;
const RESIDENT_FILTER_ACCOUNTING_DISPATCHES: u64 = 1;
const TRANSFER_LAYOUT_VERSION: u32 = 1;
const TRANSFER_HEADER_WORDS: usize = 8;
const DRIVE_TARGET_PARAMETERS: u32 = 1 << 31;
const DRIVE_INDEX_MASK: u32 = !DRIVE_TARGET_PARAMETERS;

const _: () = assert!(CANONICAL_GPU_STORAGE_BINDINGS <= 8);

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CanonicalGpuClock {
    pub epoch: u64,
    pub epoch_origin_seconds: f64,
    pub step_in_epoch: u32,
    pub time_step: f64,
}

impl CanonicalGpuClock {
    pub fn initial(time_step: f64) -> Result<Self, CanonicalGpuBuildError> {
        if !time_step.is_finite() || time_step <= 0.0 {
            return Err(CanonicalGpuBuildError::InvalidClock);
        }
        Ok(Self {
            epoch: 0,
            epoch_origin_seconds: 0.0,
            step_in_epoch: 0,
            time_step,
        })
    }

    pub fn local_seconds(self) -> f64 {
        self.step_in_epoch as f64 * self.time_step
    }

    pub fn time(self) -> f64 {
        self.epoch_origin_seconds + self.local_seconds()
    }

    pub fn requires_rebase(self) -> bool {
        self.step_in_epoch >= (1 << 16) || self.local_seconds() >= 256.0
    }

    pub fn rebased(self) -> Result<Self, CanonicalGpuBuildError> {
        self.retimed(self.time_step)
    }

    pub fn retimed(self, time_step: f64) -> Result<Self, CanonicalGpuBuildError> {
        if !time_step.is_finite() || time_step <= 0.0 {
            return Err(CanonicalGpuBuildError::InvalidClock);
        }
        Ok(Self {
            epoch: self
                .epoch
                .checked_add(1)
                .ok_or(CanonicalGpuBuildError::InvalidClock)?,
            epoch_origin_seconds: self.time(),
            step_in_epoch: 0,
            time_step,
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum CanonicalGpuBuildError {
    InvalidClock,
    InvalidLayout(&'static str),
    Unrepresentable(&'static str),
    Core(WaveError),
}

impl From<WaveError> for CanonicalGpuBuildError {
    fn from(value: WaveError) -> Self {
        Self::Core(value)
    }
}

#[derive(Clone, Copy, Default, ShaderType)]
pub(crate) struct GpuCanonicalControl {
    /// node, complementary sample, auxiliary scalar, total state-word counts.
    pub counts_a: UVec4,
    /// gap, trace, mode and active-mode counts.
    pub counts_b: UVec4,
    /// force contribution, source contribution, source drive and component counts.
    pub counts_c: UVec4,
    /// gap-record, drive-record, trace-record and mode-record word offsets.
    pub table_offsets: UVec4,
    /// Trace-matrix scalar, inverse-Schur scalar, boundary-word and scratch-word offsets.
    pub boundary_offsets: UVec4,
    /// Accepted epoch low/high, step in epoch and monotonic accepted-step serial.
    pub clock_u32: UVec4,
    /// dt, accepted local seconds, candidate local seconds and maximum admitted dt.
    pub clock_f32: Vec4,
    /// Accepted and candidate epoch origins as compensated f32 high/low pairs.
    pub clock_origin: Vec4,
    /// Accepted event serial, candidate event serial, operation and flags.
    pub event: UVec4,
    /// Last processed event serial/kind, accepted flag and rejection reason.
    pub event_result: UVec4,
    /// Source, law, pulse and maintenance runtime serials.
    pub runtime_serials: UVec4,
    /// Independent accepted slots for source and future material runtime tables.
    pub runtime_slots: UVec4,
    /// Accepted source/prescribed work, primary loss and complementary loss.
    pub accepted_accounting_a: Vec4,
    /// Accepted boundary loss, filter removal, maintenance and edit exchange.
    pub accepted_accounting_b: Vec4,
    /// Candidate counterparts of `accepted_accounting_a`.
    pub candidate_accounting_a: Vec4,
    /// Candidate counterparts of `accepted_accounting_b`.
    pub candidate_accounting_b: Vec4,
    /// Filter scale, orientation, half step and quarter step.
    pub evolution: Vec4,
}

#[derive(Clone, Copy, Default, ShaderType)]
pub(crate) struct GpuCanonicalStatus {
    /// WGSL declares these four lanes as separate atomic<u32> values.
    pub words: UVec4,
    /// Handoff completion marker followed by reserved transaction lanes.
    pub transaction: UVec4,
}

#[derive(Clone, Copy, Default, ShaderType)]
pub(crate) struct GpuCanonicalStateWord {
    /// Primary/auxiliary: accepted, candidate, scratch, scratch.
    /// Complementary: accepted xy, candidate xy.
    pub values: Vec4,
}

#[derive(Clone, Copy, Default, ShaderType)]
pub(crate) struct GpuCanonicalNode {
    /// Mass, inverse mass, accepted and candidate half-stage loss fractions.
    pub mass_loss: Vec4,
    /// Absolute packed-table starts/counts for force and source contributions.
    pub ranges: UVec4,
    /// trace position, component, prescribed flag, reserved.
    pub boundary: UVec4,
    /// Prescribed harmonic offset, amplitude, angular frequency and epoch phase.
    pub prescribed: Vec4,
    /// First-order damping and geometric support; remaining lanes are reserved.
    pub damping_support: Vec4,
    /// Absolute packed-table start/count for the static compatible stiffness.
    pub stiffness: UVec4,
}

#[derive(Clone, Copy, Default, ShaderType)]
pub(crate) struct GpuCanonicalSample {
    pub nodes_a: UVec4,
    pub nodes_b: UVec4,
    pub curls_01: Vec4,
    pub curls_23: Vec4,
    pub curls_45: Vec4,
    /// Curl 6 xy, accepted and candidate half-stage loss fractions.
    pub curl_6_loss: Vec4,
    /// inverse constitutive tensor xx, xy, yy and integration weight.
    pub constitutive: Vec4,
}

#[derive(Clone, Copy, Default, ShaderType)]
pub(crate) struct GpuCanonicalTableWord {
    /// Integer metadata stays in integer lanes. Floating lanes are stored by
    /// their IEEE bits and explicitly bitcast by the shader.
    pub data: UVec4,
}

#[derive(Clone, Copy, Default, ShaderType)]
pub(crate) struct GpuCanonicalScratchWord {
    pub values: Vec4,
}

#[derive(Clone, Copy, Default, ShaderType)]
pub(crate) struct GpuCanonicalTransferWord {
    pub data: UVec4,
}

/// One target connected component's retained share of source-component
/// integrated flux. An empty row deliberately disables total correction.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CanonicalComponentTransfer {
    pub sources: Vec<(u32, f64)>,
}

/// Stable runtime correspondences prepared by the topology transaction.
/// `None` denotes a genuinely new prescribed carrier or source drive.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CanonicalGpuRuntimeTransfer {
    pub components: Vec<CanonicalComponentTransfer>,
    pub prescribed_sources: Vec<Option<u32>>,
    pub drive_sources: Vec<Option<u32>>,
    pub runtime_serials: [u32; 4],
}

impl CanonicalGpuRuntimeTransfer {
    pub fn identity(
        source: &CanonicalWaveOperator,
        target: &CanonicalWaveOperator,
        source_forcing: &CanonicalForcing,
        target_forcing: &CanonicalForcing,
    ) -> Result<Self, CanonicalGpuBuildError> {
        if source.component_count() != target.component_count()
            || source.degrees_of_freedom() != target.degrees_of_freedom()
            || source_forcing.sources().len() != target_forcing.sources().len()
        {
            return Err(CanonicalGpuBuildError::InvalidLayout(
                "identity runtime transfer requires matching stable layouts",
            ));
        }
        Ok(Self {
            components: (0..target.component_count())
                .map(|component| CanonicalComponentTransfer {
                    sources: vec![(component as u32, 1.0)],
                })
                .collect(),
            prescribed_sources: source_forcing
                .prescribed()
                .iter()
                .zip(target_forcing.prescribed())
                .enumerate()
                .map(|(node, (source, target))| {
                    (source.is_some() && target.is_some()).then_some(node as u32)
                })
                .collect(),
            drive_sources: (0..target_forcing.sources().len())
                .map(|drive| Some(drive as u32))
                .collect(),
            runtime_serials: [0; 4],
        })
    }

    /// Builds stable runtime ownership from the already prepared support map.
    /// Component shares are geometric retained-support fractions, so a split
    /// or merge does not depend on the instantaneous field being transferred.
    pub fn from_primary_transfer(
        source: &CanonicalWaveOperator,
        target: &CanonicalWaveOperator,
        source_forcing: &CanonicalForcing,
        target_forcing: &CanonicalForcing,
        primary: &CanonicalPrimaryTransferMap,
        runtime_serials: [u32; 4],
    ) -> Result<Self, CanonicalGpuBuildError> {
        let targets = primary.targets();
        if targets.len() != target.degrees_of_freedom()
            || primary.source_support().len() != source.degrees_of_freedom()
        {
            return Err(CanonicalGpuBuildError::InvalidLayout(
                "runtime ownership requires the generation's primary transfer map",
            ));
        }
        let mut source_support = vec![0.0; source.component_count()];
        for (support, component) in primary
            .source_support()
            .iter()
            .zip(source.component_labels())
        {
            source_support[*component as usize] += support;
        }
        let mut retained = vec![vec![0.0; source.component_count()]; target.component_count()];
        for row in &targets {
            let target_component = row.target_component as usize;
            for slot in 0..row.source_count as usize {
                let source_node = row.source_nodes[slot] as usize;
                let source_component = source.component_labels()[source_node] as usize;
                retained[target_component][source_component] +=
                    row.coefficients[slot] * primary.source_support()[source_node];
            }
        }
        let components = retained
            .into_iter()
            .map(|by_source| CanonicalComponentTransfer {
                sources: by_source
                    .into_iter()
                    .enumerate()
                    .filter_map(|(component, support)| {
                        let total = source_support[component];
                        (support > 1.0e-14 * total.max(1.0) && total > 0.0)
                            .then_some((component as u32, (support / total).clamp(0.0, 1.0)))
                    })
                    .collect(),
            })
            .collect();
        let prescribed_sources = targets
            .iter()
            .enumerate()
            .map(|(target_node, row)| {
                target_forcing.prescribed()[target_node]?;
                (0..row.source_count as usize)
                    .filter_map(|slot| {
                        let source_node = row.source_nodes[slot];
                        source_forcing.prescribed()[source_node as usize]
                            .is_some()
                            .then_some((source_node, row.coefficients[slot].abs()))
                    })
                    .max_by(|left, right| left.1.total_cmp(&right.1))
                    .map(|(node, _)| node)
            })
            .collect();
        let drive_sources = (0..target_forcing.sources().len())
            .map(|drive| (drive < source_forcing.sources().len()).then_some(drive as u32))
            .collect();
        Ok(Self {
            components,
            prescribed_sources,
            drive_sources,
            runtime_serials,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CanonicalGpuTransferManifest {
    pub layout_version: u32,
    pub word_count: usize,
    pub bytes: usize,
    pub dispatches: usize,
    pub exact_primary: usize,
    pub exact_complementary: usize,
    pub target_components: usize,
}

#[derive(Clone)]
pub struct CanonicalGpuTransferPlan {
    pub manifest: CanonicalGpuTransferManifest,
    source_node_count: usize,
    source_sample_count: usize,
    source_gap_count: usize,
    source_outgoing_count: usize,
    target_node_count: usize,
    target_sample_count: usize,
    target_gap_count: usize,
    target_outgoing_count: usize,
    target_component_count: usize,
    source_drive_count: usize,
    target_drive_count: usize,
    words: Vec<GpuCanonicalTransferWord>,
}

#[derive(Clone)]
pub struct CanonicalGpuLiveEvent {
    kind: u32,
    serial: u32,
    dispatches: u64,
    upload: Vec<GpuCanonicalTableWord>,
}

impl CanonicalGpuLiveEvent {
    pub fn primary_pulse(
        operator: &CanonicalWaveOperator,
        field_increment: &[f64],
        serial: u32,
    ) -> Result<Self, CanonicalGpuBuildError> {
        if field_increment.len() != operator.degrees_of_freedom() {
            return Err(CanonicalGpuBuildError::InvalidLayout(
                "a live pulse must cover every primary node",
            ));
        }
        let values = field_increment
            .iter()
            .zip(operator.primary_mass())
            .map(|(increment, mass)| finite_f32(increment * mass, "live primary pulse"))
            .collect::<Result<Vec<_>, _>>()?;
        Self::scalar_payload(EVENT_PRIMARY_PULSE, serial, 5, &values, 0)
    }

    pub fn maintenance(
        integrated_correction: &[f64],
        serial: u32,
    ) -> Result<Self, CanonicalGpuBuildError> {
        let values = integrated_correction
            .iter()
            .copied()
            .map(|value| finite_f32(value, "live maintenance correction"))
            .collect::<Result<Vec<_>, _>>()?;
        Self::scalar_payload(EVENT_MAINTENANCE, serial, 5, &values, 0)
    }

    pub fn grid_filter(strength: f64, serial: u32) -> Result<Self, CanonicalGpuBuildError> {
        if !strength.is_finite() || !(0.0..=1.0).contains(&strength) {
            return Err(CanonicalGpuBuildError::InvalidLayout(
                "grid-filter strength must be in [0, 1]",
            ));
        }
        Self::scalar_payload(
            EVENT_GRID_FILTER,
            serial,
            7,
            &[],
            finite_f32(strength, "grid-filter strength")?.to_bits(),
        )
    }

    pub fn linear_loss_patch(
        time_step: f64,
        primary_rates: &[f64],
        complementary_rates: &[f64],
        serial: u32,
    ) -> Result<Self, CanonicalGpuBuildError> {
        let values = primary_rates
            .iter()
            .chain(complementary_rates)
            .map(|rate| loss_fraction(*rate, 0.5 * time_step, "live linear loss"))
            .collect::<Result<Vec<_>, _>>()?;
        let has_loss = values.iter().any(|value| *value != 0.0);
        let complementary_zero = values[primary_rates.len()..]
            .iter()
            .all(|value| *value == 0.0);
        let event = Self::scalar_payload(
            EVENT_LINEAR_LAW_PATCH,
            serial,
            5,
            &values,
            u32::from(has_loss) | (u32::from(complementary_zero) << 2),
        )?;
        Ok(event)
    }

    pub fn source_patch(
        forcing: &CanonicalForcing,
        time_step: f64,
        serial: u32,
    ) -> Result<Self, CanonicalGpuBuildError> {
        Self::validate_serial(serial)?;
        let clock = CanonicalGpuClock::initial(time_step)?;
        let mut upload = vec![GpuCanonicalTableWord {
            data: UVec4::new(
                EVENT_SOURCE_PATCH,
                serial,
                usize_u32(forcing.sources().len())?,
                0,
            ),
        }];
        for source in forcing.sources() {
            upload.extend(gpu_drive(source.drive(), clock)?);
        }
        Ok(Self {
            kind: EVENT_SOURCE_PATCH,
            serial,
            dispatches: 5,
            upload,
        })
    }

    fn scalar_payload(
        kind: u32,
        serial: u32,
        dispatches: u64,
        values: &[f32],
        flags: u32,
    ) -> Result<Self, CanonicalGpuBuildError> {
        Self::validate_serial(serial)?;
        let mut upload = Vec::with_capacity(values.len() + 1);
        upload.push(GpuCanonicalTableWord {
            data: UVec4::new(kind, serial, usize_u32(values.len())?, flags),
        });
        upload.extend(values.iter().map(|value| GpuCanonicalTableWord {
            data: UVec4::new(value.to_bits(), 0, 0, 0),
        }));
        Ok(Self {
            kind,
            serial,
            dispatches,
            upload,
        })
    }

    fn validate_serial(serial: u32) -> Result<(), CanonicalGpuBuildError> {
        if serial == 0 {
            Err(CanonicalGpuBuildError::InvalidLayout(
                "live canonical events require a nonzero serial",
            ))
        } else {
            Ok(())
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CanonicalGpuByteReport {
    pub control: usize,
    pub status: usize,
    pub state: usize,
    pub nodes: usize,
    pub samples: usize,
    pub tables: usize,
    pub scratch: usize,
    pub boundary: usize,
}

impl CanonicalGpuByteReport {
    pub fn steady_bytes(self) -> usize {
        self.control
            + self.status
            + self.state
            + self.nodes
            + self.samples
            + self.tables
            + self.scratch
            + self.boundary
    }

    pub fn accepted_state_bytes(self, plan: &CanonicalGpuPlan) -> usize {
        plan.node_count * size_of::<f32>()
            + plan.sample_count * size_of::<[f32; 2]>()
            + plan.auxiliary_count * size_of::<f32>()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CanonicalGpuLayoutManifest {
    pub version: u32,
    pub storage_bindings: usize,
    pub state_word_stride: usize,
    pub node_stride: usize,
    pub sample_stride: usize,
    pub table_word_stride: usize,
    pub accepted_primary_offset: usize,
    pub accepted_complementary_offset: usize,
    pub accepted_auxiliary_offset: usize,
    pub candidate_primary_lane: usize,
    pub candidate_complementary_lane: usize,
    pub candidate_auxiliary_lane: usize,
    pub dispatches_per_step: usize,
    pub event_dispatches: usize,
    pub bytes: CanonicalGpuByteReport,
}

#[derive(Clone)]
pub struct CanonicalGpuPlan {
    pub manifest: CanonicalGpuLayoutManifest,
    pub node_count: usize,
    pub sample_count: usize,
    pub auxiliary_count: usize,
    pub trace_count: usize,
    pub mode_count: usize,
    needs_loss_stages: bool,
    needs_accounting: bool,
    event_kind: u32,
    time_step: f64,
    pub(crate) control: GpuCanonicalControl,
    pub(crate) status: GpuCanonicalStatus,
    pub(crate) state: Vec<GpuCanonicalStateWord>,
    pub(crate) nodes: Vec<GpuCanonicalNode>,
    pub(crate) samples: Vec<GpuCanonicalSample>,
    pub(crate) tables: Vec<GpuCanonicalTableWord>,
    pub(crate) scratch: Vec<GpuCanonicalScratchWord>,
    pub(crate) boundary: Vec<GpuCanonicalTableWord>,
}

impl CanonicalGpuPlan {
    pub fn compile(
        operator: &CanonicalWaveOperator,
        state: &CanonicalWaveState,
        forcing: &CanonicalForcing,
        clock: CanonicalGpuClock,
    ) -> Result<Self, CanonicalGpuBuildError> {
        Self::compile_inner(operator, None, state, forcing, clock)
    }

    /// Production compilation reuses the already assembled scalar CSR for the
    /// compatible stiffness cache. Reconstructing the same rows with millions
    /// of `BTreeMap` insertions accounted for most of the final handoff pause.
    pub fn compile_with_quadratic(
        operator: &CanonicalWaveOperator,
        quadratic: &QuadraticWaveOperator,
        state: &CanonicalWaveState,
        forcing: &CanonicalForcing,
        clock: CanonicalGpuClock,
    ) -> Result<Self, CanonicalGpuBuildError> {
        if quadratic.degrees_of_freedom() != operator.degrees_of_freedom()
            || quadratic.row_offsets().len() != operator.degrees_of_freedom() + 1
            || quadratic.columns().len() != quadratic.stiffness_values().len()
        {
            return Err(CanonicalGpuBuildError::InvalidLayout(
                "the scalar and canonical operators do not share a CSR layout",
            ));
        }
        Self::compile_inner(operator, Some(quadratic), state, forcing, clock)
    }

    fn compile_inner(
        operator: &CanonicalWaveOperator,
        quadratic: Option<&QuadraticWaveOperator>,
        state: &CanonicalWaveState,
        forcing: &CanonicalForcing,
        clock: CanonicalGpuClock,
    ) -> Result<Self, CanonicalGpuBuildError> {
        if clock.requires_rebase()
            || state.primary_flux().len() != operator.degrees_of_freedom()
            || state.complementary_flux().len() != operator.complementary_degrees_of_freedom()
            || forcing.prescribed().len() != operator.degrees_of_freedom()
        {
            return Err(CanonicalGpuBuildError::InvalidLayout(
                "canonical state, forcing and clock must describe one accepted generation",
            ));
        }
        let dt = finite_f32(clock.time_step, "time step")?;
        let maximum_dt = finite_f32(operator.maximum_time_step(), "maximum time step")?;
        if dt > maximum_dt {
            return Err(CanonicalGpuBuildError::InvalidClock);
        }
        let node_count = operator.degrees_of_freedom();
        let sample_count = operator.complementary_degrees_of_freedom();
        let (gap_values, outgoing_values) = match state.auxiliaries() {
            CanonicalAuxiliaryState::None => (&[][..], &[][..]),
            CanonicalAuxiliaryState::Linear(auxiliary) => {
                (auxiliary.thin_gap_jump(), auxiliary.outgoing_z())
            }
            _ => {
                return Err(CanonicalGpuBuildError::InvalidLayout(
                    "this GPU stage does not support the auxiliary variant",
                ));
            }
        };
        if gap_values.len() != operator.thin_gap_samples().len() {
            return Err(CanonicalGpuBuildError::InvalidLayout(
                "thin-gap state does not match the operator",
            ));
        }
        let auxiliary_count = gap_values.len() + outgoing_values.len();
        let initial_force = operator.force(state.complementary_flux())?;
        let mut state_words = Vec::with_capacity(node_count + sample_count + auxiliary_count);
        for (value, force) in state.primary_flux().iter().zip(initial_force) {
            let value = finite_f32(*value, "primary state")?;
            let force = finite_f32(force, "initial force cache")?;
            state_words.push(GpuCanonicalStateWord {
                values: Vec4::new(value, value, force, force),
            });
        }
        for value in state.complementary_flux() {
            let x = finite_f32(value.x, "complementary state")?;
            let y = finite_f32(value.y, "complementary state")?;
            state_words.push(GpuCanonicalStateWord {
                values: Vec4::new(x, y, x, y),
            });
        }
        for value in gap_values.iter().chain(outgoing_values) {
            let value = finite_f32(*value, "auxiliary state")?;
            state_words.push(GpuCanonicalStateWord {
                values: Vec4::new(value, value, 0.0, 0.0),
            });
        }

        let mut force_by_node = vec![Vec::<GpuCanonicalTableWord>::new(); node_count];
        let mut stiffness_by_node = quadratic
            .is_none()
            .then(|| vec![BTreeMap::<u32, f64>::new(); node_count]);
        for (sample_index, sample) in operator.constitutive_samples().iter().enumerate() {
            let tensor = sample.complementary_inverse;
            let nodes = operator.element_nodes()[sample.element as usize];
            for (node, curl) in nodes.into_iter().zip(sample.curls()) {
                let scale = operator.orientation() * sample.integration_weight;
                let x = scale * (curl.x * tensor.xx + curl.y * tensor.xy);
                let y = scale * (curl.x * tensor.xy + curl.y * tensor.yy);
                force_by_node[node as usize].push(table_word(
                    sample_index as u32,
                    0,
                    finite_f32(x, "force coefficient")?,
                    finite_f32(y, "force coefficient")?,
                ));
            }
            if let Some(stiffness_by_node) = &mut stiffness_by_node {
                for (row_node, row_curl) in nodes.into_iter().zip(sample.curls()) {
                    for (column_node, column_curl) in nodes.into_iter().zip(sample.curls()) {
                        if row_node == column_node {
                            continue;
                        }
                        let coefficient = sample.integration_weight
                            * row_curl.dot(sample.complementary_inverse.apply(*column_curl));
                        *stiffness_by_node[row_node as usize]
                            .entry(column_node)
                            .or_default() += coefficient;
                    }
                }
            }
        }
        for (gap_index, gap) in operator.thin_gap_samples().iter().enumerate() {
            let stiffness = finite_f32(gap.stiffness, "thin-gap stiffness")?;
            let auxiliary = (gap_index + node_count + sample_count) as u32;
            force_by_node[gap.left_node as usize].push(table_word(
                auxiliary,
                FORCE_KIND_GAP,
                stiffness,
                0.0,
            ));
            force_by_node[gap.right_node as usize].push(table_word(
                auxiliary,
                FORCE_KIND_GAP,
                -stiffness,
                0.0,
            ));
        }

        let mut source_by_node = vec![Vec::<GpuCanonicalTableWord>::new(); node_count];
        for (drive_index, source) in forcing.sources().iter().enumerate() {
            for (node, weight) in source.weights().iter().copied().enumerate() {
                if weight != 0.0 {
                    source_by_node[node].push(table_word(
                        drive_index as u32,
                        0,
                        finite_f32(weight, "source weight")?,
                        0.0,
                    ));
                }
            }
        }

        let mut tables = Vec::new();
        let mut force_ranges = vec![(0_u32, 0_u32); node_count];
        for (node, contributions) in force_by_node.into_iter().enumerate() {
            force_ranges[node] = (usize_u32(tables.len())?, usize_u32(contributions.len())?);
            tables.extend(contributions);
        }
        let force_count = tables.len();
        let mut source_ranges = vec![(0_u32, 0_u32); node_count];
        for (node, contributions) in source_by_node.into_iter().enumerate() {
            source_ranges[node] = (usize_u32(tables.len())?, usize_u32(contributions.len())?);
            tables.extend(contributions);
        }
        let source_count = tables.len() - force_count;
        let drive_offset = tables.len();
        for source in forcing.sources() {
            tables.extend(gpu_drive(source.drive(), clock)?);
        }
        let gap_offset = tables.len();
        for (gap_index, gap) in operator.thin_gap_samples().iter().enumerate() {
            tables.push(GpuCanonicalTableWord {
                data: UVec4::new(
                    gap.left_node,
                    gap.right_node,
                    usize_u32(node_count + sample_count + gap_index)?,
                    0,
                ),
            });
        }
        let mut stiffness_ranges = vec![(0_u32, 0_u32); node_count];
        if let Some(quadratic) = quadratic {
            let mut gap_correction = BTreeMap::<(u32, u32), f64>::new();
            for gap in operator.thin_gap_samples() {
                *gap_correction
                    .entry((gap.left_node, gap.right_node))
                    .or_default() += gap.stiffness;
                *gap_correction
                    .entry((gap.right_node, gap.left_node))
                    .or_default() += gap.stiffness;
            }
            for (node, range) in stiffness_ranges.iter_mut().enumerate() {
                let start = quadratic.row_offsets()[node] as usize;
                let end = quadratic.row_offsets()[node + 1] as usize;
                let table_start = tables.len();
                for entry in start..end {
                    let column = quadratic.columns()[entry];
                    if column as usize == node {
                        continue;
                    }
                    let coefficient = quadratic.stiffness_values()[entry]
                        + gap_correction
                            .get(&(node as u32, column))
                            .copied()
                            .unwrap_or(0.0);
                    if coefficient != 0.0 {
                        tables.push(table_word(
                            column,
                            0,
                            finite_f32(coefficient, "stiffness coefficient")?,
                            0.0,
                        ));
                    }
                }
                *range = (
                    usize_u32(table_start)?,
                    usize_u32(tables.len() - table_start)?,
                );
            }
        } else {
            for (node, row) in stiffness_by_node.unwrap().into_iter().enumerate() {
                stiffness_ranges[node] = (usize_u32(tables.len())?, usize_u32(row.len())?);
                for (column, coefficient) in row {
                    tables.push(table_word(
                        column,
                        0,
                        finite_f32(coefficient, "stiffness coefficient")?,
                        0.0,
                    ));
                }
            }
        }
        if tables.is_empty() {
            tables.push(GpuCanonicalTableWord::default());
        }

        let prescribed_trace = operator
            .outgoing_boundary()
            .map(|boundary| {
                boundary
                    .trace_nodes()
                    .iter()
                    .map(|node| forcing.prescribed()[*node as usize].is_some())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let CompiledBoundary {
            words: mut boundary,
            trace_positions,
            offsets: boundary_offsets,
            trace_count,
            mode_count,
        } = compile_boundary(
            operator,
            state.outgoing_midpoint_factor(),
            &prescribed_trace,
        )?;
        if boundary.is_empty() {
            boundary.push(GpuCanonicalTableWord::default());
        }
        if outgoing_values.len()
            != operator
                .outgoing_boundary()
                .map_or(0, |value| value.auxiliary_count())
        {
            return Err(CanonicalGpuBuildError::InvalidLayout(
                "outgoing state does not match the boundary",
            ));
        }

        let mut nodes = Vec::with_capacity(node_count);
        for node in 0..node_count {
            let mass = finite_f32(operator.primary_mass()[node], "primary mass")?;
            let primary_loss = loss_fraction(
                operator.primary_loss_rate()[node],
                0.5 * clock.time_step,
                "primary loss",
            )?;
            let damping = finite_f32(
                operator.first_order_boundary_damping()[node],
                "boundary damping",
            )?;
            let prescribed = forcing.prescribed()[node];
            nodes.push(GpuCanonicalNode {
                mass_loss: Vec4::new(mass, mass.recip(), primary_loss, primary_loss),
                ranges: UVec4::new(
                    force_ranges[node].0,
                    force_ranges[node].1,
                    source_ranges[node].0,
                    source_ranges[node].1,
                ),
                boundary: UVec4::new(
                    trace_positions[node],
                    operator.component_labels()[node],
                    u32::from(prescribed.is_some()),
                    0,
                ),
                prescribed: prescribed
                    .map(|signal| gpu_signal(signal, clock))
                    .transpose()?
                    .unwrap_or(Vec4::ZERO),
                damping_support: Vec4::new(
                    damping,
                    finite_f32(operator.geometric_support()[node], "geometric support")?,
                    0.0,
                    0.0,
                ),
                stiffness: UVec4::new(stiffness_ranges[node].0, stiffness_ranges[node].1, 0, 0),
            });
        }

        let mut samples = Vec::with_capacity(sample_count);
        for sample in operator.constitutive_samples() {
            let nodes = operator.element_nodes()[sample.element as usize];
            let curls = sample.curls();
            let loss = loss_fraction(
                operator.complementary_loss_rate()[samples.len()],
                0.5 * clock.time_step,
                "complementary loss",
            )?;
            samples.push(GpuCanonicalSample {
                nodes_a: UVec4::new(nodes[0], nodes[1], nodes[2], nodes[3]),
                nodes_b: UVec4::new(nodes[4], nodes[5], nodes[6], 0),
                curls_01: pair(curls[0], curls[1])?,
                curls_23: pair(curls[2], curls[3])?,
                curls_45: pair(curls[4], curls[5])?,
                curl_6_loss: Vec4::new(
                    finite_f32(curls[6].x, "curl")?,
                    finite_f32(curls[6].y, "curl")?,
                    loss,
                    loss,
                ),
                constitutive: Vec4::new(
                    finite_f32(sample.complementary_inverse.xx, "constitutive tensor")?,
                    finite_f32(sample.complementary_inverse.xy, "constitutive tensor")?,
                    finite_f32(sample.complementary_inverse.yy, "constitutive tensor")?,
                    finite_f32(sample.integration_weight, "integration weight")?,
                ),
            });
        }

        let total_state_words = state_words.len();
        let work_scratch_count = total_state_words + mode_count.saturating_mul(3) + trace_count;
        let accounting_item_count = node_count + sample_count + mode_count;
        let scratch_count = (work_scratch_count + 2 * accounting_item_count).max(1);
        let needs_loss_stages = operator.primary_loss_rate().iter().any(|rate| *rate != 0.0)
            || operator
                .complementary_loss_rate()
                .iter()
                .any(|rate| *rate != 0.0)
            || forcing.prescribed().iter().any(Option::is_some);
        let needs_accounting = needs_loss_stages
            || !forcing.sources().is_empty()
            || operator
                .first_order_boundary_damping()
                .iter()
                .any(|damping| *damping != 0.0)
            || trace_count != 0;
        let use_force_cache = operator
            .complementary_loss_rate()
            .iter()
            .all(|rate| *rate == 0.0);
        let dispatches_per_step = 4
            + usize::from(needs_loss_stages) * 2
            + usize::from(needs_accounting)
            + usize::from(trace_count != 0) * 8;
        let control = GpuCanonicalControl {
            counts_a: UVec4::new(
                usize_u32(node_count)?,
                usize_u32(sample_count)?,
                usize_u32(auxiliary_count)?,
                usize_u32(total_state_words)?,
            ),
            counts_b: UVec4::new(
                usize_u32(gap_values.len())?,
                usize_u32(trace_count)?,
                usize_u32(mode_count)?,
                usize_u32(outgoing_values.len() / 3)?,
            ),
            counts_c: UVec4::new(
                usize_u32(force_count)?,
                usize_u32(source_count)?,
                usize_u32(forcing.sources().len())?,
                usize_u32(operator.component_count())?,
            ),
            table_offsets: UVec4::new(
                usize_u32(gap_offset)?,
                usize_u32(drive_offset)?,
                boundary_offsets.0,
                boundary_offsets.1,
            ),
            boundary_offsets: UVec4::new(
                boundary_offsets.2,
                boundary_offsets.3,
                usize_u32(boundary.len())?,
                u32::from(needs_loss_stages)
                    | (u32::from(needs_accounting) << 1)
                    | (u32::from(use_force_cache) << 2)
                    | (u32::from(prescribed_trace.iter().any(|value| *value)) << 3),
            ),
            clock_u32: UVec4::new(
                clock.epoch as u32,
                (clock.epoch >> 32) as u32,
                clock.step_in_epoch,
                clock.step_in_epoch,
            ),
            clock_f32: Vec4::new(
                dt,
                finite_f32(clock.local_seconds(), "local clock")?,
                finite_f32(clock.local_seconds(), "local clock")?,
                maximum_dt,
            ),
            clock_origin: {
                let [high, low] = split_f64(clock.epoch_origin_seconds)?;
                Vec4::new(high, low, high, low)
            },
            event: UVec4::ZERO,
            event_result: UVec4::ZERO,
            runtime_serials: UVec4::ZERO,
            runtime_slots: UVec4::ZERO,
            accepted_accounting_a: Vec4::ZERO,
            accepted_accounting_b: Vec4::ZERO,
            candidate_accounting_a: Vec4::ZERO,
            candidate_accounting_b: Vec4::ZERO,
            evolution: Vec4::new(
                finite_f32(
                    1.0 / (4.0 / operator.maximum_time_step().powi(2)).powi(2),
                    "filter scale",
                )?,
                operator.orientation() as f32,
                0.5 * dt,
                0.25 * dt,
            ),
        };
        let bytes = CanonicalGpuByteReport {
            control: size_of::<GpuCanonicalControl>(),
            status: size_of::<GpuCanonicalStatus>(),
            state: state_words.len() * size_of::<GpuCanonicalStateWord>(),
            nodes: nodes.len() * size_of::<GpuCanonicalNode>(),
            samples: samples.len() * size_of::<GpuCanonicalSample>(),
            tables: tables.len() * size_of::<GpuCanonicalTableWord>(),
            scratch: scratch_count * size_of::<GpuCanonicalScratchWord>(),
            boundary: boundary.len() * size_of::<GpuCanonicalTableWord>(),
        };
        let manifest = CanonicalGpuLayoutManifest {
            version: CANONICAL_GPU_LAYOUT_VERSION,
            storage_bindings: CANONICAL_GPU_STORAGE_BINDINGS,
            state_word_stride: size_of::<GpuCanonicalStateWord>(),
            node_stride: size_of::<GpuCanonicalNode>(),
            sample_stride: size_of::<GpuCanonicalSample>(),
            table_word_stride: size_of::<GpuCanonicalTableWord>(),
            accepted_primary_offset: 0,
            accepted_complementary_offset: node_count,
            accepted_auxiliary_offset: node_count + sample_count,
            candidate_primary_lane: 1,
            candidate_complementary_lane: 2,
            candidate_auxiliary_lane: 1,
            dispatches_per_step,
            event_dispatches: 0,
            bytes,
        };
        Ok(Self {
            manifest,
            node_count,
            sample_count,
            auxiliary_count,
            trace_count,
            mode_count,
            needs_loss_stages,
            needs_accounting,
            event_kind: EVENT_NONE,
            time_step: clock.time_step,
            control,
            status: GpuCanonicalStatus::default(),
            state: state_words,
            nodes,
            samples,
            tables,
            scratch: vec![GpuCanonicalScratchWord::default(); scratch_count],
            boundary,
        })
    }

    /// Stages one field-valued pulse in the plan's candidate scratch. The
    /// shader applies it to free primary nodes at a zero-duration boundary.
    pub fn stage_primary_pulse(
        &mut self,
        field_increment: &[f64],
        serial: u32,
    ) -> Result<(), CanonicalGpuBuildError> {
        if field_increment.len() != self.node_count {
            return Err(CanonicalGpuBuildError::InvalidLayout(
                "a GPU pulse must cover every primary node",
            ));
        }
        let integrated = field_increment
            .iter()
            .copied()
            .enumerate()
            .map(|(node, increment)| {
                finite_f32(
                    increment * self.nodes[node].mass_loss.x as f64,
                    "primary pulse",
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        self.begin_event(EVENT_PRIMARY_PULSE, serial, 4)?;
        for (node, increment) in integrated.into_iter().enumerate() {
            self.scratch[node].values.x = increment;
        }
        Ok(())
    }

    /// Stages the paired fixed-linear grid filter. Its matrix applications are
    /// evaluated by the GPU from the accepted Q,b state.
    pub fn stage_grid_filter(
        &mut self,
        strength: f64,
        serial: u32,
    ) -> Result<(), CanonicalGpuBuildError> {
        if !strength.is_finite() || !(0.0..=1.0).contains(&strength) {
            return Err(CanonicalGpuBuildError::InvalidLayout(
                "grid-filter strength must be in [0, 1]",
            ));
        }
        self.begin_event(EVENT_GRID_FILTER, serial, 6)?;
        self.control.event.w = finite_f32(strength, "grid-filter strength")?.to_bits();
        Ok(())
    }

    /// Stages accepted/candidate fixed loss rates without mutating the live
    /// rates until global validation has completed.
    pub fn stage_linear_loss_patch(
        &mut self,
        primary_rates: &[f64],
        complementary_rates: &[f64],
        serial: u32,
    ) -> Result<(), CanonicalGpuBuildError> {
        if primary_rates.len() != self.node_count || complementary_rates.len() != self.sample_count
        {
            return Err(CanonicalGpuBuildError::InvalidLayout(
                "a linear-law patch must cover every compiled coefficient",
            ));
        }
        let primary_rates = primary_rates
            .iter()
            .copied()
            .map(|rate| {
                if rate < 0.0 {
                    return Err(CanonicalGpuBuildError::Unrepresentable("primary loss rate"));
                }
                loss_fraction(rate, 0.5 * self.time_step, "primary loss rate")
            })
            .collect::<Result<Vec<_>, _>>()?;
        let complementary_rates = complementary_rates
            .iter()
            .copied()
            .map(|rate| {
                if rate < 0.0 {
                    return Err(CanonicalGpuBuildError::Unrepresentable(
                        "complementary loss rate",
                    ));
                }
                loss_fraction(rate, 0.5 * self.time_step, "complementary loss rate")
            })
            .collect::<Result<Vec<_>, _>>()?;
        self.begin_event(EVENT_LINEAR_LAW_PATCH, serial, 4)?;
        for (node, rate) in primary_rates.iter().copied().enumerate() {
            self.nodes[node].mass_loss.w = rate;
        }
        for (sample, rate) in complementary_rates.iter().copied().enumerate() {
            self.samples[sample].curl_6_loss.w = rate;
        }
        self.needs_loss_stages = primary_rates.iter().any(|rate| *rate != 0.0)
            || complementary_rates.iter().any(|rate| *rate != 0.0)
            || self.nodes.iter().any(|node| node.boundary.z != 0);
        self.needs_accounting |= self.needs_loss_stages;
        let mut flags = self.control.boundary_offsets.w & !0b111;
        flags |= u32::from(self.needs_loss_stages);
        flags |= u32::from(self.needs_accounting) << 1;
        flags |= u32::from(complementary_rates.iter().all(|rate| *rate == 0.0)) << 2;
        self.control.boundary_offsets.w = flags;
        self.manifest.dispatches_per_step = 4
            + usize::from(self.needs_loss_stages) * 2
            + usize::from(self.needs_accounting)
            + usize::from(self.trace_count != 0) * 8;
        Ok(())
    }

    /// Stages an already bounded invariant-maintenance correction. Stage 5's
    /// live event scheduler supplies corrections from the latest-state totals.
    pub fn stage_maintenance(
        &mut self,
        integrated_primary_correction: &[f64],
        serial: u32,
    ) -> Result<(), CanonicalGpuBuildError> {
        if integrated_primary_correction.len() != self.node_count {
            return Err(CanonicalGpuBuildError::InvalidLayout(
                "maintenance correction must cover every primary node",
            ));
        }
        let corrections = integrated_primary_correction
            .iter()
            .copied()
            .map(|correction| finite_f32(correction, "maintenance correction"))
            .collect::<Result<Vec<_>, _>>()?;
        self.begin_event(EVENT_MAINTENANCE, serial, 4)?;
        for (node, correction) in corrections.into_iter().enumerate() {
            self.scratch[node].values.x = correction;
        }
        Ok(())
    }

    fn begin_event(
        &mut self,
        kind: u32,
        serial: u32,
        dispatches: usize,
    ) -> Result<(), CanonicalGpuBuildError> {
        if self.event_kind != EVENT_NONE || serial == 0 {
            return Err(CanonicalGpuBuildError::InvalidLayout(
                "one nonzero-serial event may be staged per GPU plan",
            ));
        }
        self.event_kind = kind;
        self.control.event.y = serial;
        self.control.event.z = kind << 8;
        self.manifest.event_dispatches = dispatches;
        Ok(())
    }

    pub fn stage_failure_injection(
        &mut self,
        reason: u32,
        state_word: u32,
    ) -> Result<(), CanonicalGpuBuildError> {
        if !(CANONICAL_FAILURE_LAYOUT..=CANONICAL_FAILURE_NON_FINITE).contains(&reason)
            || state_word as usize >= self.state.len()
        {
            return Err(CanonicalGpuBuildError::InvalidLayout(
                "failure injection must name a valid state word and reason",
            ));
        }
        self.status.words.z = reason;
        self.status.words.w = state_word;
        Ok(())
    }
}

impl CanonicalGpuTransferPlan {
    #[allow(clippy::too_many_arguments)]
    pub fn compile(
        source: &CanonicalWaveOperator,
        target: &CanonicalWaveOperator,
        source_forcing: &CanonicalForcing,
        target_forcing: &CanonicalForcing,
        primary: &CanonicalPrimaryTransferMap,
        complementary: &CanonicalVectorTransferMap,
        thin_gap: &CanonicalThinGapHistoryTransferMap,
        outgoing: &CanonicalOutgoingHistoryTransferMap,
        runtime: &CanonicalGpuRuntimeTransfer,
    ) -> Result<Self, CanonicalGpuBuildError> {
        let outgoing = outgoing.normalized_matrix(source, target)?;
        Self::compile_prepared(
            source,
            target,
            source_forcing,
            target_forcing,
            primary,
            complementary,
            thin_gap,
            &outgoing,
            runtime,
        )
    }

    /// Packs geometry-prepared transfer data without repeating the potentially
    /// nonlocal outgoing-basis composition on the UI thread.
    #[allow(clippy::too_many_arguments)]
    pub fn compile_prepared(
        source: &CanonicalWaveOperator,
        target: &CanonicalWaveOperator,
        source_forcing: &CanonicalForcing,
        target_forcing: &CanonicalForcing,
        primary: &CanonicalPrimaryTransferMap,
        complementary: &CanonicalVectorTransferMap,
        thin_gap: &CanonicalThinGapHistoryTransferMap,
        outgoing: &CanonicalOutgoingNormalizedTransfer,
        runtime: &CanonicalGpuRuntimeTransfer,
    ) -> Result<Self, CanonicalGpuBuildError> {
        let primary_targets = primary.targets();
        let vector_targets = complementary.targets();
        let gap_targets = thin_gap.targets();
        let source_gap_count = source.thin_gap_samples().len();
        let target_gap_count = target.thin_gap_samples().len();
        let source_outgoing_count = source
            .outgoing_boundary()
            .map_or(0, |boundary| boundary.auxiliary_count());
        let target_outgoing_count = target
            .outgoing_boundary()
            .map_or(0, |boundary| boundary.auxiliary_count());
        if primary.source_support().len() != source.degrees_of_freedom()
            || primary_targets.len() != target.degrees_of_freedom()
            || complementary.source_sample_count() != source.complementary_degrees_of_freedom()
            || vector_targets.len() != target.complementary_degrees_of_freedom()
            || gap_targets.len() != target_gap_count
            || thin_gap.source_energy_weights().len() != source_gap_count
            || outgoing.source_count != source_outgoing_count
            || outgoing.target_count != target_outgoing_count
            || runtime.components.len() != target.component_count()
            || runtime.prescribed_sources.len() != target.degrees_of_freedom()
            || runtime.drive_sources.len() != target_forcing.sources().len()
            || source_forcing.prescribed().len() != source.degrees_of_freedom()
            || target_forcing.prescribed().len() != target.degrees_of_freedom()
        {
            return Err(CanonicalGpuBuildError::InvalidLayout(
                "canonical transfer maps do not describe the source and target generations",
            ));
        }
        let mut retained = vec![0.0; source.component_count()];
        for component in &runtime.components {
            for (source_component, share) in &component.sources {
                if *source_component as usize >= source.component_count()
                    || !share.is_finite()
                    || *share < 0.0
                    || *share > 1.0
                {
                    return Err(CanonicalGpuBuildError::InvalidLayout(
                        "component retained shares must be finite and lie in [0, 1]",
                    ));
                }
                retained[*source_component as usize] += share;
            }
        }
        if retained.iter().any(|share| *share > 1.0 + 2.0e-12) {
            return Err(CanonicalGpuBuildError::InvalidLayout(
                "component split shares exceed the available source total",
            ));
        }
        for (target_node, source_node) in runtime.prescribed_sources.iter().enumerate() {
            if source_node
                .is_some_and(|source_node| source_node as usize >= source.degrees_of_freedom())
                || source_node.is_some()
                    && (source_forcing.prescribed()[source_node.unwrap() as usize].is_none()
                        || target_forcing.prescribed()[target_node].is_none())
            {
                return Err(CanonicalGpuBuildError::InvalidLayout(
                    "prescribed runtime correspondence is not a prescribed node",
                ));
            }
        }
        if runtime
            .drive_sources
            .iter()
            .flatten()
            .any(|drive| (*drive & DRIVE_INDEX_MASK) as usize >= source_forcing.sources().len())
        {
            return Err(CanonicalGpuBuildError::InvalidLayout(
                "source-drive runtime correspondence is out of range",
            ));
        }

        let mut words = vec![GpuCanonicalTransferWord::default(); TRANSFER_HEADER_WORDS];
        let primary_identity = source.degrees_of_freedom() == target.degrees_of_freedom()
            && primary_targets
                .iter()
                .enumerate()
                .all(|(index, target)| target.exact && target.source_nodes[0] == index as u32);
        let vector_identity = source.complementary_degrees_of_freedom()
            == target.complementary_degrees_of_freedom()
            && vector_targets.iter().enumerate().all(|(index, target)| {
                target.exact
                    && target.source_count == 1
                    && target.source_samples[0] == index as u32
                    && target.weights[0] == 1.0
            });
        let primary_offset = words.len();
        for target in primary_targets.iter().filter(|_| !primary_identity) {
            words.extend([
                transfer_word(
                    target.source_nodes[0],
                    target.source_nodes[1],
                    target.source_nodes[2],
                    target.source_nodes[3],
                ),
                transfer_float_word([
                    target.coefficients[0],
                    target.coefficients[1],
                    target.coefficients[2],
                    target.coefficients[3],
                ])?,
                transfer_word(
                    target.source_nodes[4],
                    target.source_nodes[5],
                    target.source_nodes[6],
                    u32::from(target.source_count)
                        | (u32::from(target.exact) << 8)
                        | (target.target_component << 9),
                ),
                transfer_float_word([
                    target.coefficients[4],
                    target.coefficients[5],
                    target.coefficients[6],
                    target.target_support,
                ])?,
            ]);
        }
        let vector_offset = words.len();
        for target in vector_targets.iter().filter(|_| !vector_identity) {
            words.extend([
                transfer_word(
                    target.source_samples[0],
                    target.source_samples[1],
                    target.source_samples[2],
                    target.source_samples[3],
                ),
                transfer_float_word([
                    target.weights[0],
                    target.weights[1],
                    target.weights[2],
                    target.weights[3],
                ])?,
                transfer_word(
                    target.source_samples[4],
                    target.source_samples[5],
                    0,
                    u32::from(target.source_count) | (u32::from(target.exact) << 8),
                ),
                transfer_float_word([target.weights[4], target.weights[5], 0.0, 0.0])?,
            ]);
        }
        let gap_offset = words.len();
        let target_gap_energy = thin_gap.target_energy_weights();
        for (target, energy_weight) in gap_targets.iter().zip(target_gap_energy) {
            if target.donors.len() > 3 {
                return Err(CanonicalGpuBuildError::InvalidLayout(
                    "thin-gap GPU history transfer supports at most three local donors",
                ));
            }
            let mut indices = [0_u32; 3];
            let mut weights = [0.0; 3];
            for (slot, (index, weight)) in target.donors.iter().enumerate() {
                if *index as usize >= source_gap_count {
                    return Err(CanonicalGpuBuildError::InvalidLayout(
                        "thin-gap history donor is out of range",
                    ));
                }
                indices[slot] = *index;
                weights[slot] = *weight;
            }
            words.extend([
                transfer_word(
                    indices[0],
                    indices[1],
                    indices[2],
                    target.donors.len() as u32 | (u32::from(target.exact) << 8),
                ),
                transfer_float_word([weights[0], weights[1], weights[2], 0.0])?,
                transfer_float_word([energy_weight, 0.0, 0.0, 0.0])?,
            ]);
        }
        let outgoing_offset = words.len();
        if outgoing.source_count != 0 && !outgoing.identity {
            for row in outgoing.values.chunks_exact(outgoing.source_count) {
                pack_transfer_scalars(&mut words, row)?;
            }
        }
        let source_inverse_support_offset = words.len();
        if !primary_identity {
            pack_transfer_scalars(
                &mut words,
                &primary
                    .source_support()
                    .iter()
                    .map(|support| support.recip())
                    .collect::<Vec<_>>(),
            )?;
        }
        let source_label_offset = words.len();
        if !primary_identity {
            for labels in source.component_labels().chunks(4) {
                let mut packed = [0_u32; 4];
                packed[..labels.len()].copy_from_slice(labels);
                words.push(transfer_word(packed[0], packed[1], packed[2], packed[3]));
            }
        }
        let component_offset = words.len();
        let component_stride = 1 + source.component_count().div_ceil(4);
        if !primary_identity {
            for target_component in &runtime.components {
                words.push(transfer_word(
                    u32::from(!target_component.sources.is_empty()),
                    0,
                    0,
                    0,
                ));
                let mut coefficients = vec![0.0; source.component_count()];
                for (source_component, share) in &target_component.sources {
                    coefficients[*source_component as usize] += *share;
                }
                pack_transfer_scalars(&mut words, &coefficients)?;
            }
        }
        let prescribed_offset = words.len();
        if target_forcing.prescribed().iter().any(Option::is_some) {
            for (chunk, mappings) in runtime.prescribed_sources.chunks(4).enumerate() {
                let mut packed = [NO_INDEX; 4];
                for (slot, mapping) in mappings.iter().enumerate() {
                    let target = chunk * 4 + slot;
                    packed[slot] = mapping.map_or(NO_INDEX, |source| {
                        let edited = source_forcing.prescribed()[source as usize]
                            != target_forcing.prescribed()[target];
                        source | (u32::from(edited) * DRIVE_TARGET_PARAMETERS)
                    });
                }
                words.push(transfer_word(packed[0], packed[1], packed[2], packed[3]));
            }
        }
        let drive_offset = words.len();
        for (chunk, mappings) in runtime.drive_sources.chunks(4).enumerate() {
            let mut packed = [NO_INDEX; 4];
            for (slot, mapping) in mappings.iter().enumerate() {
                let target = chunk * 4 + slot;
                packed[slot] = mapping.map_or(NO_INDEX, |source| {
                    let edited = source_forcing.sources()[source as usize].drive()
                        != target_forcing.sources()[target].drive();
                    source | (u32::from(edited) * DRIVE_TARGET_PARAMETERS)
                });
            }
            words.push(transfer_word(packed[0], packed[1], packed[2], packed[3]));
        }
        let source_gap_energy_offset = words.len();
        pack_transfer_scalars(&mut words, &thin_gap.source_energy_weights())?;
        let word_count = words.len();
        words[0] = transfer_word(
            TRANSFER_LAYOUT_VERSION,
            usize_u32(source.degrees_of_freedom())?,
            usize_u32(source.complementary_degrees_of_freedom())?,
            usize_u32(source_gap_count)?,
        );
        words[1] = transfer_word(
            usize_u32(source_outgoing_count)?,
            usize_u32(target.degrees_of_freedom())?,
            usize_u32(target.complementary_degrees_of_freedom())?,
            usize_u32(target_gap_count)?,
        );
        words[2] = transfer_word(
            usize_u32(target_outgoing_count)?,
            usize_u32(target.component_count())?,
            usize_u32(source.component_count())?,
            usize_u32(primary_offset)?,
        );
        words[3] = transfer_word(
            usize_u32(vector_offset)?,
            usize_u32(gap_offset)?,
            usize_u32(outgoing_offset)?,
            usize_u32(source_inverse_support_offset)?,
        );
        words[4] = transfer_word(
            usize_u32(source_label_offset)?,
            usize_u32(component_offset)?,
            usize_u32(prescribed_offset)?,
            usize_u32(drive_offset)?,
        );
        words[5] = transfer_word(
            usize_u32(source_gap_energy_offset)?,
            usize_u32(word_count)?,
            usize_u32(source_forcing.sources().len())?,
            usize_u32(target_forcing.sources().len())?,
        );
        words[6] = transfer_word(
            runtime.runtime_serials[0],
            runtime.runtime_serials[1],
            runtime.runtime_serials[2],
            runtime.runtime_serials[3],
        );
        words[7] = transfer_word(
            usize_u32(component_stride)?,
            u32::from(primary_identity),
            u32::from(vector_identity),
            u32::from(outgoing.identity),
        );
        Ok(Self {
            manifest: CanonicalGpuTransferManifest {
                layout_version: TRANSFER_LAYOUT_VERSION,
                word_count,
                bytes: word_count * size_of::<GpuCanonicalTransferWord>(),
                // Runtime, four state maps, reduction/correction, auxiliary
                // energy, two main-energy reductions, finalization and commit.
                dispatches: 14,
                exact_primary: primary.exact_nodes(),
                exact_complementary: complementary.exact_samples(),
                target_components: target.component_count(),
            },
            source_node_count: source.degrees_of_freedom(),
            source_sample_count: source.complementary_degrees_of_freedom(),
            source_gap_count,
            source_outgoing_count,
            target_node_count: target.degrees_of_freedom(),
            target_sample_count: target.complementary_degrees_of_freedom(),
            target_gap_count,
            target_outgoing_count,
            target_component_count: target.component_count(),
            source_drive_count: source_forcing.sources().len(),
            target_drive_count: target_forcing.sources().len(),
            words,
        })
    }
}

fn transfer_word(x: u32, y: u32, z: u32, w: u32) -> GpuCanonicalTransferWord {
    GpuCanonicalTransferWord {
        data: UVec4::new(x, y, z, w),
    }
}

fn transfer_float_word(
    values: [f64; 4],
) -> Result<GpuCanonicalTransferWord, CanonicalGpuBuildError> {
    Ok(transfer_word(
        finite_f32(values[0], "transfer coefficient")?.to_bits(),
        finite_f32(values[1], "transfer coefficient")?.to_bits(),
        finite_f32(values[2], "transfer coefficient")?.to_bits(),
        finite_f32(values[3], "transfer coefficient")?.to_bits(),
    ))
}

fn pack_transfer_scalars(
    words: &mut Vec<GpuCanonicalTransferWord>,
    values: &[f64],
) -> Result<(), CanonicalGpuBuildError> {
    for values in values.chunks(4) {
        let mut packed = [0.0; 4];
        packed[..values.len()].copy_from_slice(values);
        words.push(transfer_float_word(packed)?);
    }
    Ok(())
}

struct CompiledBoundary {
    words: Vec<GpuCanonicalTableWord>,
    trace_positions: Vec<u32>,
    offsets: (u32, u32, u32, u32),
    trace_count: usize,
    mode_count: usize,
}

fn compile_boundary(
    operator: &CanonicalWaveOperator,
    factor: Option<&CanonicalOutgoingMidpointFactor>,
    prescribed_trace: &[bool],
) -> Result<CompiledBoundary, CanonicalGpuBuildError> {
    let mut positions = vec![NO_INDEX; operator.degrees_of_freedom()];
    let Some(outgoing) = operator.outgoing_boundary() else {
        if factor.is_some() {
            return Err(CanonicalGpuBuildError::InvalidLayout(
                "an outgoing factor exists without a boundary",
            ));
        }
        return Ok(CompiledBoundary {
            words: Vec::new(),
            trace_positions: positions,
            offsets: (0, 0, 0, 0),
            trace_count: 0,
            mode_count: 0,
        });
    };
    let factor = factor.ok_or(CanonicalGpuBuildError::InvalidLayout(
        "the outgoing boundary has no midpoint factor",
    ))?;
    let export = factor.export_with_prescribed(prescribed_trace)?;
    let trace_count = outgoing.trace_nodes().len();
    if trace_count > CANONICAL_GPU_MAX_TRACE || export.trace_count != trace_count {
        return Err(CanonicalGpuBuildError::InvalidLayout(
            "the outgoing trace exceeds the portable workgroup contract",
        ));
    }
    let mut words = Vec::new();
    let trace_offset = words.len();
    for (position, node) in outgoing.trace_nodes().iter().copied().enumerate() {
        positions[node as usize] = position as u32;
        words.push(GpuCanonicalTableWord {
            data: UVec4::new(
                node,
                finite_f32(1.0 / operator.primary_mass()[node as usize], "trace mass")?.to_bits(),
                finite_f32(
                    operator.first_order_boundary_damping()[node as usize],
                    "trace damping",
                )?
                .to_bits(),
                0,
            ),
        });
    }
    let mode_offset = words.len();
    let elimination_by_mode = export
        .eliminated
        .iter()
        .map(|entry| (entry.mode_index, entry))
        .collect::<std::collections::BTreeMap<_, _>>();
    let energy = export.energy_transform;
    let inverse_energy = export.inverse_energy_transform;
    let residues = [6.0 / 7.0, -8.0 / 7.0, 2.0 / 7.0];
    for (mode_index, mode) in outgoing.modes().iter().enumerate() {
        let root = mode.decay.sqrt();
        let residue_z: [f64; 3] = std::array::from_fn(|column| {
            root * (0..3)
                .map(|pole| residues[pole] * inverse_energy[pole][column])
                .sum::<f64>()
        });
        let input_gain: [f64; 3] =
            std::array::from_fn(|row| root * energy[row].iter().sum::<f64>());
        let generator: [[f64; 3]; 3] = std::array::from_fn(|row| {
            std::array::from_fn(|column| {
                (0..3)
                    .map(|pole| {
                        energy[row][pole]
                            * (-mode.decay * pole as f64)
                            * inverse_energy[pole][column]
                    })
                    .sum()
            })
        });
        let elimination = elimination_by_mode.get(&mode_index).copied();
        let auxiliary = mode
            .auxiliary_offset()
            .map(|offset| offset + operator.thin_gap_samples().len())
            .map_or(NO_INDEX, |offset| offset as u32);
        words.push(GpuCanonicalTableWord {
            data: UVec4::new(
                finite_f32(mode.decay, "outgoing decay")?.to_bits(),
                auxiliary,
                elimination.is_some() as u32,
                0,
            ),
        });
        words.push(f64_triplet(residue_z)?);
        words.push(f64_triplet(input_gain)?);
        for row in generator {
            words.push(f64_triplet(row)?);
        }
        for row in 0..3 {
            words.push(f64_triplet(
                elimination.map_or([0.0; 3], |entry| entry.inverse[row]),
            )?);
        }
        words.push(f64_triplet(
            elimination.map_or([0.0; 3], |entry| entry.aqz_coefficient),
        )?);
        words.push(f64_triplet(
            elimination.map_or([0.0; 3], |entry| entry.solved_column_coefficient),
        )?);
        let b = (31.0_f64 / 7.0).sqrt();
        let ell = [0.0, 1.0 - b, 2.0 * b - 4.0];
        let loss_coefficient: [f64; 3] = std::array::from_fn(|column| {
            root * (0..3)
                .map(|pole| ell[pole] * inverse_energy[pole][column])
                .sum::<f64>()
        });
        words.push(f64_triplet(loss_coefficient)?);
    }
    debug_assert_eq!(
        words.len(),
        mode_offset + outgoing.modes().len() * MODE_WORDS
    );
    let trace_matrix_offset = words.len();
    pack_scalars(
        &mut words,
        outgoing
            .modes()
            .iter()
            .flat_map(|mode| mode.trace().iter().copied()),
        "outgoing trace matrix",
    )?;
    let inverse_offset = words.len();
    pack_scalars(
        &mut words,
        export.inverse_schur.iter().copied(),
        "outgoing inverse Schur factor",
    )?;
    pack_scalars(
        &mut words,
        (0..trace_count)
            .flat_map(|trace| outgoing.modes().iter().map(move |mode| mode.trace()[trace])),
        "transposed outgoing trace matrix",
    )?;
    Ok(CompiledBoundary {
        words,
        trace_positions: positions,
        offsets: (
            usize_u32(trace_offset)?,
            usize_u32(mode_offset)?,
            usize_u32(trace_matrix_offset)?,
            usize_u32(inverse_offset)?,
        ),
        trace_count,
        mode_count: outgoing.modes().len(),
    })
}

fn gpu_signal(
    signal: TimeSignal,
    clock: CanonicalGpuClock,
) -> Result<Vec4, CanonicalGpuBuildError> {
    let [offset, amplitude, frequency, phase] = signal.harmonic_parameters();
    let omega = std::f64::consts::TAU * frequency;
    let omega_f32 = finite_f32(omega, "signal angular frequency")?;
    if omega_f32.abs() > f32::MAX / 256.0 {
        return Err(CanonicalGpuBuildError::Unrepresentable(
            "signal angular frequency",
        ));
    }
    Ok(Vec4::new(
        finite_f32(offset, "signal offset")?,
        finite_f32(amplitude, "signal amplitude")?,
        omega_f32,
        finite_f32(
            reduced_phase(phase + omega * clock.epoch_origin_seconds),
            "signal phase anchor",
        )?,
    ))
}

fn gpu_drive(
    drive: CanonicalRateDrive,
    clock: CanonicalGpuClock,
) -> Result<[GpuCanonicalTableWord; 4], CanonicalGpuBuildError> {
    let (signal, kind, rate_anchor) = match drive {
        CanonicalRateDrive::Direct(signal) => (signal, 0_u32, 0.0),
        CanonicalRateDrive::LegacyIntegratedHarmonic { acceleration, .. } => {
            (acceleration, 1, drive.value(clock.epoch_origin_seconds)?)
        }
    };
    let parameters = gpu_signal(signal, clock)?;
    let runtime = GpuCanonicalTableWord {
        data: UVec4::new(
            kind,
            finite_f32(rate_anchor, "source rate anchor")?.to_bits(),
            0,
            0,
        ),
    };
    Ok([
        float_word(parameters),
        runtime,
        float_word(parameters),
        runtime,
    ])
}

fn pair(a: Point2, b: Point2) -> Result<Vec4, CanonicalGpuBuildError> {
    Ok(Vec4::new(
        finite_f32(a.x, "curl")?,
        finite_f32(a.y, "curl")?,
        finite_f32(b.x, "curl")?,
        finite_f32(b.y, "curl")?,
    ))
}

fn table_word(index: u32, kind: u32, x: f32, y: f32) -> GpuCanonicalTableWord {
    GpuCanonicalTableWord {
        data: UVec4::new(index, kind, x.to_bits(), y.to_bits()),
    }
}

fn float_word(values: Vec4) -> GpuCanonicalTableWord {
    GpuCanonicalTableWord {
        data: UVec4::new(
            values.x.to_bits(),
            values.y.to_bits(),
            values.z.to_bits(),
            values.w.to_bits(),
        ),
    }
}

fn f64_triplet(values: [f64; 3]) -> Result<GpuCanonicalTableWord, CanonicalGpuBuildError> {
    Ok(float_word(Vec4::new(
        finite_f32(values[0], "boundary coefficient")?,
        finite_f32(values[1], "boundary coefficient")?,
        finite_f32(values[2], "boundary coefficient")?,
        0.0,
    )))
}

fn pack_scalars(
    target: &mut Vec<GpuCanonicalTableWord>,
    values: impl IntoIterator<Item = f64>,
    name: &'static str,
) -> Result<(), CanonicalGpuBuildError> {
    let mut lanes = [0.0; 4];
    let mut lane = 0;
    for value in values {
        lanes[lane] = finite_f32(value, name)?;
        lane += 1;
        if lane == 4 {
            target.push(float_word(Vec4::from_array(lanes)));
            lanes = [0.0; 4];
            lane = 0;
        }
    }
    if lane != 0 {
        target.push(float_word(Vec4::from_array(lanes)));
    }
    Ok(())
}

fn finite_f32(value: f64, name: &'static str) -> Result<f32, CanonicalGpuBuildError> {
    let converted = value as f32;
    if value.is_finite() && converted.is_finite() {
        Ok(converted)
    } else {
        Err(CanonicalGpuBuildError::Unrepresentable(name))
    }
}

fn split_f64(value: f64) -> Result<[f32; 2], CanonicalGpuBuildError> {
    let high = finite_f32(value, "clock epoch origin")?;
    let low = finite_f32(value - high as f64, "clock epoch origin residual")?;
    Ok([high, low])
}

fn loss_fraction(
    rate: f64,
    duration: f64,
    name: &'static str,
) -> Result<f32, CanonicalGpuBuildError> {
    if !rate.is_finite() || rate < 0.0 || !duration.is_finite() || duration < 0.0 {
        return Err(CanonicalGpuBuildError::Unrepresentable(name));
    }
    finite_f32(-(-duration * rate).exp_m1(), name)
}

fn usize_u32(value: usize) -> Result<u32, CanonicalGpuBuildError> {
    value
        .try_into()
        .map_err(|_| CanonicalGpuBuildError::InvalidLayout("a GPU table exceeds u32 indexing"))
}

fn reduced_phase(value: f64) -> f64 {
    let phase = value.rem_euclid(std::f64::consts::TAU);
    if phase > std::f64::consts::PI {
        phase - std::f64::consts::TAU
    } else {
        phase
    }
}

const GPU_STATUS_READY: u32 = 1;
const GPU_STATUS_FAILED: u32 = 2;
const GPU_HANDOFF_PENDING: u32 = u32::MAX;
pub const CANONICAL_FAILURE_LAYOUT: u32 = 1;
pub const CANONICAL_FAILURE_TIMESTEP: u32 = 2;
pub const CANONICAL_FAILURE_INVERSE_DOMAIN: u32 = 3;
pub const CANONICAL_FAILURE_NON_FINITE: u32 = 4;
// Large explicit validation requests are encoded in one command buffer. The
// interactive caller still controls its much smaller per-frame request size.
const MAX_STEPS_PER_FRAME: u64 = 256;

#[derive(Default)]
pub struct CanonicalGpuStats {
    completed_steps: AtomicU64,
    local_step: AtomicU32,
    dispatches: AtomicU64,
    status: AtomicU32,
    failure: AtomicU32,
    processed_event: AtomicU32,
    event_rejection: AtomicU32,
}

impl CanonicalGpuStats {
    pub fn completed_steps(&self) -> u64 {
        self.completed_steps.load(Ordering::Relaxed)
    }

    pub fn dispatches(&self) -> u64 {
        self.dispatches.load(Ordering::Relaxed)
    }

    pub fn local_step(&self) -> u32 {
        self.local_step.load(Ordering::Relaxed)
    }

    pub fn failure(&self) -> u32 {
        self.failure.load(Ordering::Relaxed)
    }

    pub fn status(&self) -> &'static str {
        match self.status.load(Ordering::Relaxed) {
            GPU_STATUS_READY => "ready",
            GPU_STATUS_FAILED => "failed",
            _ => "loading",
        }
    }

    pub fn processed_event(&self) -> u32 {
        self.processed_event.load(Ordering::Relaxed)
    }

    pub fn event_rejection(&self) -> u32 {
        self.event_rejection.load(Ordering::Relaxed)
    }
}

#[derive(Clone)]
pub(crate) struct CanonicalGpuBufferHandles {
    pub(crate) control: Handle<ShaderBuffer>,
    pub(crate) status: Handle<ShaderBuffer>,
    pub(crate) state: Handle<ShaderBuffer>,
    pub(crate) nodes: Handle<ShaderBuffer>,
    pub(crate) samples: Handle<ShaderBuffer>,
    pub(crate) tables: Handle<ShaderBuffer>,
    pub(crate) scratch: Handle<ShaderBuffer>,
    pub(crate) boundary: Handle<ShaderBuffer>,
    pub(crate) node_count: u32,
    pub(crate) sample_count: u32,
    gap_count: u32,
    state_count: u32,
    scratch_count: u32,
    accounting_item_count: u32,
    trace_count: u32,
    drive_count: u32,
    rebase_step_limit: u32,
    dispatches_per_step: u64,
    needs_loss_stages: bool,
    needs_accounting: bool,
    event_kind: u32,
    event_serial: u32,
    event_dispatches: u64,
}

impl CanonicalGpuBufferHandles {
    fn all(&self) -> [&Handle<ShaderBuffer>; CANONICAL_GPU_STORAGE_BINDINGS] {
        [
            &self.control,
            &self.status,
            &self.state,
            &self.nodes,
            &self.samples,
            &self.tables,
            &self.scratch,
            &self.boundary,
        ]
    }
}

#[derive(Default)]
struct CanonicalGpuHandoffStats {
    completed: AtomicU32,
    failure: AtomicU32,
}

#[derive(Clone)]
struct CanonicalGpuHandoffHandles {
    target: CanonicalGpuBufferHandles,
    transfer: Handle<ShaderBuffer>,
    manifest: CanonicalGpuLayoutManifest,
    transfer_manifest: CanonicalGpuTransferManifest,
    stats: Arc<CanonicalGpuHandoffStats>,
    status_entity: Entity,
}

#[derive(Clone)]
struct CanonicalGpuLiveEventHandles {
    upload: Handle<ShaderBuffer>,
    kind: u32,
    serial: u32,
    dispatches: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CanonicalGpuHandoffOutcome {
    #[default]
    None,
    Pending,
    Accepted,
    Rejected(u32),
}

#[derive(Resource, Clone, ExtractResource)]
pub struct CanonicalGpuRequest {
    generation: u64,
    revision: u64,
    desired_steps: u64,
    buffers: Option<CanonicalGpuBufferHandles>,
    manifest: Option<CanonicalGpuLayoutManifest>,
    stats: Arc<CanonicalGpuStats>,
    readback_entities: Vec<Entity>,
    status_readback_entity: Option<Entity>,
    full_state_readback_entity: Option<Entity>,
    continuous_full_state_readback: bool,
    grid_scale_filter: bool,
    handoff: Option<CanonicalGpuHandoffHandles>,
    handoff_outcome: CanonicalGpuHandoffOutcome,
    live_event: Option<CanonicalGpuLiveEventHandles>,
}

impl Default for CanonicalGpuRequest {
    fn default() -> Self {
        Self {
            generation: 0,
            revision: 0,
            desired_steps: 0,
            buffers: None,
            manifest: None,
            stats: Arc::new(CanonicalGpuStats::default()),
            readback_entities: Vec::new(),
            status_readback_entity: None,
            full_state_readback_entity: None,
            continuous_full_state_readback: false,
            grid_scale_filter: false,
            handoff: None,
            handoff_outcome: CanonicalGpuHandoffOutcome::None,
            live_event: None,
        }
    }
}

fn spawn_canonical_state_readback(
    commands: &mut Commands,
    handles: &CanonicalGpuBufferHandles,
    generation: u64,
    full: bool,
    one_shot: bool,
) -> Entity {
    let readback = if full {
        Readback::buffer(handles.state.clone())
    } else {
        Readback::buffer_range(
            handles.state.clone(),
            0,
            u64::from(handles.node_count) * size_of::<GpuCanonicalStateWord>() as u64,
        )
    };
    commands
        .spawn((
            readback,
            CanonicalStateReadback {
                generation,
                node_count: handles.node_count,
                sample_count: handles.sample_count,
                state_count: if full {
                    handles.state_count
                } else {
                    handles.node_count
                },
                full,
                one_shot,
            },
        ))
        .id()
}

struct AddedCanonicalBuffers {
    handles: CanonicalGpuBufferHandles,
    manifest: CanonicalGpuLayoutManifest,
    initial_step: u64,
    local_step: u32,
}

fn add_canonical_buffers(
    assets: &mut Assets<ShaderBuffer>,
    plan: CanonicalGpuPlan,
) -> AddedCanonicalBuffers {
    let initial_step = plan.control.clock_u32.w as u64;
    let local_step = plan.control.clock_u32.z;
    let node_count = plan.control.counts_a.x;
    let sample_count = plan.control.counts_a.y;
    let gap_count = plan.control.counts_b.x;
    let state_count = plan.control.counts_a.w;
    let trace_count = plan.control.counts_b.y;
    let drive_count = plan.control.counts_c.z;
    let time_limit = (256.0 / plan.control.clock_f32.x as f64)
        .ceil()
        .clamp(1.0, u32::MAX as f64) as u32;
    let rebase_step_limit = time_limit.min(1 << 16);
    let scratch_count = plan.scratch.len() as u32;
    let accounting_item_count = node_count + sample_count + plan.control.counts_b.z;
    let dispatches_per_step = plan.manifest.dispatches_per_step as u64;
    let handles = CanonicalGpuBufferHandles {
        control: add_shader_buffer!(assets, plan.control),
        status: add_shader_buffer!(assets, plan.status),
        state: add_shader_buffer!(assets, plan.state),
        nodes: add_shader_buffer!(assets, plan.nodes),
        samples: add_shader_buffer!(assets, plan.samples),
        tables: add_shader_buffer!(assets, plan.tables),
        scratch: add_shader_buffer!(assets, plan.scratch),
        boundary: add_shader_buffer!(assets, plan.boundary),
        node_count,
        sample_count,
        gap_count,
        state_count,
        scratch_count,
        accounting_item_count,
        trace_count,
        drive_count,
        rebase_step_limit,
        dispatches_per_step,
        needs_loss_stages: plan.needs_loss_stages,
        needs_accounting: plan.needs_accounting,
        event_kind: plan.event_kind,
        event_serial: plan.control.event.y,
        event_dispatches: plan.manifest.event_dispatches as u64,
    };
    AddedCanonicalBuffers {
        handles,
        manifest: plan.manifest,
        initial_step,
        local_step,
    }
}

impl CanonicalGpuRequest {
    pub(crate) fn buffer_handles(&self) -> Option<&CanonicalGpuBufferHandles> {
        self.buffers.as_ref()
    }

    pub fn install(
        &mut self,
        assets: &mut Assets<ShaderBuffer>,
        commands: &mut Commands,
        plan: CanonicalGpuPlan,
    ) {
        self.clear(assets, commands);
        let generation = self.generation.wrapping_add(1).max(1);
        self.stats = Arc::new(CanonicalGpuStats::default());
        let added = add_canonical_buffers(assets, plan);
        let handles = added.handles;
        let state_entity = spawn_canonical_state_readback(
            commands,
            &handles,
            generation,
            self.continuous_full_state_readback,
            false,
        );
        let control_entity = commands
            .spawn((
                Readback::buffer(handles.control.clone()),
                CanonicalControlReadback {
                    generation,
                    stats: self.stats.clone(),
                },
            ))
            .id();
        let status_entity = commands
            .spawn((
                Readback::buffer(handles.status.clone()),
                CanonicalStatusReadback {
                    stats: self.stats.clone(),
                },
            ))
            .id();
        self.generation = generation;
        self.revision = self.revision.wrapping_add(1).max(1);
        let initial_step = added.initial_step;
        self.desired_steps = initial_step;
        self.stats
            .completed_steps
            .store(initial_step, Ordering::Relaxed);
        self.stats
            .local_step
            .store(added.local_step, Ordering::Relaxed);
        self.manifest = Some(added.manifest);
        self.buffers = Some(handles);
        self.readback_entities = vec![state_entity, control_entity, status_entity];
        self.status_readback_entity = Some(status_entity);
        self.full_state_readback_entity = None;
        self.handoff_outcome = CanonicalGpuHandoffOutcome::None;
    }

    pub fn clear(&mut self, assets: &mut Assets<ShaderBuffer>, commands: &mut Commands) {
        if let Some(handoff) = self.handoff.take() {
            for handle in handoff.target.all() {
                assets.remove(handle.id());
            }
            assets.remove(handoff.transfer.id());
            commands.entity(handoff.status_entity).despawn();
        }
        if let Some(event) = self.live_event.take() {
            assets.remove(event.upload.id());
        }
        if let Some(handles) = self.buffers.take() {
            for handle in handles.all() {
                assets.remove(handle.id());
            }
        }
        for entity in self.readback_entities.drain(..) {
            commands.entity(entity).despawn();
        }
        self.status_readback_entity = None;
        self.full_state_readback_entity = None;
        self.manifest = None;
        self.handoff_outcome = CanonicalGpuHandoffOutcome::None;
    }

    pub fn request_steps(&mut self, count: u64) {
        self.desired_steps = self.desired_steps.saturating_add(count);
    }

    pub fn requested_steps(&self) -> u64 {
        self.desired_steps
    }

    pub fn live_event_pending(&self) -> bool {
        self.live_event.is_some()
    }

    /// Enables the resident paired Q,b grid filter. Unlike authored live
    /// events, this deterministic maintenance operation is encoded directly at
    /// the fixed solver-step cadence and never changes buffer ownership or
    /// waits for a host acknowledgement.
    pub fn set_grid_scale_filter(&mut self, enabled: bool) {
        self.grid_scale_filter = enabled;
    }

    pub fn grid_scale_filter(&self) -> bool {
        self.grid_scale_filter
    }

    pub fn caught_up(&self) -> bool {
        self.stats.completed_steps() >= self.desired_steps || self.stats.failure() != 0
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn ready(&self) -> bool {
        self.buffers.is_some() && self.stats.status() == "ready" && self.stats.failure() == 0
    }

    pub fn failed(&self) -> bool {
        self.stats.failure() != 0
    }

    pub fn manifest(&self) -> Option<&CanonicalGpuLayoutManifest> {
        self.manifest.as_ref()
    }

    /// Validation harnesses need every physical lane on every readback. The
    /// interactive app leaves this disabled and continuously reads only the
    /// primary node prefix, requesting full snapshots at diagnostic cadence.
    pub fn set_continuous_full_state_readback(
        &mut self,
        enabled: bool,
    ) -> Result<(), &'static str> {
        if self.buffers.is_some() || self.handoff.is_some() {
            return Err("state readback mode must be selected before GPU installation");
        }
        self.continuous_full_state_readback = enabled;
        Ok(())
    }

    /// Queues one full physical-state snapshot without changing the continuous
    /// primary-only display stream. Returns whether a new request was queued.
    pub fn request_full_state_readback(&mut self, commands: &mut Commands) -> bool {
        if self.continuous_full_state_readback || self.full_state_readback_entity.is_some() {
            return false;
        }
        let Some(handles) = self.buffers.as_ref() else {
            return false;
        };
        let entity = spawn_canonical_state_readback(commands, handles, self.generation, true, true);
        self.full_state_readback_entity = Some(entity);
        self.readback_entities.push(entity);
        true
    }

    pub fn stats(&self) -> &Arc<CanonicalGpuStats> {
        &self.stats
    }

    pub fn handoff_outcome(&self) -> CanonicalGpuHandoffOutcome {
        self.handoff_outcome
    }

    pub fn handoff_manifest(&self) -> Option<&CanonicalGpuTransferManifest> {
        self.handoff
            .as_ref()
            .map(|handoff| &handoff.transfer_manifest)
    }

    pub fn queue_live_event(
        &mut self,
        assets: &mut Assets<ShaderBuffer>,
        event: CanonicalGpuLiveEvent,
    ) -> Result<(), &'static str> {
        if self.buffers.is_none() {
            return Err("canonical GPU is not installed");
        }
        if self.handoff.is_some() || self.live_event.is_some() {
            return Err("another canonical transaction is pending");
        }
        if self.stats.failure() != 0 || event.serial <= self.stats.processed_event() {
            return Err("live canonical event serial is stale or the solver has failed");
        }
        let handles = self.buffers.as_ref().expect("checked installed buffers");
        let payload_valid = match event.kind {
            EVENT_PRIMARY_PULSE | EVENT_MAINTENANCE => {
                event.upload.len() == handles.node_count as usize + 1
            }
            EVENT_GRID_FILTER => event.upload.len() == 1,
            EVENT_LINEAR_LAW_PATCH => {
                event.upload.len()
                    == handles.node_count as usize + handles.sample_count as usize + 1
            }
            EVENT_SOURCE_PATCH => event.upload.len() == handles.drive_count as usize * 4 + 1,
            _ => false,
        };
        if !payload_valid {
            return Err("live canonical event does not match the active generation");
        }
        if event.kind == EVENT_LINEAR_LAW_PATCH {
            let handles = self.buffers.as_mut().expect("checked installed buffers");
            // Conservative host scheduling: extra loss/accounting dispatches
            // are harmless if a later valid patch disables every rate.
            handles.needs_loss_stages = true;
            handles.needs_accounting = true;
        }
        if event.kind == EVENT_SOURCE_PATCH
            && self.buffers.as_ref().is_some_and(|handles| {
                handles.drive_count as usize != event.upload[0].data.z as usize
            })
        {
            return Err("source patch changes the compiled drive layout");
        }
        self.live_event = Some(CanonicalGpuLiveEventHandles {
            upload: add_shader_buffer!(assets, event.upload),
            kind: event.kind,
            serial: event.serial,
            dispatches: event.dispatches,
        });
        self.revision = self.revision.wrapping_add(1).max(1);
        Ok(())
    }

    /// Begins an all-or-none latest-state GPU generation handoff.
    pub fn begin_handoff(
        &mut self,
        assets: &mut Assets<ShaderBuffer>,
        commands: &mut Commands,
        mut target: CanonicalGpuPlan,
        transfer: CanonicalGpuTransferPlan,
    ) -> Result<(), &'static str> {
        if self.handoff.is_some() {
            return Err("a canonical GPU handoff is already pending");
        }
        if !self.caught_up() || self.stats.failure() != 0 {
            return Err("canonical GPU handoff requires a healthy complete-step boundary");
        }
        let source = self
            .buffers
            .as_ref()
            .ok_or("canonical GPU is not installed")?;
        if source.event_kind != EVENT_NONE && self.stats.processed_event() < source.event_serial {
            return Err("canonical GPU handoff is waiting for the staged event boundary");
        }
        if source.node_count as usize != transfer.source_node_count
            || source.sample_count as usize != transfer.source_sample_count
            || source.gap_count as usize != transfer.source_gap_count
            || source.state_count as usize
                != transfer.source_node_count
                    + transfer.source_sample_count
                    + transfer.source_gap_count
                    + transfer.source_outgoing_count
            || source.drive_count as usize != transfer.source_drive_count
            || target.node_count != transfer.target_node_count
            || target.sample_count != transfer.target_sample_count
            || target.control.counts_b.x as usize != transfer.target_gap_count
            || target.auxiliary_count != transfer.target_gap_count + transfer.target_outgoing_count
            || target.control.counts_c.w as usize != transfer.target_component_count
            || target.control.counts_c.z as usize != transfer.target_drive_count
            || target.event_kind != EVENT_NONE
        {
            return Err("canonical GPU handoff layouts do not match");
        }
        target.status.transaction.x = GPU_HANDOFF_PENDING;
        let added = add_canonical_buffers(assets, target);
        let stats = Arc::new(CanonicalGpuHandoffStats::default());
        let status_entity = commands
            .spawn((
                Readback::buffer(added.handles.status.clone()),
                CanonicalHandoffStatusReadback {
                    stats: stats.clone(),
                },
            ))
            .id();
        self.handoff = Some(CanonicalGpuHandoffHandles {
            target: added.handles,
            transfer: add_shader_buffer!(assets, transfer.words),
            manifest: added.manifest,
            transfer_manifest: transfer.manifest,
            stats,
            status_entity,
        });
        self.handoff_outcome = CanonicalGpuHandoffOutcome::Pending;
        self.revision = self.revision.wrapping_add(1).max(1);
        Ok(())
    }

    /// Installs a deterministic failure at final validation of the selected
    /// state word. This replaces only the status record and therefore leaves
    /// the accepted lanes and every immutable operator buffer untouched.
    pub fn inject_failure(
        &mut self,
        assets: &mut Assets<ShaderBuffer>,
        commands: &mut Commands,
        reason: u32,
        state_word: u32,
    ) -> Result<(), &'static str> {
        if !(CANONICAL_FAILURE_LAYOUT..=CANONICAL_FAILURE_NON_FINITE).contains(&reason) {
            return Err("invalid canonical GPU failure code");
        }
        let handles = self
            .buffers
            .as_mut()
            .ok_or("canonical GPU is not installed")?;
        if state_word >= handles.state_count {
            return Err("failure-injection index is outside the state");
        }
        self.replace_status(
            assets,
            commands,
            GpuCanonicalStatus {
                words: UVec4::new(0, 0, reason, state_word),
                transaction: UVec4::ZERO,
            },
        );
        Ok(())
    }

    /// Clears a latched runtime failure after the caller has installed a valid
    /// correction. Failed already-encoded requests are dropped; a subsequent
    /// explicit request resumes from the last accepted step.
    pub fn clear_failure(
        &mut self,
        assets: &mut Assets<ShaderBuffer>,
        commands: &mut Commands,
    ) -> Result<(), &'static str> {
        if self.buffers.is_none() {
            return Err("canonical GPU is not installed");
        }
        self.desired_steps = self.stats.completed_steps();
        self.stats.failure.store(0, Ordering::Relaxed);
        self.stats.status.store(0, Ordering::Relaxed);
        self.replace_status(assets, commands, GpuCanonicalStatus::default());
        Ok(())
    }

    fn replace_status(
        &mut self,
        assets: &mut Assets<ShaderBuffer>,
        commands: &mut Commands,
        value: GpuCanonicalStatus,
    ) {
        let replacement = add_shader_buffer!(assets, value);
        let handles = self.buffers.as_mut().expect("checked installed buffers");
        let old = std::mem::replace(&mut handles.status, replacement);
        assets.remove(old.id());
        if let Some(entity) = self.status_readback_entity.take() {
            commands.entity(entity).despawn();
            self.readback_entities
                .retain(|candidate| *candidate != entity);
        }
        let entity = commands
            .spawn((
                Readback::buffer(handles.status.clone()),
                CanonicalStatusReadback {
                    stats: self.stats.clone(),
                },
            ))
            .id();
        self.status_readback_entity = Some(entity);
        self.readback_entities.push(entity);
        self.revision = self.revision.wrapping_add(1).max(1);
    }
}

#[derive(Resource, Default)]
pub struct CanonicalGpuDisplay {
    pub generation: u64,
    pub primary_flux: Vec<f32>,
    pub previous_primary_flux: Vec<f32>,
    /// Accepted constitutive force cache. Thin-gap and boundary terms remain
    /// separately owned physical contributions and are not folded into it.
    pub constitutive_force: Vec<f32>,
    pub complementary_flux: Vec<[f32; 2]>,
    pub previous_complementary_flux: Vec<[f32; 2]>,
    pub auxiliary: Vec<f32>,
    pub previous_auxiliary: Vec<f32>,
    pub clock: Option<CanonicalGpuDisplayClock>,
    pub accounting: [f32; 8],
    pub readbacks: u64,
    pub full_readbacks: u64,
    /// State-readback serial at which the latest full snapshot arrived. Equal
    /// to `readbacks` only while the primary lanes still belong to that same
    /// physical snapshot.
    pub full_readback_at: u64,
    pub runtime_serials: [u32; 4],
    pub event_result: [u32; 4],
    raw_state: Vec<GpuCanonicalStateWord>,
    raw_primary: Vec<GpuCanonicalStateWord>,
    node_count: usize,
    sample_count: usize,
    accepted_slot: u32,
    raw_state_slot: u32,
    raw_state_continuous: bool,
}

impl CanonicalGpuDisplay {
    /// Bit-exact accepted physical state plus the accepted force cache. This is
    /// intentionally narrow and exists for transaction rollback verification.
    pub fn accepted_storage_bits(&self) -> Vec<u32> {
        let mut result =
            Vec::with_capacity(self.node_count * 2 + self.sample_count * 2 + self.auxiliary.len());
        let second = self.accepted_slot != 0;
        for word in self.raw_state.iter().take(self.node_count) {
            result.push(if second { word.values.y } else { word.values.x }.to_bits());
            result.push(if second { word.values.w } else { word.values.z }.to_bits());
        }
        for word in self
            .raw_state
            .iter()
            .skip(self.node_count)
            .take(self.sample_count)
        {
            let value = if second {
                word.values.zw()
            } else {
                word.values.xy()
            };
            result.extend([value.x.to_bits(), value.y.to_bits()]);
        }
        for word in self
            .raw_state
            .iter()
            .skip(self.node_count + self.sample_count)
        {
            result.push(if second { word.values.y } else { word.values.x }.to_bits());
        }
        result
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CanonicalGpuDisplayClock {
    pub epoch: u64,
    pub epoch_origin_seconds: f64,
    pub absolute_seconds: f64,
    pub step_in_epoch: u32,
    pub accepted_steps: u32,
    pub local_seconds: f32,
    pub time_step: f32,
    pub event_serial: u32,
}

#[derive(Component)]
struct CanonicalStateReadback {
    generation: u64,
    node_count: u32,
    sample_count: u32,
    state_count: u32,
    full: bool,
    one_shot: bool,
}

#[derive(Component)]
struct CanonicalControlReadback {
    generation: u64,
    stats: Arc<CanonicalGpuStats>,
}

#[derive(Component)]
struct CanonicalStatusReadback {
    stats: Arc<CanonicalGpuStats>,
}

#[derive(Component)]
struct CanonicalHandoffStatusReadback {
    stats: Arc<CanonicalGpuHandoffStats>,
}

fn receive_canonical_state(
    event: On<ReadbackComplete>,
    tags: Query<&CanonicalStateReadback>,
    mut commands: Commands,
    mut request: ResMut<CanonicalGpuRequest>,
    mut display: ResMut<CanonicalGpuDisplay>,
) {
    let Ok(tag) = tags.get(event.entity) else {
        return;
    };
    if tag.one_shot {
        // A readback component is continuous until its deferred despawn reaches
        // the render world. Ignore duplicate completions after the first one;
        // otherwise one requested snapshot can be counted and decoded more
        // than once, and can queue several despawns for the same entity.
        if request.full_state_readback_entity != Some(event.entity) {
            return;
        }
        request
            .readback_entities
            .retain(|candidate| *candidate != event.entity);
        request.full_state_readback_entity = None;
        commands.entity(event.entity).try_despawn();
    }
    if tag.generation != request.generation {
        return;
    }
    let words: Vec<GpuCanonicalStateWord> = event.to_shader_type();
    if words.len() != tag.state_count as usize {
        return;
    }
    begin_canonical_display_generation(&mut display, tag.generation);
    display.node_count = tag.node_count as usize;
    display.sample_count = tag.sample_count as usize;
    if tag.full {
        display.raw_state_slot = display.accepted_slot;
        display.raw_state_continuous = !tag.one_shot;
        display.raw_primary.clear();
        display
            .raw_primary
            .extend(words.iter().take(tag.node_count as usize).copied());
        display.raw_state = words;
    } else {
        display.raw_primary = words;
    }
    refresh_canonical_display(&mut display);
    display.readbacks = display.readbacks.saturating_add(1);
    if tag.full {
        display.full_readbacks = display.full_readbacks.saturating_add(1);
        display.full_readback_at = display.readbacks;
    }
}

fn begin_canonical_display_generation(display: &mut CanonicalGpuDisplay, generation: u64) {
    if display.generation == generation {
        return;
    }
    display.generation = generation;
    display.primary_flux.clear();
    display.previous_primary_flux.clear();
    display.constitutive_force.clear();
    display.complementary_flux.clear();
    display.previous_complementary_flux.clear();
    display.auxiliary.clear();
    display.previous_auxiliary.clear();
    display.clock = None;
    display.raw_primary.clear();
    display.raw_state.clear();
    display.node_count = 0;
    display.sample_count = 0;
    display.raw_state_slot = 0;
    display.raw_state_continuous = false;
    display.full_readback_at = u64::MAX;
}

fn receive_canonical_control(
    event: On<ReadbackComplete>,
    tags: Query<&CanonicalControlReadback>,
    request: Res<CanonicalGpuRequest>,
    mut display: ResMut<CanonicalGpuDisplay>,
) {
    let Ok(tag) = tags.get(event.entity) else {
        return;
    };
    if tag.generation != request.generation {
        return;
    }
    let values: Vec<GpuCanonicalControl> = event.to_shader_type();
    let Some(value) = values.first() else { return };
    let step = value.clock_u32.z;
    tag.stats
        .completed_steps
        .store(value.clock_u32.w as u64, Ordering::Relaxed);
    tag.stats.local_step.store(step, Ordering::Relaxed);
    tag.stats
        .processed_event
        .store(value.event_result.z, Ordering::Relaxed);
    tag.stats
        .event_rejection
        .store(value.event_result.w, Ordering::Relaxed);
    if tag.stats.failure() == 0 {
        tag.stats.status.store(GPU_STATUS_READY, Ordering::Relaxed);
    }
    begin_canonical_display_generation(&mut display, tag.generation);
    display.clock = Some(CanonicalGpuDisplayClock {
        epoch: value.clock_u32.x as u64 | ((value.clock_u32.y as u64) << 32),
        epoch_origin_seconds: value.clock_origin.x as f64 + value.clock_origin.y as f64,
        absolute_seconds: value.clock_origin.x as f64
            + value.clock_origin.y as f64
            + value.clock_f32.y as f64,
        step_in_epoch: step,
        accepted_steps: value.clock_u32.w,
        local_seconds: value.clock_f32.y,
        time_step: value.clock_f32.x,
        event_serial: value.event.x,
    });
    display.accounting[..4].copy_from_slice(&value.accepted_accounting_a.to_array());
    display.accounting[4..].copy_from_slice(&value.accepted_accounting_b.to_array());
    display.runtime_serials = value.runtime_serials.to_array();
    display.event_result = value.event_result.to_array();
    display.accepted_slot = value.event.z & 1;
    refresh_canonical_display(&mut display);
}

fn refresh_canonical_display(display: &mut CanonicalGpuDisplay) {
    let nodes = display.node_count;
    let samples = display.sample_count;
    if display.raw_primary.len() != nodes {
        return;
    }
    let second = display.accepted_slot != 0;
    display.primary_flux.clear();
    display.primary_flux.extend(
        display.raw_primary[..nodes]
            .iter()
            .map(|word| if second { word.values.y } else { word.values.x }),
    );
    display.previous_primary_flux.clear();
    display.previous_primary_flux.extend(
        display.raw_primary[..nodes]
            .iter()
            .map(|word| if second { word.values.x } else { word.values.y }),
    );
    display.constitutive_force.clear();
    display.constitutive_force.extend(
        display.raw_primary[..nodes]
            .iter()
            .map(|word| if second { word.values.w } else { word.values.z }),
    );
    if display.raw_state.len() < nodes + samples {
        return;
    }
    let snapshot_second = if display.raw_state_continuous {
        second
    } else {
        display.raw_state_slot != 0
    };
    display.complementary_flux.clear();
    display
        .complementary_flux
        .extend(
            display.raw_state[nodes..nodes + samples]
                .iter()
                .map(|word| {
                    if snapshot_second {
                        [word.values.z, word.values.w]
                    } else {
                        [word.values.x, word.values.y]
                    }
                }),
        );
    display.previous_complementary_flux.clear();
    display.previous_complementary_flux.extend(
        display.raw_state[nodes..nodes + samples]
            .iter()
            .map(|word| {
                if snapshot_second {
                    [word.values.x, word.values.y]
                } else {
                    [word.values.z, word.values.w]
                }
            }),
    );
    display.auxiliary.clear();
    display
        .auxiliary
        .extend(display.raw_state[nodes + samples..].iter().map(|word| {
            if snapshot_second {
                word.values.y
            } else {
                word.values.x
            }
        }));
    display.previous_auxiliary.clear();
    display
        .previous_auxiliary
        .extend(display.raw_state[nodes + samples..].iter().map(|word| {
            if snapshot_second {
                word.values.x
            } else {
                word.values.y
            }
        }));
}

fn receive_canonical_status(event: On<ReadbackComplete>, tags: Query<&CanonicalStatusReadback>) {
    let Ok(tag) = tags.get(event.entity) else {
        return;
    };
    let values: Vec<GpuCanonicalStatus> = event.to_shader_type();
    let Some(value) = values.first() else { return };
    let failure = value.words.y.max(value.words.x);
    tag.stats.failure.store(failure, Ordering::Relaxed);
    tag.stats.status.store(
        if failure == 0 {
            GPU_STATUS_READY
        } else {
            GPU_STATUS_FAILED
        },
        Ordering::Relaxed,
    );
}

fn receive_canonical_handoff_status(
    event: On<ReadbackComplete>,
    tags: Query<&CanonicalHandoffStatusReadback>,
) {
    let Ok(tag) = tags.get(event.entity) else {
        return;
    };
    let values: Vec<GpuCanonicalStatus> = event.to_shader_type();
    let Some(value) = values.first() else { return };
    if value.transaction.x == GPU_HANDOFF_PENDING {
        return;
    }
    tag.stats
        .failure
        .store(value.words.x.max(value.words.y), Ordering::Relaxed);
    tag.stats.completed.store(1, Ordering::Release);
}

fn settle_canonical_handoff(
    mut commands: Commands,
    mut assets: ResMut<Assets<ShaderBuffer>>,
    mut request: ResMut<CanonicalGpuRequest>,
) {
    let completed = request
        .handoff
        .as_ref()
        .is_some_and(|handoff| handoff.stats.completed.load(Ordering::Acquire) != 0);
    if !completed {
        return;
    }
    let handoff = request.handoff.take().expect("checked pending handoff");
    commands.entity(handoff.status_entity).despawn();
    assets.remove(handoff.transfer.id());
    let failure = handoff.stats.failure.load(Ordering::Relaxed);
    if failure != 0 {
        for handle in handoff.target.all() {
            assets.remove(handle.id());
        }
        request.handoff_outcome = CanonicalGpuHandoffOutcome::Rejected(failure);
        request.revision = request.revision.wrapping_add(1).max(1);
        return;
    }

    if let Some(old) = request.buffers.take() {
        for handle in old.all() {
            assets.remove(handle.id());
        }
    }
    for entity in request.readback_entities.drain(..) {
        commands.entity(entity).despawn();
    }
    let generation = request.generation.wrapping_add(1).max(1);
    let completed_steps = request.stats.completed_steps();
    let stats = Arc::new(CanonicalGpuStats::default());
    stats
        .completed_steps
        .store(completed_steps, Ordering::Relaxed);
    stats.status.store(GPU_STATUS_READY, Ordering::Relaxed);
    let target = handoff.target;
    let state_entity = spawn_canonical_state_readback(
        &mut commands,
        &target,
        generation,
        request.continuous_full_state_readback,
        false,
    );
    let control_entity = commands
        .spawn((
            Readback::buffer(target.control.clone()),
            CanonicalControlReadback {
                generation,
                stats: stats.clone(),
            },
        ))
        .id();
    let status_entity = commands
        .spawn((
            Readback::buffer(target.status.clone()),
            CanonicalStatusReadback {
                stats: stats.clone(),
            },
        ))
        .id();
    request.generation = generation;
    request.revision = request.revision.wrapping_add(1).max(1);
    request.desired_steps = completed_steps;
    request.buffers = Some(target);
    request.manifest = Some(handoff.manifest);
    request.stats = stats;
    request.readback_entities = vec![state_entity, control_entity, status_entity];
    request.status_readback_entity = Some(status_entity);
    request.full_state_readback_entity = None;
    request.handoff_outcome = CanonicalGpuHandoffOutcome::Accepted;
}

fn settle_canonical_live_event(
    mut assets: ResMut<Assets<ShaderBuffer>>,
    mut request: ResMut<CanonicalGpuRequest>,
) {
    let completed = request
        .live_event
        .as_ref()
        .is_some_and(|event| request.stats.processed_event() == event.serial);
    if !completed {
        return;
    }
    let event = request.live_event.take().expect("checked live event");
    assets.remove(event.upload.id());
    request.revision = request.revision.wrapping_add(1).max(1);
}

pub struct CanonicalWaveGpuPlugin;

impl Plugin for CanonicalWaveGpuPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "canonical_wave.wgsl");
        embedded_asset!(app, "canonical_transfer.wgsl");
        embedded_asset!(app, "canonical_transfer_runtime.wgsl");
        embedded_asset!(app, "canonical_transfer_energy.wgsl");
        app.init_resource::<CanonicalGpuRequest>()
            .init_resource::<CanonicalGpuDisplay>()
            .add_observer(receive_canonical_state)
            .add_observer(receive_canonical_control)
            .add_observer(receive_canonical_status)
            .add_observer(receive_canonical_handoff_status)
            .add_systems(
                Update,
                (settle_canonical_handoff, settle_canonical_live_event),
            )
            .add_plugins(ExtractResourcePlugin::<CanonicalGpuRequest>::default());
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        render_app
            .add_systems(RenderStartup, init_canonical_pipeline)
            .add_systems(
                Render,
                (
                    prepare_canonical_bind_group,
                    prepare_canonical_handoff_bind_groups,
                    prepare_canonical_live_event_bind_group,
                )
                    .in_set(RenderSystems::PrepareBindGroups),
            )
            .add_systems(
                RenderGraph,
                (
                    compute_canonical_handoff,
                    compute_canonical_wave.after(compute_canonical_handoff),
                )
                    .before(camera_driver),
            );
    }
}

#[derive(Resource)]
struct CanonicalPipeline {
    layout: BindGroupLayoutDescriptor,
    start_loss: CachedComputePipelineId,
    kick_first: CachedComputePipelineId,
    boundary_prepare_first: CachedComputePipelineId,
    boundary_reduce_first: CachedComputePipelineId,
    boundary_solve: CachedComputePipelineId,
    boundary_finalize_first: CachedComputePipelineId,
    drift: CachedComputePipelineId,
    kick_second: CachedComputePipelineId,
    boundary_prepare_second: CachedComputePipelineId,
    boundary_reduce_second: CachedComputePipelineId,
    boundary_finalize_second: CachedComputePipelineId,
    finish_loss_validate: CachedComputePipelineId,
    reduce_accounting: CachedComputePipelineId,
    stage_accounting: CachedComputePipelineId,
    commit_step: CachedComputePipelineId,
    event_begin: CachedComputePipelineId,
    event_simple_stage: CachedComputePipelineId,
    filter_first: CachedComputePipelineId,
    filter_second: CachedComputePipelineId,
    filter_finalize: CachedComputePipelineId,
    event_validate: CachedComputePipelineId,
    event_accept_tables: CachedComputePipelineId,
    commit_event: CachedComputePipelineId,
    rebase_clock_records: CachedComputePipelineId,
    commit_clock_rebase: CachedComputePipelineId,
    handoff_finalize: CachedComputePipelineId,
    handoff_reduce_prescribed: CachedComputePipelineId,
    handoff_clear_scratch: CachedComputePipelineId,
    handoff_commit: CachedComputePipelineId,
    live_event_stage: CachedComputePipelineId,
    resident_filter_begin: CachedComputePipelineId,
    resident_filter_commit: CachedComputePipelineId,
}

#[derive(Resource)]
struct CanonicalTransferPipeline {
    map_layout: BindGroupLayoutDescriptor,
    runtime_layout: BindGroupLayoutDescriptor,
    energy_layout: BindGroupLayoutDescriptor,
    transfer_runtime: CachedComputePipelineId,
    transfer_primary: CachedComputePipelineId,
    transfer_vector: CachedComputePipelineId,
    transfer_gap: CachedComputePipelineId,
    transfer_outgoing: CachedComputePipelineId,
    reduce_density: CachedComputePipelineId,
    reduce_components: CachedComputePipelineId,
    correct_primary: CachedComputePipelineId,
    reduce_auxiliary_energy: CachedComputePipelineId,
    account_handoff: CachedComputePipelineId,
}

fn init_canonical_pipeline(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    pipeline_cache: Res<PipelineCache>,
) {
    let layout = BindGroupLayoutDescriptor::new(
        "canonical wave buffers",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::COMPUTE,
            (
                storage_buffer::<GpuCanonicalControl>(false),
                storage_buffer::<GpuCanonicalStatus>(false),
                storage_buffer::<Vec<GpuCanonicalStateWord>>(false),
                storage_buffer::<Vec<GpuCanonicalNode>>(false),
                storage_buffer::<Vec<GpuCanonicalSample>>(false),
                storage_buffer::<Vec<GpuCanonicalTableWord>>(false),
                storage_buffer::<Vec<GpuCanonicalScratchWord>>(false),
                storage_buffer::<Vec<GpuCanonicalTableWord>>(false),
            ),
        ),
    );
    let shader = load_embedded_asset!(asset_server.as_ref(), "canonical_wave.wgsl");
    let queue = |entry: &'static str| {
        pipeline_cache.queue_compute_pipeline(ComputePipelineDescriptor {
            label: Some(Cow::Owned(format!("canonical wave {entry}"))),
            layout: vec![layout.clone()],
            shader: shader.clone(),
            entry_point: Some(Cow::Borrowed(entry)),
            ..default()
        })
    };
    let start_loss = queue("start_loss");
    let kick_first = queue("kick_first");
    let boundary_prepare_first = queue("boundary_prepare_first");
    let boundary_reduce_first = queue("boundary_reduce_first");
    let boundary_solve = queue("boundary_solve");
    let boundary_finalize_first = queue("boundary_finalize_first");
    let drift = queue("drift");
    let kick_second = queue("kick_second");
    let boundary_prepare_second = queue("boundary_prepare_second");
    let boundary_reduce_second = queue("boundary_reduce_second");
    let boundary_finalize_second = queue("boundary_finalize_second");
    let finish_loss_validate = queue("finish_loss_validate");
    let reduce_accounting = queue("reduce_accounting");
    let stage_accounting = queue("stage_accounting");
    let commit_step = queue("commit_step");
    let event_begin = queue("event_begin");
    let event_simple_stage = queue("event_simple_stage");
    let filter_first = queue("filter_first");
    let filter_second = queue("filter_second");
    let filter_finalize = queue("filter_finalize");
    let event_validate = queue("event_validate");
    let event_accept_tables = queue("event_accept_tables");
    let commit_event = queue("commit_event");
    let rebase_clock_records = queue("rebase_clock_records");
    let commit_clock_rebase = queue("commit_clock_rebase");
    let handoff_finalize = queue("handoff_finalize");
    let handoff_reduce_prescribed = queue("handoff_reduce_prescribed");
    let handoff_clear_scratch = queue("handoff_clear_scratch");
    let handoff_commit = queue("handoff_commit");
    let live_event_stage = queue("live_event_stage");
    let resident_filter_begin = queue("resident_filter_begin");
    let resident_filter_commit = queue("resident_filter_commit");
    commands.insert_resource(CanonicalPipeline {
        layout,
        start_loss,
        kick_first,
        boundary_prepare_first,
        boundary_reduce_first,
        boundary_solve,
        boundary_finalize_first,
        drift,
        kick_second,
        boundary_prepare_second,
        boundary_reduce_second,
        boundary_finalize_second,
        finish_loss_validate,
        reduce_accounting,
        stage_accounting,
        commit_step,
        event_begin,
        event_simple_stage,
        filter_first,
        filter_second,
        filter_finalize,
        event_validate,
        event_accept_tables,
        commit_event,
        rebase_clock_records,
        commit_clock_rebase,
        handoff_finalize,
        handoff_reduce_prescribed,
        handoff_clear_scratch,
        handoff_commit,
        live_event_stage,
        resident_filter_begin,
        resident_filter_commit,
    });

    let map_layout = BindGroupLayoutDescriptor::new(
        "canonical latest-state transfer buffers",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::COMPUTE,
            (
                storage_buffer::<GpuCanonicalControl>(false),
                storage_buffer::<Vec<GpuCanonicalStateWord>>(false),
                storage_buffer::<GpuCanonicalControl>(false),
                storage_buffer::<GpuCanonicalStatus>(false),
                storage_buffer::<Vec<GpuCanonicalStateWord>>(false),
                storage_buffer::<Vec<GpuCanonicalNode>>(false),
                storage_buffer::<Vec<GpuCanonicalTransferWord>>(false),
                storage_buffer::<Vec<GpuCanonicalScratchWord>>(false),
            ),
        ),
    );
    let runtime_layout = BindGroupLayoutDescriptor::new(
        "canonical runtime transfer buffers",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::COMPUTE,
            (
                storage_buffer::<GpuCanonicalControl>(false),
                storage_buffer::<Vec<GpuCanonicalNode>>(false),
                storage_buffer::<Vec<GpuCanonicalTableWord>>(false),
                storage_buffer::<GpuCanonicalControl>(false),
                storage_buffer::<GpuCanonicalStatus>(false),
                storage_buffer::<Vec<GpuCanonicalNode>>(false),
                storage_buffer::<Vec<GpuCanonicalTableWord>>(false),
                storage_buffer::<Vec<GpuCanonicalTransferWord>>(false),
            ),
        ),
    );
    let energy_layout = BindGroupLayoutDescriptor::new(
        "canonical handoff energy buffers",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::COMPUTE,
            (
                storage_buffer::<GpuCanonicalControl>(false),
                storage_buffer::<Vec<GpuCanonicalStateWord>>(false),
                storage_buffer::<Vec<GpuCanonicalNode>>(false),
                storage_buffer::<Vec<GpuCanonicalSample>>(false),
                storage_buffer::<GpuCanonicalControl>(false),
                storage_buffer::<Vec<GpuCanonicalStateWord>>(false),
                storage_buffer::<Vec<GpuCanonicalNode>>(false),
                storage_buffer::<Vec<GpuCanonicalSample>>(false),
            ),
        ),
    );
    let transfer_shader = load_embedded_asset!(asset_server.as_ref(), "canonical_transfer.wgsl");
    let runtime_shader =
        load_embedded_asset!(asset_server.as_ref(), "canonical_transfer_runtime.wgsl");
    let energy_shader =
        load_embedded_asset!(asset_server.as_ref(), "canonical_transfer_energy.wgsl");
    let queue_transfer = |label: &'static str,
                          entry: &'static str,
                          layout: BindGroupLayoutDescriptor,
                          shader: Handle<Shader>| {
        pipeline_cache.queue_compute_pipeline(ComputePipelineDescriptor {
            label: Some(Cow::Borrowed(label)),
            layout: vec![layout],
            shader,
            entry_point: Some(Cow::Borrowed(entry)),
            ..default()
        })
    };
    commands.insert_resource(CanonicalTransferPipeline {
        map_layout: map_layout.clone(),
        runtime_layout: runtime_layout.clone(),
        energy_layout: energy_layout.clone(),
        transfer_runtime: queue_transfer(
            "canonical transfer runtime",
            "transfer_runtime",
            runtime_layout,
            runtime_shader,
        ),
        transfer_primary: queue_transfer(
            "canonical transfer Q",
            "transfer_primary",
            map_layout.clone(),
            transfer_shader.clone(),
        ),
        transfer_vector: queue_transfer(
            "canonical transfer b",
            "transfer_vector",
            map_layout.clone(),
            transfer_shader.clone(),
        ),
        transfer_gap: queue_transfer(
            "canonical transfer gap history",
            "transfer_gap",
            map_layout.clone(),
            transfer_shader.clone(),
        ),
        transfer_outgoing: queue_transfer(
            "canonical transfer outgoing history",
            "transfer_outgoing",
            map_layout.clone(),
            transfer_shader.clone(),
        ),
        reduce_density: queue_transfer(
            "canonical transfer density reduction",
            "reduce_density",
            map_layout.clone(),
            transfer_shader.clone(),
        ),
        reduce_components: queue_transfer(
            "canonical transfer component reduction",
            "reduce_components",
            map_layout.clone(),
            transfer_shader.clone(),
        ),
        correct_primary: queue_transfer(
            "canonical transfer Q correction",
            "correct_primary",
            map_layout.clone(),
            transfer_shader.clone(),
        ),
        reduce_auxiliary_energy: queue_transfer(
            "canonical transfer auxiliary energy",
            "reduce_auxiliary_energy",
            map_layout,
            transfer_shader,
        ),
        account_handoff: queue_transfer(
            "canonical transfer energy accounting",
            "account_handoff",
            energy_layout,
            energy_shader,
        ),
    });
}

#[derive(Resource)]
struct CanonicalBindGroup {
    generation: u64,
    revision: u64,
    encoded_steps: u64,
    encoded_local_step: u32,
    encoded_event: bool,
    encoded_live_event: u32,
    bind_group: BindGroup,
}

#[derive(Resource)]
struct CanonicalHandoffBindGroups {
    revision: u64,
    encoded: bool,
    map: BindGroup,
    runtime: BindGroup,
    energy: BindGroup,
    target: BindGroup,
}

#[derive(Resource)]
struct CanonicalLiveEventBindGroup {
    revision: u64,
    bind_group: BindGroup,
}

fn prepare_canonical_bind_group(
    mut commands: Commands,
    request: Option<Res<CanonicalGpuRequest>>,
    existing: Option<Res<CanonicalBindGroup>>,
    pipeline: Res<CanonicalPipeline>,
    pipeline_cache: Res<PipelineCache>,
    render_device: Res<RenderDevice>,
    gpu_buffers: Res<RenderAssets<GpuShaderBuffer>>,
) {
    let Some(request) = request else { return };
    let Some(handles) = request.buffers.as_ref() else {
        commands.remove_resource::<CanonicalBindGroup>();
        return;
    };
    if existing.as_ref().is_some_and(|group| {
        group.generation == request.generation && group.revision == request.revision
    }) {
        return;
    }
    let buffers = handles
        .all()
        .into_iter()
        .map(|handle| gpu_buffers.get(handle))
        .collect::<Option<Vec<_>>>();
    let Some(buffers) = buffers else { return };
    let bind_group = render_device.create_bind_group(
        Some("canonical wave bind group"),
        &pipeline_cache.get_bind_group_layout(&pipeline.layout),
        &BindGroupEntries::sequential((
            buffers[0].buffer.as_entire_buffer_binding(),
            buffers[1].buffer.as_entire_buffer_binding(),
            buffers[2].buffer.as_entire_buffer_binding(),
            buffers[3].buffer.as_entire_buffer_binding(),
            buffers[4].buffer.as_entire_buffer_binding(),
            buffers[5].buffer.as_entire_buffer_binding(),
            buffers[6].buffer.as_entire_buffer_binding(),
            buffers[7].buffer.as_entire_buffer_binding(),
        )),
    );
    let (encoded_steps, encoded_local_step, encoded_event, encoded_live_event) = existing
        .as_ref()
        .filter(|group| group.generation == request.generation)
        .map_or(
            (
                request.stats.completed_steps(),
                request.stats.local_step(),
                false,
                request.stats.processed_event(),
            ),
            |group| {
                (
                    group.encoded_steps,
                    group.encoded_local_step,
                    group.encoded_event,
                    group.encoded_live_event,
                )
            },
        );
    commands.insert_resource(CanonicalBindGroup {
        generation: request.generation,
        revision: request.revision,
        encoded_steps,
        encoded_local_step,
        encoded_event,
        encoded_live_event,
        bind_group,
    });
}

#[allow(clippy::too_many_arguments)]
fn prepare_canonical_handoff_bind_groups(
    mut commands: Commands,
    request: Option<Res<CanonicalGpuRequest>>,
    existing: Option<Res<CanonicalHandoffBindGroups>>,
    canonical_pipeline: Res<CanonicalPipeline>,
    transfer_pipeline: Res<CanonicalTransferPipeline>,
    pipeline_cache: Res<PipelineCache>,
    render_device: Res<RenderDevice>,
    gpu_buffers: Res<RenderAssets<GpuShaderBuffer>>,
) {
    let Some(request) = request else { return };
    let (Some(source), Some(handoff)) = (request.buffers.as_ref(), request.handoff.as_ref()) else {
        commands.remove_resource::<CanonicalHandoffBindGroups>();
        return;
    };
    if existing
        .as_ref()
        .is_some_and(|groups| groups.revision == request.revision)
    {
        return;
    }
    let target = &handoff.target;
    let get = |handle: &Handle<ShaderBuffer>| gpu_buffers.get(handle);
    let (
        Some(old_control),
        Some(old_state),
        Some(old_nodes),
        Some(old_samples),
        Some(old_tables),
        Some(new_control),
        Some(new_status),
        Some(new_state),
        Some(new_nodes),
        Some(new_samples),
        Some(new_tables),
        Some(new_scratch),
        Some(new_boundary),
        Some(transfer),
    ) = (
        get(&source.control),
        get(&source.state),
        get(&source.nodes),
        get(&source.samples),
        get(&source.tables),
        get(&target.control),
        get(&target.status),
        get(&target.state),
        get(&target.nodes),
        get(&target.samples),
        get(&target.tables),
        get(&target.scratch),
        get(&target.boundary),
        get(&handoff.transfer),
    )
    else {
        return;
    };
    let map = render_device.create_bind_group(
        Some("canonical latest-state transfer bind group"),
        &pipeline_cache.get_bind_group_layout(&transfer_pipeline.map_layout),
        &BindGroupEntries::sequential((
            old_control.buffer.as_entire_buffer_binding(),
            old_state.buffer.as_entire_buffer_binding(),
            new_control.buffer.as_entire_buffer_binding(),
            new_status.buffer.as_entire_buffer_binding(),
            new_state.buffer.as_entire_buffer_binding(),
            new_nodes.buffer.as_entire_buffer_binding(),
            transfer.buffer.as_entire_buffer_binding(),
            new_scratch.buffer.as_entire_buffer_binding(),
        )),
    );
    let runtime = render_device.create_bind_group(
        Some("canonical runtime transfer bind group"),
        &pipeline_cache.get_bind_group_layout(&transfer_pipeline.runtime_layout),
        &BindGroupEntries::sequential((
            old_control.buffer.as_entire_buffer_binding(),
            old_nodes.buffer.as_entire_buffer_binding(),
            old_tables.buffer.as_entire_buffer_binding(),
            new_control.buffer.as_entire_buffer_binding(),
            new_status.buffer.as_entire_buffer_binding(),
            new_nodes.buffer.as_entire_buffer_binding(),
            new_tables.buffer.as_entire_buffer_binding(),
            transfer.buffer.as_entire_buffer_binding(),
        )),
    );
    let energy = render_device.create_bind_group(
        Some("canonical handoff energy bind group"),
        &pipeline_cache.get_bind_group_layout(&transfer_pipeline.energy_layout),
        &BindGroupEntries::sequential((
            old_control.buffer.as_entire_buffer_binding(),
            old_state.buffer.as_entire_buffer_binding(),
            old_nodes.buffer.as_entire_buffer_binding(),
            old_samples.buffer.as_entire_buffer_binding(),
            new_control.buffer.as_entire_buffer_binding(),
            new_state.buffer.as_entire_buffer_binding(),
            new_nodes.buffer.as_entire_buffer_binding(),
            new_samples.buffer.as_entire_buffer_binding(),
        )),
    );
    let target_group = render_device.create_bind_group(
        Some("canonical handoff target bind group"),
        &pipeline_cache.get_bind_group_layout(&canonical_pipeline.layout),
        &BindGroupEntries::sequential((
            new_control.buffer.as_entire_buffer_binding(),
            new_status.buffer.as_entire_buffer_binding(),
            new_state.buffer.as_entire_buffer_binding(),
            new_nodes.buffer.as_entire_buffer_binding(),
            new_samples.buffer.as_entire_buffer_binding(),
            new_tables.buffer.as_entire_buffer_binding(),
            new_scratch.buffer.as_entire_buffer_binding(),
            new_boundary.buffer.as_entire_buffer_binding(),
        )),
    );
    commands.insert_resource(CanonicalHandoffBindGroups {
        revision: request.revision,
        encoded: false,
        map,
        runtime,
        energy,
        target: target_group,
    });
}

fn prepare_canonical_live_event_bind_group(
    mut commands: Commands,
    request: Option<Res<CanonicalGpuRequest>>,
    existing: Option<Res<CanonicalLiveEventBindGroup>>,
    pipeline: Res<CanonicalPipeline>,
    pipeline_cache: Res<PipelineCache>,
    render_device: Res<RenderDevice>,
    gpu_buffers: Res<RenderAssets<GpuShaderBuffer>>,
) {
    let Some(request) = request else { return };
    let (Some(handles), Some(event)) = (request.buffers.as_ref(), request.live_event.as_ref())
    else {
        commands.remove_resource::<CanonicalLiveEventBindGroup>();
        return;
    };
    if existing
        .as_ref()
        .is_some_and(|group| group.revision == request.revision)
    {
        return;
    }
    let canonical = handles
        .all()
        .into_iter()
        .take(7)
        .map(|handle| gpu_buffers.get(handle))
        .collect::<Option<Vec<_>>>();
    let (Some(canonical), Some(upload)) = (canonical, gpu_buffers.get(&event.upload)) else {
        return;
    };
    let bind_group = render_device.create_bind_group(
        Some("canonical live event bind group"),
        &pipeline_cache.get_bind_group_layout(&pipeline.layout),
        &BindGroupEntries::sequential((
            canonical[0].buffer.as_entire_buffer_binding(),
            canonical[1].buffer.as_entire_buffer_binding(),
            canonical[2].buffer.as_entire_buffer_binding(),
            canonical[3].buffer.as_entire_buffer_binding(),
            canonical[4].buffer.as_entire_buffer_binding(),
            canonical[5].buffer.as_entire_buffer_binding(),
            canonical[6].buffer.as_entire_buffer_binding(),
            upload.buffer.as_entire_buffer_binding(),
        )),
    );
    commands.insert_resource(CanonicalLiveEventBindGroup {
        revision: request.revision,
        bind_group,
    });
}

#[allow(clippy::too_many_arguments)]
fn compute_canonical_wave(
    mut render_context: RenderContext,
    request: Option<Res<CanonicalGpuRequest>>,
    group: Option<ResMut<CanonicalBindGroup>>,
    live_group: Option<Res<CanonicalLiveEventBindGroup>>,
    recorders: Option<Res<WaveGpuRequest>>,
    probe_group: Option<Res<ProbeBindGroup>>,
    curve_probe_group: Option<Res<CurveProbeBindGroup>>,
    area_probe_group: Option<Res<AreaProbeBindGroup>>,
    far_field_group: Option<Res<FarFieldBindGroup>>,
    consumer_pipeline: Option<Res<WavePipeline>>,
    pipeline: Res<CanonicalPipeline>,
    pipeline_cache: Res<PipelineCache>,
) {
    let Some(request) = request else {
        return;
    };
    let Some(mut group) = group else {
        return;
    };
    let Some(handles) = request.buffers.as_ref() else {
        return;
    };
    if request.handoff.is_some() {
        return;
    }
    if group.generation != request.generation || group.revision != request.revision {
        return;
    }
    let pipeline_ids = [
        pipeline.start_loss,
        pipeline.kick_first,
        pipeline.boundary_prepare_first,
        pipeline.boundary_reduce_first,
        pipeline.boundary_solve,
        pipeline.boundary_finalize_first,
        pipeline.drift,
        pipeline.kick_second,
        pipeline.boundary_prepare_second,
        pipeline.boundary_reduce_second,
        pipeline.boundary_finalize_second,
        pipeline.finish_loss_validate,
        pipeline.reduce_accounting,
        pipeline.commit_step,
        pipeline.event_begin,
        pipeline.event_simple_stage,
        pipeline.filter_first,
        pipeline.filter_second,
        pipeline.filter_finalize,
        pipeline.event_validate,
        pipeline.event_accept_tables,
        pipeline.commit_event,
        pipeline.rebase_clock_records,
        pipeline.commit_clock_rebase,
        pipeline.stage_accounting,
        pipeline.live_event_stage,
        pipeline.resident_filter_begin,
        pipeline.resident_filter_commit,
    ];
    for id in &pipeline_ids {
        if let CachedPipelineState::Err(error) = pipeline_cache.get_compute_pipeline_state(*id) {
            // Shader assets are extracted asynchronously and Bevy represents
            // that transient state as `ShaderNotLoaded`, then queues a retry.
            // Permanent processing/module errors are logged by PipelineCache.
            trace!("canonical GPU pipeline is waiting: {error}");
            return;
        }
    }
    let pipelines = pipeline_ids
        .iter()
        .map(|id| pipeline_cache.get_compute_pipeline(*id))
        .collect::<Option<Vec<_>>>();
    let Some(pipelines) = pipelines else { return };
    let pending = request
        .desired_steps
        .saturating_sub(group.encoded_steps)
        .min(MAX_STEPS_PER_FRAME);
    let live_event = request.live_event.as_ref().filter(|event| {
        event.serial != group.encoded_live_event
            && live_group
                .as_ref()
                .is_some_and(|group| group.revision == request.revision)
    });
    let has_pending_event =
        live_event.is_some() || (handles.event_kind != EVENT_NONE && !group.encoded_event);
    if (pending == 0 && !has_pending_event) || request.stats.failure() != 0 {
        return;
    }
    let workgroups = |count: u32| count.div_ceil(CANONICAL_GPU_WORKGROUP_SIZE).max(1);
    let mut pass = render_context
        .command_encoder()
        .begin_compute_pass(&ComputePassDescriptor {
            label: Some("canonical wave evolution"),
            ..default()
        });
    pass.set_bind_group(0, &group.bind_group, &[]);
    if has_pending_event {
        let (event_kind, event_dispatches, is_live) = if let Some(event) = live_event {
            pass.set_bind_group(
                0,
                &live_group.as_ref().expect("filtered live group").bind_group,
                &[],
            );
            pass.set_pipeline(pipelines[25]);
            pass.dispatch_workgroups(
                workgroups(handles.state_count.max(handles.drive_count)),
                1,
                1,
            );
            (event.kind, event.dispatches, true)
        } else {
            (handles.event_kind, handles.event_dispatches, false)
        };
        pass.set_pipeline(pipelines[14]);
        pass.dispatch_workgroups(
            workgroups(
                handles
                    .state_count
                    .max(handles.drive_count)
                    .max(handles.accounting_item_count),
            ),
            1,
            1,
        );
        match event_kind {
            EVENT_PRIMARY_PULSE | EVENT_MAINTENANCE => {
                pass.set_pipeline(pipelines[15]);
                pass.dispatch_workgroups(workgroups(handles.state_count), 1, 1);
                pass.set_pipeline(pipelines[19]);
                pass.dispatch_workgroups(1, 1, 1);
            }
            EVENT_GRID_FILTER => {
                pass.set_pipeline(pipelines[16]);
                pass.dispatch_workgroups(workgroups(handles.node_count), 1, 1);
                pass.set_pipeline(pipelines[17]);
                pass.dispatch_workgroups(workgroups(handles.node_count), 1, 1);
                pass.set_pipeline(pipelines[18]);
                pass.dispatch_workgroups(workgroups(handles.state_count), 1, 1);
                pass.set_pipeline(pipelines[19]);
                pass.dispatch_workgroups(1, 1, 1);
            }
            EVENT_LINEAR_LAW_PATCH => {
                pass.set_pipeline(pipelines[15]);
                pass.dispatch_workgroups(workgroups(handles.state_count), 1, 1);
                pass.set_pipeline(pipelines[20]);
                pass.dispatch_workgroups(
                    workgroups(handles.node_count.max(handles.sample_count)),
                    1,
                    1,
                );
            }
            EVENT_SOURCE_PATCH => {
                pass.set_pipeline(pipelines[19]);
                pass.dispatch_workgroups(1, 1, 1);
            }
            _ => unreachable!("validated canonical GPU event kind"),
        }
        pass.set_pipeline(pipelines[21]);
        pass.dispatch_workgroups(1, 1, 1);
        if is_live {
            group.encoded_live_event = live_event.expect("selected live event").serial;
        } else {
            group.encoded_event = true;
        }
        request
            .stats
            .dispatches
            .fetch_add(event_dispatches, Ordering::Relaxed);
        pass.set_bind_group(0, &group.bind_group, &[]);
    }
    let point_recorder = recorders.as_ref().and_then(|recorders| {
        let consumer_pipeline = consumer_pipeline.as_ref()?;
        let handles = recorders.probes.as_ref()?;
        let bind_group = probe_group.as_ref()?;
        (handles.canonical
            && bind_group.generation == request.generation
            && bind_group.revision == recorders.probe_revision)
            .then(|| {
                (
                    handles,
                    bind_group,
                    pipeline_cache.get_compute_pipeline(consumer_pipeline.canonical_probe),
                )
            })
    });
    let curve_recorder = recorders.as_ref().and_then(|recorders| {
        let consumer_pipeline = consumer_pipeline.as_ref()?;
        let handles = recorders.curve_probes.as_ref()?;
        let bind_group = curve_probe_group.as_ref()?;
        (handles.canonical
            && bind_group.generation == request.generation
            && bind_group.revision == recorders.curve_probe_revision)
            .then(|| {
                (
                    handles,
                    bind_group,
                    pipeline_cache.get_compute_pipeline(consumer_pipeline.canonical_curve_probe),
                )
            })
    });
    let area_recorder = recorders.as_ref().and_then(|recorders| {
        let consumer_pipeline = consumer_pipeline.as_ref()?;
        let handles = recorders.area_probes.as_ref()?;
        let bind_group = area_probe_group.as_ref()?;
        (handles.canonical
            && bind_group.generation == request.generation
            && bind_group.revision == recorders.area_probe_revision)
            .then(|| {
                (
                    handles,
                    bind_group,
                    pipeline_cache
                        .get_compute_pipeline(consumer_pipeline.canonical_area_probe_elements),
                    pipeline_cache
                        .get_compute_pipeline(consumer_pipeline.canonical_area_probe_reduce),
                )
            })
    });
    let far_recorder = recorders.as_ref().and_then(|recorders| {
        let consumer_pipeline = consumer_pipeline.as_ref()?;
        let handles = recorders.far_field.as_ref()?;
        let bind_group = far_field_group.as_ref()?;
        (handles.canonical
            && bind_group.generation == request.generation
            && bind_group.revision == recorders.far_field_revision)
            .then(|| {
                (
                    handles,
                    bind_group,
                    pipeline_cache
                        .get_compute_pipeline(consumer_pipeline.canonical_far_field_sample),
                    pipeline_cache
                        .get_compute_pipeline(consumer_pipeline.canonical_far_field_project),
                )
            })
    });
    let mut rebases = 0_u64;
    let mut resident_filters = 0_u64;
    for offset in 0..pending {
        if group.encoded_local_step.saturating_add(1) >= handles.rebase_step_limit {
            pass.set_pipeline(pipelines[22]);
            pass.dispatch_workgroups(
                workgroups(handles.node_count.max(handles.drive_count)),
                1,
                1,
            );
            pass.set_pipeline(pipelines[23]);
            pass.dispatch_workgroups(1, 1, 1);
            group.encoded_local_step = 0;
            rebases += 1;
        }
        if handles.needs_loss_stages {
            pass.set_pipeline(pipelines[0]);
            pass.dispatch_workgroups(workgroups(handles.scratch_count), 1, 1);
        }
        pass.set_pipeline(pipelines[1]);
        pass.dispatch_workgroups(workgroups(handles.node_count), 1, 1);
        if handles.trace_count > 0 {
            pass.set_pipeline(pipelines[2]);
            pass.dispatch_workgroups(handles.trace_count, 1, 1);
            pass.set_pipeline(pipelines[3]);
            pass.dispatch_workgroups(handles.trace_count, 1, 1);
            pass.set_pipeline(pipelines[4]);
            pass.dispatch_workgroups(handles.trace_count, 1, 1);
            pass.set_pipeline(pipelines[5]);
            pass.dispatch_workgroups(handles.trace_count, 1, 1);
        }
        pass.set_pipeline(pipelines[6]);
        pass.dispatch_workgroups(
            workgroups(handles.sample_count.max(handles.gap_count)),
            1,
            1,
        );
        pass.set_pipeline(pipelines[7]);
        pass.dispatch_workgroups(workgroups(handles.node_count), 1, 1);
        if handles.trace_count > 0 {
            pass.set_pipeline(pipelines[8]);
            pass.dispatch_workgroups(handles.trace_count, 1, 1);
            pass.set_pipeline(pipelines[9]);
            pass.dispatch_workgroups(handles.trace_count, 1, 1);
            pass.set_pipeline(pipelines[4]);
            pass.dispatch_workgroups(handles.trace_count, 1, 1);
            pass.set_pipeline(pipelines[10]);
            pass.dispatch_workgroups(handles.trace_count, 1, 1);
        }
        if handles.needs_loss_stages {
            pass.set_pipeline(pipelines[11]);
            pass.dispatch_workgroups(workgroups(handles.state_count), 1, 1);
        }
        if handles.needs_accounting {
            pass.set_pipeline(pipelines[24]);
            pass.dispatch_workgroups(workgroups(handles.accounting_item_count), 1, 1);
        }
        pass.set_pipeline(pipelines[13]);
        pass.dispatch_workgroups(1, 1, 1);
        let step_after = group.encoded_steps + offset + 1;
        if request.grid_scale_filter && step_after.is_multiple_of(GRID_SCALE_FILTER_CADENCE) {
            if handles.needs_accounting {
                // Per-step contributions live in a lane-paired scratch bank.
                // Consolidate them before the zero-duration event flips the
                // accepted lane; event_begin then starts the next bank empty.
                pass.set_pipeline(pipelines[12]);
                pass.dispatch_workgroups(1, 1, 1);
            }
            pass.set_pipeline(pipelines[26]);
            pass.dispatch_workgroups(1, 1, 1);
            pass.set_pipeline(pipelines[14]);
            pass.dispatch_workgroups(
                workgroups(
                    handles
                        .state_count
                        .max(handles.drive_count)
                        .max(handles.accounting_item_count),
                ),
                1,
                1,
            );
            pass.set_pipeline(pipelines[16]);
            pass.dispatch_workgroups(workgroups(handles.node_count), 1, 1);
            pass.set_pipeline(pipelines[17]);
            pass.dispatch_workgroups(workgroups(handles.node_count), 1, 1);
            pass.set_pipeline(pipelines[18]);
            pass.dispatch_workgroups(workgroups(handles.state_count), 1, 1);
            pass.set_pipeline(pipelines[19]);
            pass.dispatch_workgroups(1, 1, 1);
            pass.set_pipeline(pipelines[27]);
            pass.dispatch_workgroups(1, 1, 1);
            resident_filters += 1;
        }
        if let Some((handles, bind_group, Some(consumer))) = point_recorder
            && probe_sample_due(step_after, handles.sample_stride)
        {
            pass.set_bind_group(0, &bind_group.bind_group, &[]);
            pass.set_pipeline(consumer);
            pass.dispatch_workgroups(1, 1, 1);
            pass.set_bind_group(0, &group.bind_group, &[]);
        }
        if let Some((handles, bind_group, Some(consumer))) = curve_recorder
            && handles
                .sample_strides
                .iter()
                .any(|stride| probe_sample_due(step_after, *stride))
        {
            pass.set_bind_group(0, &bind_group.bind_group, &[]);
            pass.set_pipeline(consumer);
            pass.dispatch_workgroups(handles.point_count.div_ceil(64), 1, 1);
            pass.set_bind_group(0, &group.bind_group, &[]);
        }
        if let Some((handles, bind_group, Some(elements), Some(reduce))) = area_recorder
            && probe_sample_due(step_after, handles.sample_stride)
        {
            pass.set_bind_group(0, &bind_group.bind_group, &[]);
            if handles.contribution_count > 0 {
                pass.set_pipeline(elements);
                pass.dispatch_workgroups(handles.contribution_count.div_ceil(64), 1, 1);
            }
            pass.set_pipeline(reduce);
            pass.dispatch_workgroups(1, 1, 1);
            pass.set_bind_group(0, &group.bind_group, &[]);
        }
        if let Some((handles, bind_group, Some(sample), Some(project))) = far_recorder
            && probe_sample_due(step_after, handles.sample_stride)
        {
            pass.set_bind_group(0, &bind_group.bind_group, &[]);
            pass.set_pipeline(sample);
            pass.dispatch_workgroups((FAR_FIELD_CONTOUR_POINTS as u32).div_ceil(64), 1, 1);
            pass.set_pipeline(project);
            pass.dispatch_workgroups((FAR_FIELD_DIRECTIONS as u32).div_ceil(64), 1, 1);
            pass.set_bind_group(0, &group.bind_group, &[]);
        }
        group.encoded_local_step += 1;
    }
    if handles.needs_accounting && pending != 0 {
        pass.set_pipeline(pipelines[12]);
        pass.dispatch_workgroups(1, 1, 1);
    }
    drop(pass);
    group.encoded_steps += pending;
    request.stats.dispatches.fetch_add(
        pending
            .saturating_mul(handles.dispatches_per_step)
            .saturating_add(2 * rebases)
            .saturating_add(RESIDENT_FILTER_DISPATCHES * resident_filters)
            .saturating_add(
                RESIDENT_FILTER_ACCOUNTING_DISPATCHES
                    * resident_filters
                    * u64::from(handles.needs_accounting),
            )
            .saturating_add(u64::from(handles.needs_accounting && pending != 0)),
        Ordering::Relaxed,
    );
}

fn compute_canonical_handoff(
    mut render_context: RenderContext,
    request: Option<Res<CanonicalGpuRequest>>,
    groups: Option<ResMut<CanonicalHandoffBindGroups>>,
    canonical: Res<CanonicalPipeline>,
    transfer: Res<CanonicalTransferPipeline>,
    pipeline_cache: Res<PipelineCache>,
) {
    let Some(request) = request else { return };
    let Some(mut groups) = groups else { return };
    let Some(handoff) = request.handoff.as_ref() else {
        return;
    };
    if groups.encoded || groups.revision != request.revision {
        return;
    }
    let ids = [
        transfer.transfer_runtime,
        transfer.transfer_primary,
        transfer.transfer_vector,
        transfer.transfer_gap,
        transfer.transfer_outgoing,
        transfer.reduce_density,
        transfer.reduce_components,
        transfer.correct_primary,
        transfer.reduce_auxiliary_energy,
        canonical.handoff_finalize,
        canonical.handoff_reduce_prescribed,
        transfer.account_handoff,
        canonical.handoff_clear_scratch,
        canonical.handoff_commit,
    ];
    if ids.iter().any(|id| {
        matches!(
            pipeline_cache.get_compute_pipeline_state(*id),
            CachedPipelineState::Err(_)
        )
    }) {
        return;
    }
    let pipelines = ids
        .iter()
        .map(|id| pipeline_cache.get_compute_pipeline(*id))
        .collect::<Option<Vec<_>>>();
    let Some(pipelines) = pipelines else { return };
    let target = &handoff.target;
    let workgroups = |count: u32| count.div_ceil(CANONICAL_GPU_WORKGROUP_SIZE).max(1);
    let mut pass = render_context
        .command_encoder()
        .begin_compute_pass(&ComputePassDescriptor {
            label: Some("canonical generation handoff"),
            ..default()
        });
    pass.set_bind_group(0, &groups.runtime, &[]);
    pass.set_pipeline(pipelines[0]);
    pass.dispatch_workgroups(workgroups(target.node_count.max(target.drive_count)), 1, 1);

    pass.set_bind_group(0, &groups.map, &[]);
    pass.set_pipeline(pipelines[1]);
    pass.dispatch_workgroups(workgroups(target.node_count), 1, 1);
    pass.set_pipeline(pipelines[2]);
    pass.dispatch_workgroups(workgroups(target.sample_count), 1, 1);
    pass.set_pipeline(pipelines[3]);
    pass.dispatch_workgroups(workgroups(target.gap_count), 1, 1);
    pass.set_pipeline(pipelines[4]);
    pass.dispatch_workgroups(
        workgroups(target.state_count - target.node_count - target.sample_count - target.gap_count),
        1,
        1,
    );
    pass.set_pipeline(pipelines[5]);
    pass.dispatch_workgroups(1, 1, 1);
    pass.set_pipeline(pipelines[6]);
    pass.dispatch_workgroups(
        handoff.transfer_manifest.target_components.max(1) as u32,
        1,
        1,
    );
    pass.set_pipeline(pipelines[7]);
    pass.dispatch_workgroups(workgroups(target.node_count), 1, 1);
    pass.set_pipeline(pipelines[8]);
    pass.dispatch_workgroups(1, 1, 1);

    pass.set_bind_group(0, &groups.target, &[]);
    pass.set_pipeline(pipelines[9]);
    pass.dispatch_workgroups(workgroups(target.state_count), 1, 1);
    pass.set_pipeline(pipelines[10]);
    pass.dispatch_workgroups(1, 1, 1);

    pass.set_bind_group(0, &groups.energy, &[]);
    pass.set_pipeline(pipelines[11]);
    pass.dispatch_workgroups(1, 1, 1);

    pass.set_bind_group(0, &groups.target, &[]);
    pass.set_pipeline(pipelines[12]);
    pass.dispatch_workgroups(workgroups(target.scratch_count), 1, 1);
    pass.set_pipeline(pipelines[13]);
    pass.dispatch_workgroups(1, 1, 1);
    drop(pass);
    groups.encoded = true;
    request
        .stats
        .dispatches
        .fetch_add(ids.len() as u64, Ordering::Relaxed);
}

#[cfg(test)]
mod tests {
    use super::*;
    use funfern_core::{
        CanonicalSource, MeshingOptions, OuterBoundaryCondition, QuadraticTransferMap,
        QuadraticWaveOperator, Scene, mesh_scene,
    };

    fn plan(boundary: OuterBoundaryCondition) -> CanonicalGpuPlan {
        let scene = Scene::initial();
        let mesh = mesh_scene(
            &scene,
            1,
            MeshingOptions {
                target_edge_length: 0.24,
                ..MeshingOptions::default()
            },
        )
        .unwrap();
        let scalar = QuadraticWaveOperator::assemble_scene(&mesh, &scene, boundary).unwrap();
        let operator = CanonicalWaveOperator::compile_scene(&mesh, &scalar, &scene, 7).unwrap();
        let dt = 0.8 * operator.maximum_time_step();
        let state = CanonicalWaveState::zero(&operator, dt).unwrap();
        let mut forcing = CanonicalForcing::none(&operator);
        forcing
            .push_source(
                CanonicalSource::direct(
                    &operator,
                    operator.primary_mass().to_vec(),
                    TimeSignal::harmonic(0.1, 0.2, 1.5, 0.3),
                )
                .unwrap(),
            )
            .unwrap();
        CanonicalGpuPlan::compile(
            &operator,
            &state,
            &forcing,
            CanonicalGpuClock::initial(dt).unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn manifest_uses_portable_bindings_and_separate_candidate_lanes() {
        let plan = plan(OuterBoundaryCondition::Reflecting);
        assert_eq!(plan.manifest.version, CANONICAL_GPU_LAYOUT_VERSION);
        assert_eq!(plan.manifest.storage_bindings, 8);
        assert_eq!(plan.manifest.state_word_stride, 16);
        assert_eq!(plan.manifest.candidate_primary_lane, 1);
        assert_eq!(plan.manifest.candidate_complementary_lane, 2);
        assert_eq!(plan.manifest.candidate_auxiliary_lane, 1);
        assert_eq!(plan.state.len(), plan.control.counts_a.w as usize);
        assert!(plan.manifest.bytes.steady_bytes() > plan.manifest.bytes.state);
    }

    #[test]
    fn rust_and_wgsl_layout_manifests_match_exactly() {
        let shader = include_str!("canonical_wave.wgsl");
        for declaration in [
            "const LAYOUT_VERSION: u32 = 2u;",
            "const STATE_WORD_STRIDE: u32 = 16u;",
            "const NODE_STRIDE: u32 = 96u;",
            "const SAMPLE_STRIDE: u32 = 112u;",
            "const TABLE_WORD_STRIDE: u32 = 16u;",
            "const MAX_TRACE: u32 = 1024u;",
        ] {
            assert!(shader.contains(declaration), "missing {declaration}");
        }
        assert_eq!(GpuCanonicalStateWord::min_size().get() as usize, 16);
        assert_eq!(GpuCanonicalNode::min_size().get() as usize, 96);
        assert_eq!(GpuCanonicalSample::min_size().get() as usize, 112);
        assert_eq!(GpuCanonicalTableWord::min_size().get() as usize, 16);
        assert_eq!(size_of::<GpuCanonicalControl>(), 272);
        assert_eq!(GpuCanonicalControl::min_size().get() as usize, 272);
    }

    #[test]
    fn second_order_manifest_contains_the_parallel_trace_factor() {
        let plan = plan(OuterBoundaryCondition::SecondOrderOutgoing);
        assert!(plan.trace_count > 0);
        assert_eq!(plan.trace_count, plan.mode_count);
        assert!(plan.boundary.len() > plan.trace_count + plan.mode_count * MODE_WORDS);
        assert!(plan.trace_count <= CANONICAL_GPU_MAX_TRACE);
        assert_eq!(plan.manifest.dispatches_per_step, 13);
    }

    #[test]
    fn clock_contract_rebases_before_f32_integer_or_elapsed_limits() {
        let clock = CanonicalGpuClock {
            epoch: u64::MAX - 1,
            epoch_origin_seconds: 1.0e12,
            step_in_epoch: (1 << 16) - 1,
            time_step: 1.0 / 1024.0,
        };
        assert!(!clock.requires_rebase());
        assert!(
            CanonicalGpuClock {
                step_in_epoch: 1 << 16,
                ..clock
            }
            .requires_rebase()
        );
        assert!(
            CanonicalGpuClock {
                step_in_epoch: 32_768,
                time_step: 1.0 / 128.0,
                ..clock
            }
            .requires_rebase()
        );
    }

    #[test]
    fn clock_retime_preserves_time_and_signal_phase_anchor() {
        let clock = CanonicalGpuClock {
            epoch: 9,
            epoch_origin_seconds: 123.5,
            step_in_epoch: 12_345,
            time_step: 0.007,
        };
        let signal = TimeSignal::harmonic(0.3, 0.8, 1.7, -0.4);
        let before = gpu_signal(signal, clock).unwrap();
        let retimed = clock.retimed(0.0025).unwrap();
        let after = gpu_signal(signal, retimed).unwrap();
        assert_eq!(retimed.time(), clock.time());
        assert_eq!(retimed.step_in_epoch, 0);
        let old_value =
            before.x + before.y * (before.w + before.z * clock.local_seconds() as f32).sin();
        let new_value = after.x + after.y * after.w.sin();
        assert!((old_value - new_value).abs() < 2.0e-5);
    }

    #[test]
    fn every_linear_event_declares_a_staged_dispatch_path() {
        let base = plan(OuterBoundaryCondition::Reflecting);
        let mut pulse = base.clone();
        pulse
            .stage_primary_pulse(&vec![0.0; pulse.node_count], 1)
            .unwrap();
        assert_eq!(pulse.manifest.event_dispatches, 4);

        let mut filter = base.clone();
        filter.stage_grid_filter(0.5, 2).unwrap();
        assert_eq!(filter.manifest.event_dispatches, 6);

        let mut patch = base.clone();
        patch
            .stage_linear_loss_patch(
                &vec![0.0; patch.node_count],
                &vec![0.0; patch.sample_count],
                3,
            )
            .unwrap();
        assert_eq!(patch.manifest.event_dispatches, 4);

        let mut maintenance = base;
        maintenance
            .stage_maintenance(&vec![0.0; maintenance.node_count], 4)
            .unwrap();
        assert_eq!(maintenance.manifest.event_dispatches, 4);
    }

    #[test]
    fn latest_state_transfer_plan_packs_every_physical_state_class() {
        let scene = Scene::initial();
        let mesh = mesh_scene(
            &scene,
            9,
            MeshingOptions {
                target_edge_length: 0.25,
                ..MeshingOptions::default()
            },
        )
        .unwrap();
        let scalar = QuadraticWaveOperator::assemble_scene(
            &mesh,
            &scene,
            OuterBoundaryCondition::SecondOrderOutgoing,
        )
        .unwrap();
        let operator = CanonicalWaveOperator::compile_scene(&mesh, &scalar, &scene, 9).unwrap();
        let interpolation =
            QuadraticTransferMap::identity_on_mesh(&mesh, &scalar, &scalar).unwrap();
        let primary =
            CanonicalPrimaryTransferMap::prepare(&interpolation, &operator, &operator).unwrap();
        let vector =
            CanonicalVectorTransferMap::prepare(&mesh, &operator, &mesh, &operator).unwrap();
        let gap = CanonicalThinGapHistoryTransferMap::prepare(
            operator.thin_gap_samples(),
            operator.thin_gap_samples(),
        )
        .unwrap();
        let outgoing =
            CanonicalOutgoingHistoryTransferMap::prepare(&interpolation, &operator, &operator)
                .unwrap();
        let forcing = CanonicalForcing::none(&operator);
        let runtime =
            CanonicalGpuRuntimeTransfer::identity(&operator, &operator, &forcing, &forcing)
                .unwrap();
        let transfer = CanonicalGpuTransferPlan::compile(
            &operator, &operator, &forcing, &forcing, &primary, &vector, &gap, &outgoing, &runtime,
        )
        .unwrap();
        assert_eq!(transfer.manifest.layout_version, TRANSFER_LAYOUT_VERSION);
        assert_eq!(
            transfer.manifest.exact_primary,
            operator.degrees_of_freedom()
        );
        assert_eq!(
            transfer.manifest.exact_complementary,
            operator.complementary_degrees_of_freedom()
        );
        assert_eq!(transfer.manifest.dispatches, 14);
        assert_eq!(transfer.manifest.word_count, TRANSFER_HEADER_WORDS);
        assert_eq!(transfer.words[7].data.y, 1);
        assert_eq!(transfer.words[7].data.z, 1);
        assert_eq!(transfer.words[7].data.w, 1);
    }

    #[test]
    fn edited_source_handoff_selects_target_parameters_with_old_runtime_anchor() {
        let scene = Scene::initial();
        let mesh = mesh_scene(
            &scene,
            17,
            MeshingOptions {
                target_edge_length: 0.3,
                ..MeshingOptions::default()
            },
        )
        .unwrap();
        let scalar = QuadraticWaveOperator::assemble_scene(
            &mesh,
            &scene,
            OuterBoundaryCondition::Reflecting,
        )
        .unwrap();
        let operator = CanonicalWaveOperator::compile_scene(&mesh, &scalar, &scene, 17).unwrap();
        let interpolation =
            QuadraticTransferMap::identity_on_mesh(&mesh, &scalar, &scalar).unwrap();
        let primary =
            CanonicalPrimaryTransferMap::prepare(&interpolation, &operator, &operator).unwrap();
        let vector =
            CanonicalVectorTransferMap::prepare(&mesh, &operator, &mesh, &operator).unwrap();
        let gap = CanonicalThinGapHistoryTransferMap::prepare(
            operator.thin_gap_samples(),
            operator.thin_gap_samples(),
        )
        .unwrap();
        let outgoing =
            CanonicalOutgoingHistoryTransferMap::prepare(&interpolation, &operator, &operator)
                .unwrap();
        let mut source = CanonicalForcing::none(&operator);
        source
            .push_source(
                CanonicalSource::legacy(
                    &operator,
                    operator.primary_mass().to_vec(),
                    TimeSignal::harmonic(0.0, 1.0, 2.0, 0.1),
                    0.0,
                )
                .unwrap(),
            )
            .unwrap();
        let mut target = CanonicalForcing::none(&operator);
        target
            .push_source(
                CanonicalSource::legacy(
                    &operator,
                    operator.primary_mass().to_vec(),
                    TimeSignal::harmonic(0.2, 0.7, 3.0, 0.4),
                    0.0,
                )
                .unwrap(),
            )
            .unwrap();
        let runtime =
            CanonicalGpuRuntimeTransfer::identity(&operator, &operator, &source, &target).unwrap();
        let transfer = CanonicalGpuTransferPlan::compile(
            &operator, &operator, &source, &target, &primary, &vector, &gap, &outgoing, &runtime,
        )
        .unwrap();
        let drive_offset = transfer.words[4].data.w as usize;
        assert_eq!(transfer.words[drive_offset].data.x, DRIVE_TARGET_PARAMETERS);
        let shader = include_str!("canonical_transfer_runtime.wgsl");
        assert!(shader.contains("replace_drive(old_base + slot, new_base"));
        assert!(shader.contains("runtime.y = bitcast<u32>(old_drive(old_base, old_elapsed));"));
        assert!(shader.contains("start_drive(new_base, preparation_delta);"));
    }

    #[test]
    fn live_event_payloads_are_zero_duration_and_revision_ordered() {
        let base = plan(OuterBoundaryCondition::Reflecting);
        let operator_nodes = base.node_count;
        let pulse = CanonicalGpuLiveEvent::maintenance(&vec![0.0; operator_nodes], 7).unwrap();
        assert_eq!(pulse.serial, 7);
        assert_eq!(pulse.kind, EVENT_MAINTENANCE);
        assert_eq!(pulse.upload.len(), operator_nodes + 1);
        assert!(CanonicalGpuLiveEvent::grid_filter(1.1, 8).is_err());
        assert!(CanonicalGpuLiveEvent::maintenance(&[], 0).is_err());
    }

    #[test]
    fn resident_filter_toggle_does_not_create_a_host_event_or_revision() {
        let mut request = CanonicalGpuRequest::default();
        let revision = request.revision();
        request.set_grid_scale_filter(true);
        assert!(request.grid_scale_filter());
        assert_eq!(request.revision(), revision);
        assert!(!request.live_event_pending());
    }
}
