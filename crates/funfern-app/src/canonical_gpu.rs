//! Production-intended f32 layout and execution path for the canonical
//! integrated-flux solver.
//!
//! Stage 4 deliberately lives beside the currently connected scalar GPU
//! solver.  The old path remains the application path until transfer and every
//! consumer have crossed their later gates.

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
    CanonicalAuxiliaryState, CanonicalForcing, CanonicalOutgoingMidpointFactor, CanonicalRateDrive,
    CanonicalWaveOperator, CanonicalWaveState, Point2, TimeSignal, WaveError,
};

pub const CANONICAL_GPU_LAYOUT_VERSION: u32 = 1;
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
    /// Accepted event serial, candidate event serial, operation and flags.
    pub event: UVec4,
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
        let mut stiffness_by_node = vec![BTreeMap::<u32, f64>::new(); node_count];
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
        for (node, row) in stiffness_by_node.into_iter().enumerate() {
            stiffness_ranges[node] = (usize_u32(tables.len())?, usize_u32(row.len())?);
            tables.extend(
                row.into_iter()
                    .map(|(column, coefficient)| table_word(column, 0, coefficient as f32, 0.0)),
            );
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
            event: UVec4::ZERO,
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
}

#[derive(Clone)]
struct CanonicalGpuBufferHandles {
    control: Handle<ShaderBuffer>,
    status: Handle<ShaderBuffer>,
    state: Handle<ShaderBuffer>,
    nodes: Handle<ShaderBuffer>,
    samples: Handle<ShaderBuffer>,
    tables: Handle<ShaderBuffer>,
    scratch: Handle<ShaderBuffer>,
    boundary: Handle<ShaderBuffer>,
    node_count: u32,
    sample_count: u32,
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
        }
    }
}

impl CanonicalGpuRequest {
    pub fn install(
        &mut self,
        assets: &mut Assets<ShaderBuffer>,
        commands: &mut Commands,
        plan: CanonicalGpuPlan,
    ) {
        self.clear(assets, commands);
        let generation = self.generation.wrapping_add(1).max(1);
        self.stats = Arc::new(CanonicalGpuStats::default());
        let initial_step = plan.control.clock_u32.w as u64;
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
        let control = assets.add(ShaderBuffer::from(plan.control));
        let status = assets.add(ShaderBuffer::from(plan.status));
        let state = assets.add(ShaderBuffer::from(plan.state));
        let nodes = assets.add(ShaderBuffer::from(plan.nodes));
        let samples = assets.add(ShaderBuffer::from(plan.samples));
        let tables = assets.add(ShaderBuffer::from(plan.tables));
        let scratch = assets.add(ShaderBuffer::from(plan.scratch));
        let boundary = assets.add(ShaderBuffer::from(plan.boundary));
        let handles = CanonicalGpuBufferHandles {
            control,
            status,
            state,
            nodes,
            samples,
            tables,
            scratch,
            boundary,
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
            event_dispatches: plan.manifest.event_dispatches as u64,
        };
        let state_entity = commands
            .spawn((
                Readback::buffer(handles.state.clone()),
                CanonicalStateReadback {
                    generation,
                    node_count,
                    sample_count,
                    state_count,
                },
            ))
            .id();
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
        self.desired_steps = initial_step;
        self.stats
            .completed_steps
            .store(initial_step, Ordering::Relaxed);
        self.stats
            .local_step
            .store(plan.control.clock_u32.z, Ordering::Relaxed);
        self.manifest = Some(plan.manifest);
        self.buffers = Some(handles);
        self.readback_entities = vec![state_entity, control_entity, status_entity];
        self.status_readback_entity = Some(status_entity);
    }

    pub fn clear(&mut self, assets: &mut Assets<ShaderBuffer>, commands: &mut Commands) {
        if let Some(handles) = self.buffers.take() {
            for handle in handles.all() {
                assets.remove(handle.id());
            }
        }
        for entity in self.readback_entities.drain(..) {
            commands.entity(entity).despawn();
        }
        self.status_readback_entity = None;
        self.manifest = None;
    }

    pub fn request_steps(&mut self, count: u64) {
        self.desired_steps = self.desired_steps.saturating_add(count);
    }

    pub fn requested_steps(&self) -> u64 {
        self.desired_steps
    }

    pub fn caught_up(&self) -> bool {
        self.stats.completed_steps() >= self.desired_steps || self.stats.failure() != 0
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn manifest(&self) -> Option<&CanonicalGpuLayoutManifest> {
        self.manifest.as_ref()
    }

    pub fn stats(&self) -> &Arc<CanonicalGpuStats> {
        &self.stats
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
        let replacement = assets.add(ShaderBuffer::from(value));
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
    pub complementary_flux: Vec<[f32; 2]>,
    pub auxiliary: Vec<f32>,
    pub clock: Option<CanonicalGpuDisplayClock>,
    pub accounting: [f32; 8],
    pub readbacks: u64,
    raw_state: Vec<GpuCanonicalStateWord>,
    node_count: usize,
    sample_count: usize,
    accepted_slot: u32,
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

fn receive_canonical_state(
    event: On<ReadbackComplete>,
    tags: Query<&CanonicalStateReadback>,
    mut display: ResMut<CanonicalGpuDisplay>,
) {
    let Ok(tag) = tags.get(event.entity) else {
        return;
    };
    let words: Vec<GpuCanonicalStateWord> = event.to_shader_type();
    if words.len() != tag.state_count as usize {
        return;
    }
    display.generation = tag.generation;
    display.node_count = tag.node_count as usize;
    display.sample_count = tag.sample_count as usize;
    display.raw_state = words;
    refresh_canonical_display(&mut display);
    display.readbacks = display.readbacks.saturating_add(1);
}

fn receive_canonical_control(
    event: On<ReadbackComplete>,
    tags: Query<&CanonicalControlReadback>,
    mut display: ResMut<CanonicalGpuDisplay>,
) {
    let Ok(tag) = tags.get(event.entity) else {
        return;
    };
    let values: Vec<GpuCanonicalControl> = event.to_shader_type();
    let Some(value) = values.first() else { return };
    let step = value.clock_u32.z;
    tag.stats
        .completed_steps
        .store(value.clock_u32.w as u64, Ordering::Relaxed);
    tag.stats.local_step.store(step, Ordering::Relaxed);
    if tag.stats.failure() == 0 {
        tag.stats.status.store(GPU_STATUS_READY, Ordering::Relaxed);
    }
    display.generation = tag.generation;
    display.clock = Some(CanonicalGpuDisplayClock {
        epoch: value.clock_u32.x as u64 | ((value.clock_u32.y as u64) << 32),
        step_in_epoch: step,
        accepted_steps: value.clock_u32.w,
        local_seconds: value.clock_f32.y,
        time_step: value.clock_f32.x,
        event_serial: value.event.x,
    });
    display.accounting[..4].copy_from_slice(&value.accepted_accounting_a.to_array());
    display.accounting[4..].copy_from_slice(&value.accepted_accounting_b.to_array());
    display.accepted_slot = value.event.z & 1;
    refresh_canonical_display(&mut display);
}

fn refresh_canonical_display(display: &mut CanonicalGpuDisplay) {
    let nodes = display.node_count;
    let samples = display.sample_count;
    if display.raw_state.len() < nodes + samples {
        return;
    }
    let second = display.accepted_slot != 0;
    display.primary_flux = display.raw_state[..nodes]
        .iter()
        .map(|word| if second { word.values.y } else { word.values.x })
        .collect();
    display.complementary_flux = display.raw_state[nodes..nodes + samples]
        .iter()
        .map(|word| {
            if second {
                [word.values.z, word.values.w]
            } else {
                [word.values.x, word.values.y]
            }
        })
        .collect();
    display.auxiliary = display.raw_state[nodes + samples..]
        .iter()
        .map(|word| if second { word.values.y } else { word.values.x })
        .collect();
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

pub struct CanonicalWaveGpuPlugin;

impl Plugin for CanonicalWaveGpuPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "canonical_wave.wgsl");
        app.init_resource::<CanonicalGpuRequest>()
            .init_resource::<CanonicalGpuDisplay>()
            .add_observer(receive_canonical_state)
            .add_observer(receive_canonical_control)
            .add_observer(receive_canonical_status)
            .add_plugins(ExtractResourcePlugin::<CanonicalGpuRequest>::default());
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        render_app
            .add_systems(RenderStartup, init_canonical_pipeline)
            .add_systems(
                Render,
                prepare_canonical_bind_group.in_set(RenderSystems::PrepareBindGroups),
            )
            .add_systems(RenderGraph, compute_canonical_wave.before(camera_driver));
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
    });
}

#[derive(Resource)]
struct CanonicalBindGroup {
    generation: u64,
    revision: u64,
    encoded_steps: u64,
    encoded_local_step: u32,
    encoded_event: bool,
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
    let encoded_steps = request.stats.completed_steps();
    commands.insert_resource(CanonicalBindGroup {
        generation: request.generation,
        revision: request.revision,
        encoded_steps,
        encoded_local_step: request.stats.local_step(),
        encoded_event: false,
        bind_group,
    });
}

fn compute_canonical_wave(
    mut render_context: RenderContext,
    request: Option<Res<CanonicalGpuRequest>>,
    group: Option<ResMut<CanonicalBindGroup>>,
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
    let has_pending_event = handles.event_kind != EVENT_NONE && !group.encoded_event;
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
        pass.set_pipeline(pipelines[14]);
        pass.dispatch_workgroups(workgroups(handles.state_count), 1, 1);
        match handles.event_kind {
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
            _ => unreachable!("validated canonical GPU event kind"),
        }
        pass.set_pipeline(pipelines[21]);
        pass.dispatch_workgroups(1, 1, 1);
        group.encoded_event = true;
        request
            .stats
            .dispatches
            .fetch_add(handles.event_dispatches, Ordering::Relaxed);
    }
    let mut rebases = 0_u64;
    for _ in 0..pending {
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
            .saturating_add(u64::from(handles.needs_accounting && pending != 0)),
        Ordering::Relaxed,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use funfern_core::{
        CanonicalSource, MeshingOptions, OuterBoundaryCondition, QuadraticWaveOperator, Scene,
        mesh_scene,
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
            "const LAYOUT_VERSION: u32 = 1u;",
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
        assert_eq!(size_of::<GpuCanonicalControl>(), 208);
        assert_eq!(GpuCanonicalControl::min_size().get() as usize, 208);
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
}
