// Canonical direct-state f32 solver. Rust layout version 3.
const LAYOUT_VERSION: u32 = 3u;
const STATE_WORD_STRIDE: u32 = 16u;
const NODE_STRIDE: u32 = 96u;
const SAMPLE_STRIDE: u32 = 112u;
const TABLE_WORD_STRIDE: u32 = 16u;
const WORKGROUP_SIZE: u32 = 128u;
const MAX_TRACE: u32 = 1024u;
const MODE_WORDS: u32 = 12u;
const NO_INDEX: u32 = 0xffffffffu;
const FORCE_KIND_GAP: u32 = 1u;
const SNAPSHOT_METADATA_MAGIC: f32 = 8675309.0;

const STATUS_LAYOUT: u32 = 1u;
const STATUS_TIMESTEP: u32 = 2u;
const STATUS_INVERSE_DOMAIN: u32 = 3u;
const STATUS_NON_FINITE: u32 = 4u;
// Leave serialization headroom below f32::MAX: Naga's decimal WGSL writer
// rounds the exact maximum upward, which Chrome correctly rejects.
const MAX_FINITE: f32 = 3.0e+38;

struct Control {
    counts_a: vec4<u32>,
    counts_b: vec4<u32>,
    counts_c: vec4<u32>,
    table_offsets: vec4<u32>,
    boundary_offsets: vec4<u32>,
    clock_u32: vec4<u32>,
    clock_f32: vec4<f32>,
    clock_origin: vec4<f32>,
    event: vec4<u32>,
    event_result: vec4<u32>,
    runtime_serials: vec4<u32>,
    runtime_slots: vec4<u32>,
    accepted_accounting_a: vec4<f32>,
    accepted_accounting_b: vec4<f32>,
    candidate_accounting_a: vec4<f32>,
    candidate_accounting_b: vec4<f32>,
    evolution: vec4<f32>,
}

struct Status {
    candidate: atomic<u32>,
    latch: atomic<u32>,
    injection: atomic<u32>,
    injection_index: atomic<u32>,
    handoff: atomic<u32>,
    transaction_1: atomic<u32>,
    transaction_2: atomic<u32>,
    transaction_3: atomic<u32>,
}

struct StateWord { values: vec4<f32> }

struct Node {
    mass_loss: vec4<f32>,
    ranges: vec4<u32>,
    boundary: vec4<u32>,
    prescribed: vec4<f32>,
    damping_support: vec4<f32>,
    stiffness: vec4<u32>,
}

struct Sample {
    nodes_a: vec4<u32>,
    nodes_b: vec4<u32>,
    curls_01: vec4<f32>,
    curls_23: vec4<f32>,
    curls_45: vec4<f32>,
    curl_6_loss: vec4<f32>,
    constitutive: vec4<f32>,
}

struct TableWord { data: vec4<u32> }
struct ScratchWord { values: vec4<f32> }

@group(0) @binding(0) var<storage, read_write> control: Control;
@group(0) @binding(1) var<storage, read_write> status: Status;
@group(0) @binding(2) var<storage, read_write> state: array<StateWord>;
@group(0) @binding(3) var<storage, read_write> nodes: array<Node>;
@group(0) @binding(4) var<storage, read_write> samples: array<Sample>;
@group(0) @binding(5) var<storage, read_write> tables: array<TableWord>;
@group(0) @binding(6) var<storage, read_write> scratch: array<ScratchWord>;
@group(0) @binding(7) var<storage, read_write> boundary: array<TableWord>;

// Only the accounting reduction needs workgroup memory. Boundary transforms
// use globally ordered dispatches so large traces occupy the whole device.
var<workgroup> mode_values: array<vec2<f32>, 128>;
var<workgroup> reduced_values: array<f32, 128>;
var<workgroup> solved_values: array<f32, 128>;

fn stopped() -> bool {
    return atomicLoad(&status.latch) != 0u || atomicLoad(&status.candidate) != 0u;
}

fn reject(reason: u32) {
    atomicMax(&status.candidate, reason);
}

fn finite_scalar(value: f32) -> bool {
    return value >= -MAX_FINITE && value <= MAX_FINITE;
}

fn finite_vector(value: vec4<f32>) -> bool {
    return all(value >= vec4<f32>(-MAX_FINITE))
        && all(value <= vec4<f32>(MAX_FINITE));
}

fn primary_offset() -> u32 { return 0u; }
fn complementary_offset() -> u32 { return control.counts_a.x; }
fn auxiliary_offset() -> u32 { return control.counts_a.x + control.counts_a.y; }
fn mode_temporary_offset() -> u32 { return control.counts_a.w; }
fn mode_accounting_offset() -> u32 { return control.counts_a.w + control.counts_b.z; }
fn mode_modal_offset() -> u32 {
    return control.counts_a.w + 2u * control.counts_b.z;
}
fn trace_solution_offset() -> u32 {
    return control.counts_a.w + 3u * control.counts_b.z;
}
fn scratch_count() -> u32 {
    return trace_solution_offset() + control.counts_b.y;
}
fn accounting_item_count() -> u32 {
    return control.counts_a.x + control.counts_a.y + control.counts_b.z;
}
fn accounting_bank_offset(slot: u32) -> u32 {
    return scratch_count() + slot * accounting_item_count();
}
fn has_loss_stages() -> bool { return (control.boundary_offsets.w & 1u) != 0u; }
fn use_force_cache() -> bool { return (control.boundary_offsets.w & 4u) != 0u; }
fn has_prescribed_trace() -> bool { return (control.boundary_offsets.w & 8u) != 0u; }
fn accepted_slot() -> u32 { return control.event.z & 1u; }
fn event_operation() -> u32 { return control.event.z >> 8u; }
fn live_event() -> bool { return (control.event.z & 2u) != 0u; }

fn publish_snapshot_metadata() {
    state[control.counts_a.w].values = vec4<f32>(
        SNAPSHOT_METADATA_MAGIC,
        f32(accepted_slot()),
        f32(control.clock_u32.w & 0xffffu),
        f32(control.clock_u32.w >> 16u));
}

fn inject_at(state_word: u32) {
    let injection = atomicLoad(&status.injection);
    if injection != 0u && state_word == atomicLoad(&status.injection_index) {
        reject(injection);
    }
}

fn accepted_q(node: u32) -> f32 {
    return select(state[node].values.x, state[node].values.y, accepted_slot() != 0u);
}
fn candidate_q(node: u32) -> f32 {
    return select(state[node].values.y, state[node].values.x, accepted_slot() != 0u);
}
fn set_candidate_q(node: u32, value: f32) {
    if accepted_slot() == 0u { state[node].values.y = value; }
    else { state[node].values.x = value; }
}
fn accepted_force(node: u32) -> f32 {
    return select(state[node].values.z, state[node].values.w, accepted_slot() != 0u);
}
fn candidate_force(node: u32) -> f32 {
    return select(state[node].values.w, state[node].values.z, accepted_slot() != 0u);
}
fn set_candidate_force(node: u32, value: f32) {
    if accepted_slot() == 0u { state[node].values.w = value; }
    else { state[node].values.z = value; }
}
fn accepted_b(sample: u32) -> vec2<f32> {
    return select(
        state[complementary_offset() + sample].values.xy,
        state[complementary_offset() + sample].values.zw,
        accepted_slot() != 0u);
}
fn candidate_b(sample: u32) -> vec2<f32> {
    return select(
        state[complementary_offset() + sample].values.zw,
        state[complementary_offset() + sample].values.xy,
        accepted_slot() != 0u);
}
fn set_candidate_b(sample: u32, value: vec2<f32>) {
    let word = complementary_offset() + sample;
    if accepted_slot() == 0u {
        state[word].values.z = value.x;
        state[word].values.w = value.y;
    } else {
        state[word].values.x = value.x;
        state[word].values.y = value.y;
    }
}
fn accepted_auxiliary(index: u32) -> f32 {
    let value = state[auxiliary_offset() + index].values;
    return select(value.x, value.y, accepted_slot() != 0u);
}
fn candidate_auxiliary(index: u32) -> f32 {
    let value = state[auxiliary_offset() + index].values;
    return select(value.y, value.x, accepted_slot() != 0u);
}
fn set_candidate_auxiliary(index: u32, value: f32) {
    let word = auxiliary_offset() + index;
    if accepted_slot() == 0u { state[word].values.y = value; }
    else { state[word].values.x = value; }
}

fn harmonic_value(signal: vec4<f32>, local_time: f32) -> f32 {
    return signal.x + signal.y * sin(signal.w + signal.z * local_time);
}

fn sinc(value: f32) -> f32 {
    if abs(value) < 0.0005 {
        let square = value * value;
        return 1.0 - square / 6.0 + square * square / 120.0;
    }
    return sin(value) / value;
}

fn reduced_phase(value: f32) -> f32 {
    return atan2(sin(value), cos(value));
}

fn add_compensated(high: f32, low: f32, increment: f32) -> vec2<f32> {
    let sum = high + increment;
    let residual = (high - sum) + increment + low;
    let next_high = sum + residual;
    return vec2<f32>(next_high, (sum - next_high) + residual);
}

fn table_float(word: u32, lane: u32) -> f32 {
    return bitcast<f32>(tables[word].data[lane]);
}

fn boundary_float(word: u32, lane: u32) -> f32 {
    return bitcast<f32>(boundary[word].data[lane]);
}

fn packed_boundary_scalar(word_offset: u32, scalar_index: u32) -> f32 {
    return boundary_float(word_offset + scalar_index / 4u, scalar_index % 4u);
}

fn source_drive(drive: u32, local_time: f32) -> f32 {
    let base = control.table_offsets.y + 4u * drive + 2u * (control.runtime_slots.x & 1u);
    let signal = vec4<f32>(
        table_float(base, 0u), table_float(base, 1u),
        table_float(base, 2u), table_float(base, 3u));
    let kind = tables[base + 1u].data.x;
    if kind == 0u {
        return harmonic_value(signal, local_time);
    }
    let rate_anchor = table_float(base + 1u, 1u);
    let half_phase = 0.5 * signal.z * local_time;
    return rate_anchor + signal.x * local_time
        + signal.y * local_time * sinc(half_phase) * sin(signal.w + half_phase);
}

fn source_rate(node: u32, local_time: f32) -> f32 {
    let range = nodes[node].ranges.zw;
    var result = 0.0;
    for (var entry = range.x; entry < range.x + range.y; entry += 1u) {
        let drive = tables[entry].data.x;
        result += table_float(entry, 2u) * source_drive(drive, local_time);
    }
    return result;
}

fn gathered_force(node: u32, second: bool) -> f32 {
    let range = nodes[node].ranges.xy;
    var result = 0.0;
    for (var entry = range.x; entry < range.x + range.y; entry += 1u) {
        let index = tables[entry].data.x;
        let kind = tables[entry].data.y;
        let coefficient_x = table_float(entry, 2u);
        if kind == FORCE_KIND_GAP {
            let auxiliary = index - auxiliary_offset();
            result += coefficient_x * select(
                accepted_auxiliary(auxiliary), candidate_auxiliary(auxiliary),
                second || has_loss_stages());
        } else {
            let flux = select(
                accepted_b(index), candidate_b(index), second || has_loss_stages());
            result += dot(vec2<f32>(coefficient_x, table_float(entry, 3u)), flux);
        }
    }
    return result;
}

fn gap_force(node: u32, second: bool) -> f32 {
    let range = nodes[node].ranges.xy;
    var result = 0.0;
    for (var entry = range.x; entry < range.x + range.y; entry += 1u) {
        if tables[entry].data.y == FORCE_KIND_GAP {
            let auxiliary = tables[entry].data.x - auxiliary_offset();
            result += table_float(entry, 2u) * select(
                accepted_auxiliary(auxiliary), candidate_auxiliary(auxiliary),
                second || has_loss_stages());
        }
    }
    return result;
}

fn force(node: u32, second: bool) -> f32 {
    if use_force_cache() {
        let cached = select(accepted_force(node), candidate_force(node), second);
        return cached + gap_force(node, second);
    }
    return gathered_force(node, second);
}

fn stiffness_force(node: u32) -> f32 {
    let range = nodes[node].stiffness.xy;
    let row_field = candidate_q(node) * nodes[node].mass_loss.y;
    var result = 0.0;
    for (var entry = range.x; entry < range.x + range.y; entry += 1u) {
        let column = tables[entry].data.x;
        let column_field = candidate_q(column) * nodes[column].mass_loss.y;
        result += table_float(entry, 2u) * (column_field - row_field);
    }
    return result;
}

fn constitutive_force(node: u32) -> f32 {
    let range = nodes[node].ranges.xy;
    var result = 0.0;
    for (var entry = range.x; entry < range.x + range.y; entry += 1u) {
        if tables[entry].data.y != FORCE_KIND_GAP {
            let sample = tables[entry].data.x;
            let coefficient = vec2<f32>(
                table_float(entry, 2u), table_float(entry, 3u));
            result += dot(coefficient, accepted_b(sample));
        }
    }
    return result;
}

fn candidate_constitutive_force(node: u32) -> f32 {
    let range = nodes[node].ranges.xy;
    var result = 0.0;
    for (var entry = range.x; entry < range.x + range.y; entry += 1u) {
        if tables[entry].data.y != FORCE_KIND_GAP {
            let sample = tables[entry].data.x;
            let coefficient = vec2<f32>(
                table_float(entry, 2u), table_float(entry, 3u));
            result += dot(coefficient, candidate_b(sample));
        }
    }
    return result;
}

fn stiffness_of_scratch(node: u32, lane: u32) -> f32 {
    let range = nodes[node].stiffness.xy;
    let row_field = scratch[node].values[lane] * nodes[node].mass_loss.y;
    var result = 0.0;
    for (var entry = range.x; entry < range.x + range.y; entry += 1u) {
        let column = tables[entry].data.x;
        let column_field = scratch[column].values[lane] * nodes[column].mass_loss.y;
        result += table_float(entry, 2u) * (column_field - row_field);
    }
    return result;
}

fn b_energy(sample_index: u32, value: vec2<f32>) -> f32 {
    let constitutive = samples[sample_index].constitutive;
    let field = vec2<f32>(
        constitutive.x * value.x + constitutive.y * value.y,
        constitutive.y * value.x + constitutive.z * value.y);
    return 0.5 * constitutive.w * dot(value, field);
}

fn sample_node(sample: Sample, local: u32) -> u32 {
    if local < 4u { return sample.nodes_a[local]; }
    return sample.nodes_b[local - 4u];
}

fn sample_curl(sample: Sample, local: u32) -> vec2<f32> {
    switch local {
        case 0u: { return sample.curls_01.xy; }
        case 1u: { return sample.curls_01.zw; }
        case 2u: { return sample.curls_23.xy; }
        case 3u: { return sample.curls_23.zw; }
        case 4u: { return sample.curls_45.xy; }
        case 5u: { return sample.curls_45.zw; }
        default: { return sample.curl_6_loss.xy; }
    }
}

// In this entry point binding 7 is the transient upload buffer rather than
// the immutable outgoing-boundary table. Subsequent event stages do not read
// boundary data, so the same eight-binding layout remains portable.
@compute @workgroup_size(128)
fn live_event_stage(@builtin(global_invocation_id) id: vec3<u32>) {
    let i = id.x;
    if stopped() { return; }
    let upload = boundary[0u].data;
    let operation = upload.x;
    if i == 0u {
        control.event.y = upload.y;
        control.event.z = accepted_slot() | 2u | (operation << 8u);
        control.event.w = upload.w;
    }
    if operation == 1u || operation == 4u {
        if i < control.counts_a.x && i < upload.z {
            scratch[i].values.x = bitcast<f32>(boundary[1u + i].data.x);
        }
        return;
    }
    if operation == 3u {
        if i < control.counts_a.x {
            nodes[i].mass_loss.w = bitcast<f32>(boundary[1u + i].data.x);
        } else if i < control.counts_a.x + control.counts_a.y {
            let sample = i - control.counts_a.x;
            samples[sample].curl_6_loss.w = bitcast<f32>(
                boundary[1u + control.counts_a.x + sample].data.x);
        }
        return;
    }
    if operation == 5u && i < control.counts_c.z {
        let source_slot = control.runtime_slots.x & 1u;
        let accepted_base = control.table_offsets.y + 4u * i + 2u * source_slot;
        let candidate_base = control.table_offsets.y + 4u * i
            + 2u * (source_slot ^ 1u);
        let upload_base = 1u + 4u * i;
        var parameters = bitcast<vec4<f32>>(boundary[upload_base].data);
        var runtime = boundary[upload_base + 1u].data;
        let old_parameters = vec4<f32>(
            table_float(accepted_base, 0u), table_float(accepted_base, 1u),
            table_float(accepted_base, 2u), table_float(accepted_base, 3u));
        let current_phase = reduced_phase(
            old_parameters.w + old_parameters.z * control.clock_f32.y);
        parameters.w = reduced_phase(
            current_phase - parameters.z * control.clock_f32.y);
        if runtime.x != 0u {
            let elapsed = control.clock_f32.y;
            let half_phase = 0.5 * parameters.z * elapsed;
            let new_integral = parameters.x * elapsed
                + parameters.y * elapsed * sinc(half_phase)
                    * sin(parameters.w + half_phase);
            runtime.y = bitcast<u32>(
                source_drive(i, control.clock_f32.y) - new_integral);
        }
        tables[candidate_base].data = bitcast<vec4<u32>>(parameters);
        tables[candidate_base + 1u].data = runtime;
        if !finite_vector(parameters) { reject(STATUS_NON_FINITE); }
    }
}

// Periodic grid damping is resident solver maintenance, not a host-authored
// event. Start the same staged paired filter without touching event serials or
// runtime ownership; the matching commit below accepts it atomically on the
// GPU before the next ordinary step in this command buffer.
@compute @workgroup_size(1)
fn resident_filter_begin() {
    if stopped() { return; }
    control.event.z = accepted_slot() | (2u << 8u);
    control.event.w = bitcast<u32>(1.0);
}

@compute @workgroup_size(128)
fn event_begin(@builtin(global_invocation_id) id: vec3<u32>) {
    let i = id.x;
    if stopped() || event_operation() == 0u { return; }
    if i == 0u {
        control.candidate_accounting_a = control.accepted_accounting_a;
        control.candidate_accounting_b = control.accepted_accounting_b;
    }
    // State and the cumulative step-accounting bank share the accepted lane.
    // A zero-duration event flips that lane, so begin a fresh contribution
    // bank from the consolidated control totals rather than exposing the
    // previous candidate bank as accepted.
    if i < accounting_item_count() {
        let candidate = accounting_bank_offset(accepted_slot() ^ 1u) + i;
        scratch[candidate].values = vec4<f32>(0.0);
    }
    if i < control.counts_a.x {
        set_candidate_q(i, accepted_q(i));
        set_candidate_force(i, accepted_force(i));
    } else if i < control.counts_a.x + control.counts_a.y {
        let sample = i - complementary_offset();
        set_candidate_b(sample, accepted_b(sample));
    } else if i < control.counts_a.w {
        let auxiliary = i - auxiliary_offset();
        set_candidate_auxiliary(auxiliary, accepted_auxiliary(auxiliary));
    }
    if i < control.counts_c.z && event_operation() != 5u {
        let source_slot = control.runtime_slots.x & 1u;
        let accepted_base = control.table_offsets.y + 4u * i + 2u * source_slot;
        let candidate_base = control.table_offsets.y + 4u * i
            + 2u * (source_slot ^ 1u);
        tables[candidate_base].data = tables[accepted_base].data;
        tables[candidate_base + 1u].data = tables[accepted_base + 1u].data;
    }
    inject_at(i);
}

@compute @workgroup_size(128)
fn event_simple_stage(@builtin(global_invocation_id) id: vec3<u32>) {
    let i = id.x;
    if stopped() { return; }
    let operation = event_operation();
    if i < control.counts_a.x {
        if operation == 1u || operation == 4u {
            var next = accepted_q(i) + scratch[i].values.x;
            if nodes[i].boundary.z != 0u {
                next = accepted_q(i);
            }
            set_candidate_q(i, next);
            if !finite_scalar(next) { reject(STATUS_NON_FINITE); }
        } else if operation == 3u {
            let fraction = nodes[i].mass_loss.w;
            if !finite_scalar(fraction) || fraction < 0.0 || fraction > 1.0 {
                reject(STATUS_LAYOUT);
            }
        }
    } else if operation == 3u && i < control.counts_a.x + control.counts_a.y {
        let sample = i - complementary_offset();
        let fraction = samples[sample].curl_6_loss.w;
        if !finite_scalar(fraction) || fraction < 0.0 || fraction > 1.0 {
            reject(STATUS_LAYOUT);
        }
    }
}

@compute @workgroup_size(128)
fn filter_first(@builtin(global_invocation_id) id: vec3<u32>) {
    let node = id.x;
    if stopped() || node >= control.counts_a.x { return; }
    scratch[node].values.x = stiffness_force(node);
    scratch[node].values.y = constitutive_force(node);
}

@compute @workgroup_size(128)
fn filter_second(@builtin(global_invocation_id) id: vec3<u32>) {
    let node = id.x;
    if stopped() || node >= control.counts_a.x { return; }
    scratch[node].values.z = stiffness_of_scratch(node, 0u);
    scratch[node].values.w = stiffness_of_scratch(node, 1u);
}

@compute @workgroup_size(128)
fn filter_finalize(@builtin(global_invocation_id) id: vec3<u32>) {
    let i = id.x;
    if stopped() { return; }
    let scale = bitcast<f32>(control.event.w) * control.evolution.x;
    if i < control.counts_a.x {
        var next = accepted_q(i) - scale * scratch[i].values.z;
        if nodes[i].boundary.z != 0u { next = accepted_q(i); }
        set_candidate_q(i, next);
        let next_force = accepted_force(i) - scale * stiffness_of_scratch(i, 3u);
        set_candidate_force(i, next_force);
        if !finite_scalar(next) || !finite_scalar(next_force) {
            reject(STATUS_NON_FINITE);
        }
        return;
    }
    if i < control.counts_a.x + control.counts_a.y {
        let sample_index = i - complementary_offset();
        let sample = samples[sample_index];
        let reference = scratch[sample.nodes_a.x].values.w
            * nodes[sample.nodes_a.x].mass_loss.y;
        var correction = vec2<f32>(0.0);
        for (var local = 1u; local < 7u; local += 1u) {
            let node = sample_node(sample, local);
            let field = scratch[node].values.w * nodes[node].mass_loss.y;
            correction += sample_curl(sample, local) * (field - reference);
        }
        let next = accepted_b(sample_index) - scale * control.evolution.y * correction;
        set_candidate_b(sample_index, next);
        if !all(next >= vec2<f32>(-MAX_FINITE))
            || !all(next <= vec2<f32>(MAX_FINITE)) {
            reject(STATUS_NON_FINITE);
        }
    }
}

@compute @workgroup_size(128)
fn event_validate(@builtin(local_invocation_id) id: vec3<u32>) {
    let local = id.x;
    let participating = !stopped();
    var energies = vec2<f32>(0.0);
    if participating {
        for (var node = local; node < control.counts_a.x; node += WORKGROUP_SIZE) {
            let inverse_mass = nodes[node].mass_loss.y;
            let accepted = accepted_q(node);
            let candidate = candidate_q(node);
            energies += 0.5 * inverse_mass * vec2<f32>(accepted * accepted, candidate * candidate);
        }
        for (var sample = local; sample < control.counts_a.y; sample += WORKGROUP_SIZE) {
            energies.x += b_energy(sample, accepted_b(sample));
            energies.y += b_energy(sample, candidate_b(sample));
        }
    }
    let total = reduce_boundary_vector(local, energies);
    if !participating || local != 0u { return; }
    if !all(total >= vec2<f32>(-MAX_FINITE))
        || !all(total <= vec2<f32>(MAX_FINITE)) {
        reject(STATUS_NON_FINITE);
        return;
    }
    let change = total.y - total.x;
    switch event_operation() {
        case 1u: { control.candidate_accounting_a.x += change; }
        case 2u: {
            let tolerance = 5.0e-6 * max(total.x, 1.0);
            if change > tolerance { reject(STATUS_TIMESTEP); }
            control.candidate_accounting_b.y += max(-change, 0.0);
        }
        case 4u: { control.candidate_accounting_b.z += change; }
        default: {}
    }
}

@compute @workgroup_size(128)
fn event_accept_tables(@builtin(global_invocation_id) id: vec3<u32>) {
    let i = id.x;
    if atomicLoad(&status.candidate) != 0u || atomicLoad(&status.latch) != 0u { return; }
    if event_operation() != 3u { return; }
    if i < control.counts_a.x {
        nodes[i].mass_loss.z = nodes[i].mass_loss.w;
    } else if i < control.counts_a.x + control.counts_a.y {
        let sample = i - complementary_offset();
        samples[sample].curl_6_loss.z = samples[sample].curl_6_loss.w;
    }
}

@compute @workgroup_size(1)
fn commit_event() {
    let failure = atomicLoad(&status.candidate);
    if failure != 0u {
        if live_event() {
            control.event_result = vec4<u32>(
                control.event.y, event_operation(), control.event.y, failure);
            control.event.z = accepted_slot();
            atomicStore(&status.candidate, 0u);
        } else {
            atomicMax(&status.latch, failure);
        }
        return;
    }
    let operation = event_operation();
    control.event.z = accepted_slot() ^ 1u;
    control.event.x = control.event.y;
    control.accepted_accounting_a = control.candidate_accounting_a;
    control.accepted_accounting_b = control.candidate_accounting_b;
    if operation == 3u {
        var flags = control.boundary_offsets.w & 8u;
        flags |= control.event.w & 1u;
        flags |= 2u;
        flags |= control.event.w & 4u;
        control.boundary_offsets.w = flags;
        control.runtime_serials.y = control.event.y;
    } else if operation == 1u {
        control.runtime_serials.z = control.event.y;
    } else if operation == 4u {
        control.runtime_serials.w = control.event.y;
    } else if operation == 5u {
        control.runtime_serials.x = control.event.y;
        control.runtime_slots.x ^= 1u;
    }
    control.event_result = vec4<u32>(
        control.event.y, operation, control.event.y, 0u);
    publish_snapshot_metadata();
}

@compute @workgroup_size(1)
fn resident_filter_commit() {
    let failure = atomicLoad(&status.candidate);
    if failure != 0u {
        atomicMax(&status.latch, failure);
        return;
    }
    control.event.z = accepted_slot() ^ 1u;
    control.accepted_accounting_a = control.candidate_accounting_a;
    control.accepted_accounting_b = control.candidate_accounting_b;
    publish_snapshot_metadata();
}

@compute @workgroup_size(128)
fn handoff_finalize(@builtin(global_invocation_id) id: vec3<u32>) {
    let i = id.x;
    if stopped() { return; }
    if i < control.counts_a.x {
        let before = candidate_q(i);
        var next = before;
        var exchange = 0.0;
        if nodes[i].boundary.z != 0u {
            next = nodes[i].mass_loss.x * harmonic_value(nodes[i].prescribed, 0.0);
            exchange = 0.5 * (next * next - before * before) * nodes[i].mass_loss.y;
            set_candidate_q(i, next);
        }
        let next_force = candidate_constitutive_force(i);
        set_candidate_force(i, next_force);
        scratch[i].values.y = exchange;
        if !finite_scalar(next) || !finite_scalar(next_force) {
            reject(STATUS_NON_FINITE);
        }
        inject_at(i);
        return;
    }
    if i < control.counts_a.x + control.counts_a.y {
        let sample = i - control.counts_a.x;
        let value = candidate_b(sample);
        if !all(value >= vec2<f32>(-MAX_FINITE))
            || !all(value <= vec2<f32>(MAX_FINITE)) {
            reject(STATUS_NON_FINITE);
        }
        inject_at(i);
        return;
    }
    if i < control.counts_a.w {
        let auxiliary = i - auxiliary_offset();
        if !finite_scalar(candidate_auxiliary(auxiliary)) {
            reject(STATUS_NON_FINITE);
        }
        inject_at(i);
    }
}

@compute @workgroup_size(128)
fn handoff_reduce_prescribed(@builtin(local_invocation_id) id: vec3<u32>) {
    let local = id.x;
    let participating = !stopped();
    var exchange = 0.0;
    if participating {
        for (var node = local; node < control.counts_a.x; node += WORKGROUP_SIZE) {
            exchange += scratch[node].values.y;
        }
    }
    let total = reduce_boundary_scalar(local, exchange);
    if participating && local == 0u {
        control.candidate_accounting_a.y += total;
    }
}

@compute @workgroup_size(128)
fn handoff_clear_scratch(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x < arrayLength(&scratch) { scratch[id.x].values = vec4<f32>(0.0); }
}

@compute @workgroup_size(1)
fn handoff_commit() {
    var failure = atomicLoad(&status.candidate);
    if !finite_vector(control.candidate_accounting_a)
        || !finite_vector(control.candidate_accounting_b) {
        failure = max(failure, STATUS_NON_FINITE);
    }
    if failure != 0u {
        atomicMax(&status.latch, failure);
        atomicStore(&status.handoff, 0u);
        return;
    }
    control.event.z = accepted_slot() ^ 1u;
    control.accepted_accounting_a = control.candidate_accounting_a;
    control.accepted_accounting_b = control.candidate_accounting_b;
    atomicStore(&status.transaction_1, control.clock_u32.w);
    atomicStore(&status.transaction_2, control.clock_u32.z);
    atomicStore(&status.handoff, 0u);
    publish_snapshot_metadata();
}

@compute @workgroup_size(128)
fn rebase_clock_records(@builtin(global_invocation_id) id: vec3<u32>) {
    let i = id.x;
    if stopped() { return; }
    let elapsed = control.clock_f32.y;
    if i < control.counts_a.x && nodes[i].boundary.z != 0u {
        nodes[i].prescribed.w = reduced_phase(
            nodes[i].prescribed.w + nodes[i].prescribed.z * elapsed);
    }
    if i < control.counts_c.z {
        let root = control.table_offsets.y + 4u * i;
        let base = root + 2u * (control.runtime_slots.x & 1u);
        let kind = tables[base + 1u].data.x;
        var next_anchor = 0.0;
        if kind != 0u {
            next_anchor = source_drive(i, elapsed);
        }
        let phase = reduced_phase(
            table_float(base, 3u) + table_float(base, 2u) * elapsed);
        tables[root].data = tables[base].data;
        tables[root + 2u].data = tables[base].data;
        tables[root].data.w = bitcast<u32>(phase);
        tables[root + 2u].data.w = bitcast<u32>(phase);
        if kind != 0u {
            tables[root + 1u].data = tables[base + 1u].data;
            tables[root + 3u].data = tables[base + 1u].data;
            tables[root + 1u].data.y = bitcast<u32>(next_anchor);
            tables[root + 3u].data.y = bitcast<u32>(next_anchor);
        }
    }
}

@compute @workgroup_size(1)
fn commit_clock_rebase() {
    if stopped() { return; }
    let origin = add_compensated(
        control.clock_origin.x, control.clock_origin.y, control.clock_f32.y);
    let low = control.clock_u32.x + 1u;
    if low == 0u { control.clock_u32.y += 1u; }
    control.clock_u32.x = low;
    control.clock_u32.z = 0u;
    control.clock_origin = vec4<f32>(origin, origin);
    control.clock_f32.y = 0.0;
    control.clock_f32.z = 0.0;
    publish_snapshot_metadata();
}

@compute @workgroup_size(128)
fn start_loss(@builtin(global_invocation_id) id: vec3<u32>) {
    let i = id.x;
    if stopped() { return; }
    if i < scratch_count() {
        scratch[i].values = vec4<f32>(0.0);
    }
    if i >= control.counts_a.w { return; }
    let node_count = control.counts_a.x;
    let sample_count = control.counts_a.y;
    if i < node_count {
        let old = accepted_q(i);
        var owned = old;
        var exchange = 0.0;
        if nodes[i].boundary.z != 0u {
            owned = nodes[i].mass_loss.x
                * harmonic_value(nodes[i].prescribed, control.clock_f32.y);
            exchange = 0.5 * (owned * owned - old * old) * nodes[i].mass_loss.y;
        }
        let next = owned - owned * nodes[i].mass_loss.z;
        set_candidate_q(i, next);
        if use_force_cache() {
            set_candidate_force(i, accepted_force(i));
        }
        scratch[i].values.y = exchange;
        scratch[i].values.z = 0.5 * (owned * owned - next * next) * nodes[i].mass_loss.y;
    } else if i < node_count + sample_count {
        let sample_index = i - node_count;
        let old = accepted_b(sample_index);
        let next = old - old * samples[sample_index].curl_6_loss.z;
        set_candidate_b(sample_index, next);
        scratch[i].values.x = b_energy(sample_index, old) - b_energy(sample_index, next);
    } else {
        let auxiliary = i - auxiliary_offset();
        set_candidate_auxiliary(auxiliary, accepted_auxiliary(auxiliary));
    }
}

fn kick_node(node: u32, second: bool) {
    if stopped() || node >= control.counts_a.x { return; }
    if !second && !has_loss_stages() {
        scratch[node].values = vec4<f32>(0.0);
    }
    let duration = control.evolution.z;
    let source_time = control.clock_f32.y + select(0.0, control.clock_f32.x, second);
    let target_time = control.clock_f32.y
        + select(duration, control.clock_f32.x, second);
    let source = source_rate(node, source_time);
    let held_force = force(node, second);
    let net = source - held_force;
    if nodes[node].boundary.x != NO_INDEX {
        return;
    }
    let old = select(
        accepted_q(node), candidate_q(node), second || has_loss_stages());
    let inverse_mass = nodes[node].mass_loss.y;
    let damping = nodes[node].damping_support.x;
    let ratio = 0.5 * duration * damping * inverse_mass;
    var next = ((1.0 - ratio) * old + duration * net) / (1.0 + ratio);
    if nodes[node].boundary.z != 0u {
        next = nodes[node].mass_loss.x * harmonic_value(nodes[node].prescribed, target_time);
    }
    let midpoint = 0.5 * (old + next) * inverse_mass;
    let source_work = duration * midpoint * source;
    let force_work = duration * midpoint * held_force;
    let boundary_loss = duration * damping * midpoint * midpoint;
    let energy_change = 0.5 * (next * next - old * old) * inverse_mass;
    set_candidate_q(node, next);
    scratch[node].values.x += source_work;
    scratch[node].values.w += boundary_loss;
    if nodes[node].boundary.z != 0u {
        scratch[node].values.y += energy_change - source_work + force_work + boundary_loss;
    }
    if !finite_scalar(next) { reject(STATUS_NON_FINITE); }
    if second { inject_at(node); }
}

@compute @workgroup_size(128)
fn kick_first(@builtin(global_invocation_id) id: vec3<u32>) {
    kick_node(id.x, false);
}

@compute @workgroup_size(128)
fn kick_second(@builtin(global_invocation_id) id: vec3<u32>) {
    kick_node(id.x, true);
}

fn mode_word(mode: u32, word: u32) -> u32 {
    return control.table_offsets.w + mode * MODE_WORDS + word;
}

fn trace_coefficient(mode: u32, trace: u32) -> f32 {
    return packed_boundary_scalar(
        control.boundary_offsets.x,
        mode * control.counts_b.y + trace);
}

fn trace_coefficient_transposed(trace: u32, mode: u32) -> f32 {
    let scalar_count = control.counts_b.y * control.counts_b.z;
    let transposed_offset = control.boundary_offsets.y + (scalar_count + 3u) / 4u;
    return packed_boundary_scalar(
        transposed_offset,
        trace * control.counts_b.z + mode);
}

fn inverse_schur(row: u32, column: u32) -> f32 {
    return packed_boundary_scalar(
        control.boundary_offsets.y,
        row * control.counts_b.y + column);
}

fn reduce_boundary_scalar(local: u32, value: f32) -> f32 {
    reduced_values[local] = value;
    workgroupBarrier();
    var width = WORKGROUP_SIZE / 2u;
    loop {
        if width == 0u { break; }
        if local < width {
            reduced_values[local] += reduced_values[local + width];
        }
        workgroupBarrier();
        width /= 2u;
    }
    return reduced_values[0];
}

fn reduce_boundary_vector(local: u32, value: vec2<f32>) -> vec2<f32> {
    mode_values[local] = value;
    workgroupBarrier();
    var width = WORKGROUP_SIZE / 2u;
    loop {
        if width == 0u { break; }
        if local < width {
            mode_values[local] += mode_values[local + width];
        }
        workgroupBarrier();
        width /= 2u;
    }
    return mode_values[0];
}

fn boundary_prepare(mode: u32, local: u32, second: bool) {
    let participating = !stopped() && mode < control.counts_b.z;
    let trace_count = control.counts_b.y;
    var partial_modal = 0.0;
    if participating {
        for (var trace = local; trace < trace_count; trace += WORKGROUP_SIZE) {
            let trace_word = boundary[control.table_offsets.z + trace].data;
            let node = trace_word.x;
            let current = select(
                accepted_q(node), candidate_q(node), second || has_loss_stages());
            partial_modal += trace_coefficient(mode, trace)
                * current * bitcast<f32>(trace_word.y);
        }
    }
    let modal = reduce_boundary_scalar(local, partial_modal);
    if !participating || local != 0u { return; }
    let mode_header = boundary[mode_word(mode, 0u)].data;
    if !second && !has_loss_stages() {
        scratch[mode_accounting_offset() + mode].values = vec4<f32>(0.0);
    }
    var memory = 0.0;
    var solved = vec3<f32>(0.0);
    var coupling = 0.0;
    if mode_header.z != 0u {
        let auxiliary = mode_header.y;
        let old_z = vec3<f32>(
            select(accepted_auxiliary(auxiliary), candidate_auxiliary(auxiliary), second || has_loss_stages()),
            select(accepted_auxiliary(auxiliary + 1u), candidate_auxiliary(auxiliary + 1u), second || has_loss_stages()),
            select(accepted_auxiliary(auxiliary + 2u), candidate_auxiliary(auxiliary + 2u), second || has_loss_stages()));
        let residue = vec3<f32>(
            boundary_float(mode_word(mode, 1u), 0u),
            boundary_float(mode_word(mode, 1u), 1u),
            boundary_float(mode_word(mode, 1u), 2u));
        memory = dot(residue, old_z);
        let input_gain = vec3<f32>(
            boundary_float(mode_word(mode, 2u), 0u),
            boundary_float(mode_word(mode, 2u), 1u),
            boundary_float(mode_word(mode, 2u), 2u));
        var right_z = old_z + control.evolution.w * input_gain * modal;
        for (var row = 0u; row < 3u; row += 1u) {
            let generator = vec3<f32>(
                boundary_float(mode_word(mode, 3u + row), 0u),
                boundary_float(mode_word(mode, 3u + row), 1u),
                boundary_float(mode_word(mode, 3u + row), 2u));
            right_z[row] += control.evolution.w * dot(generator, old_z);
        }
        for (var row = 0u; row < 3u; row += 1u) {
            let inverse = vec3<f32>(
                boundary_float(mode_word(mode, 6u + row), 0u),
                boundary_float(mode_word(mode, 6u + row), 1u),
                boundary_float(mode_word(mode, 6u + row), 2u));
            solved[row] = dot(inverse, right_z);
        }
        let aqz = vec3<f32>(
            boundary_float(mode_word(mode, 9u), 0u),
            boundary_float(mode_word(mode, 9u), 1u),
            boundary_float(mode_word(mode, 9u), 2u));
        coupling = dot(aqz, solved);
    }
    scratch[mode_temporary_offset() + mode].values = vec4<f32>(solved, coupling);
    scratch[mode_modal_offset() + mode].values = vec4<f32>(modal, memory, 0.0, 0.0);
}

fn boundary_reduce(trace: u32, local: u32, second: bool) {
    let participating = !stopped() && trace < control.counts_b.y;
    var partial = vec2<f32>(0.0);
    if participating {
        for (var mode = local; mode < control.counts_b.z; mode += WORKGROUP_SIZE) {
            let modal_memory = scratch[mode_modal_offset() + mode].values.xy;
            let coefficient = trace_coefficient_transposed(trace, mode);
            partial.x += coefficient * (modal_memory.x + modal_memory.y);
            partial.y += coefficient * scratch[mode_temporary_offset() + mode].values.w;
        }
    }
    let coupling = reduce_boundary_vector(local, partial);
    if !participating || local != 0u { return; }
    let trace_word_index = control.table_offsets.z + trace;
    let trace_word = boundary[trace_word_index].data;
    let node = trace_word.x;
    let inverse_mass = bitcast<f32>(trace_word.y);
    let damping = bitcast<f32>(trace_word.z);
    let old = select(
        accepted_q(node), candidate_q(node), second || has_loss_stages());
    let derivative = -damping * inverse_mass * old - coupling.x;
    let source_time = control.clock_f32.y
        + select(0.0, control.clock_f32.x, second);
    let source = source_rate(node, source_time);
    let held_force = force(node, second);
    var reduced = old + control.evolution.w * derivative
        + control.evolution.z * (source - held_force);
    if nodes[node].boundary.z != 0u {
        let target_time = control.clock_f32.y
            + select(control.evolution.z, control.clock_f32.x, second);
        reduced = nodes[node].mass_loss.x
            * harmonic_value(nodes[node].prescribed, target_time);
    } else {
        reduced -= coupling.y;
    }
    boundary[trace_word_index].data.w = bitcast<u32>(reduced);
}

fn boundary_solve_row(row: u32, local: u32) {
    let participating = !stopped() && row < control.counts_b.y;
    var partial = 0.0;
    if participating {
        for (var column = local; column < control.counts_b.y; column += WORKGROUP_SIZE) {
            let reduced = bitcast<f32>(boundary[control.table_offsets.z + column].data.w);
            partial += inverse_schur(row, column) * reduced;
        }
    }
    let result = reduce_boundary_scalar(local, partial);
    if participating && local == 0u {
        scratch[trace_solution_offset() + row].values.x = result;
    }
}

fn boundary_finalize(i: u32, local: u32, second: bool) {
    let trace_count = control.counts_b.y;
    let mode_count = control.counts_b.z;
    let participating = !stopped() && i < max(trace_count, mode_count);
    let duration = control.evolution.z;
    var partial_modal = 0.0;
    if participating && i < mode_count {
        for (var trace = local; trace < trace_count; trace += WORKGROUP_SIZE) {
            let trace_word = boundary[control.table_offsets.z + trace].data;
            partial_modal += trace_coefficient(i, trace)
                * scratch[trace_solution_offset() + trace].values.x
                * bitcast<f32>(trace_word.y);
        }
    }
    let new_modal = reduce_boundary_scalar(local, partial_modal);
    if !participating || local != 0u { return; }
    if i < trace_count {
        let trace = i;
        let trace_word = boundary[control.table_offsets.z + trace].data;
        let node = trace_word.x;
        let inverse_mass = bitcast<f32>(trace_word.y);
        let damping = bitcast<f32>(trace_word.z);
        let old = select(
            accepted_q(node), candidate_q(node), second || has_loss_stages());
        let next = scratch[trace_solution_offset() + trace].values.x;
        let source_time = control.clock_f32.y
            + select(0.0, control.clock_f32.x, second);
        let source = source_rate(node, source_time);
        let held_force = force(node, second);
        let midpoint = 0.5 * (old + next) * inverse_mass;
        let source_work = duration * midpoint * source;
        let force_work = duration * midpoint * held_force;
        let boundary_loss = duration * damping * midpoint * midpoint;
        let energy_change = 0.5 * (next * next - old * old) * inverse_mass;
        set_candidate_q(node, next);
        scratch[node].values.x += source_work;
        scratch[node].values.w += boundary_loss;
        if has_prescribed_trace() {
            scratch[node].values.y += energy_change - source_work + force_work + boundary_loss;
        }
        if !finite_scalar(next) { reject(STATUS_NON_FINITE); }
        if second { inject_at(node); }
    }

    if i < mode_count {
        let mode = i;
        let mode_header = boundary[mode_word(mode, 0u)].data;
        var auxiliary_change = 0.0;
        var midpoint_memory = 0.0;
        if mode_header.z != 0u {
            let auxiliary = mode_header.y;
            let old_z = vec3<f32>(
                select(accepted_auxiliary(auxiliary), candidate_auxiliary(auxiliary), second || has_loss_stages()),
                select(accepted_auxiliary(auxiliary + 1u), candidate_auxiliary(auxiliary + 1u), second || has_loss_stages()),
                select(accepted_auxiliary(auxiliary + 2u), candidate_auxiliary(auxiliary + 2u), second || has_loss_stages()));
            var new_z = scratch[mode_temporary_offset() + mode].values.xyz;
            let solved_column = vec3<f32>(
                boundary_float(mode_word(mode, 10u), 0u),
                boundary_float(mode_word(mode, 10u), 1u),
                boundary_float(mode_word(mode, 10u), 2u));
            new_z = vec3<f32>(
                fma(-solved_column.x, new_modal, new_z.x),
                fma(-solved_column.y, new_modal, new_z.y),
                fma(-solved_column.z, new_modal, new_z.z));
            set_candidate_auxiliary(auxiliary, new_z.x);
            set_candidate_auxiliary(auxiliary + 1u, new_z.y);
            set_candidate_auxiliary(auxiliary + 2u, new_z.z);
            auxiliary_change = 0.5 * (dot(new_z, new_z) - dot(old_z, old_z));
            let loss_coefficient = vec3<f32>(
                boundary_float(mode_word(mode, 11u), 0u),
                boundary_float(mode_word(mode, 11u), 1u),
                boundary_float(mode_word(mode, 11u), 2u));
            midpoint_memory = dot(loss_coefficient, 0.5 * (old_z + new_z));
            if !all(new_z >= vec3<f32>(-MAX_FINITE))
                || !all(new_z <= vec3<f32>(MAX_FINITE)) {
                reject(STATUS_NON_FINITE);
            }
            if second {
                inject_at(auxiliary_offset() + auxiliary);
                inject_at(auxiliary_offset() + auxiliary + 1u);
                inject_at(auxiliary_offset() + auxiliary + 2u);
            }
        }
        let old_modal = scratch[mode_modal_offset() + mode].values.x;
        let midpoint_field = 0.5 * (old_modal + new_modal);
        let outgoing_loss = duration
            * (midpoint_field + midpoint_memory) * (midpoint_field + midpoint_memory);
        scratch[mode_accounting_offset() + mode].values.x += auxiliary_change;
        scratch[mode_accounting_offset() + mode].values.w += outgoing_loss;
    }
}

@compute @workgroup_size(128)
fn boundary_prepare_first(
    @builtin(workgroup_id) group: vec3<u32>,
    @builtin(local_invocation_id) local: vec3<u32>,
) {
    boundary_prepare(group.x, local.x, false);
}

@compute @workgroup_size(128)
fn boundary_reduce_first(
    @builtin(workgroup_id) group: vec3<u32>,
    @builtin(local_invocation_id) local: vec3<u32>,
) {
    boundary_reduce(group.x, local.x, false);
}

@compute @workgroup_size(128)
fn boundary_solve(
    @builtin(workgroup_id) group: vec3<u32>,
    @builtin(local_invocation_id) local: vec3<u32>,
) {
    boundary_solve_row(group.x, local.x);
}

@compute @workgroup_size(128)
fn boundary_finalize_first(
    @builtin(workgroup_id) group: vec3<u32>,
    @builtin(local_invocation_id) local: vec3<u32>,
) {
    boundary_finalize(group.x, local.x, false);
}

@compute @workgroup_size(128)
fn boundary_prepare_second(
    @builtin(workgroup_id) group: vec3<u32>,
    @builtin(local_invocation_id) local: vec3<u32>,
) {
    boundary_prepare(group.x, local.x, true);
}

@compute @workgroup_size(128)
fn boundary_reduce_second(
    @builtin(workgroup_id) group: vec3<u32>,
    @builtin(local_invocation_id) local: vec3<u32>,
) {
    boundary_reduce(group.x, local.x, true);
}

@compute @workgroup_size(128)
fn boundary_finalize_second(
    @builtin(workgroup_id) group: vec3<u32>,
    @builtin(local_invocation_id) local: vec3<u32>,
) {
    boundary_finalize(group.x, local.x, true);
}

@compute @workgroup_size(128)
fn drift(@builtin(global_invocation_id) id: vec3<u32>) {
    let i = id.x;
    if stopped() { return; }
    if i < control.counts_a.x && use_force_cache() {
        let next_force = accepted_force(i)
            + control.clock_f32.x * stiffness_force(i);
        set_candidate_force(i, next_force);
        if !finite_scalar(next_force) { reject(STATUS_NON_FINITE); }
    }
    if i < control.counts_a.y {
        if !has_loss_stages() {
            scratch[complementary_offset() + i].values = vec4<f32>(0.0);
        }
        let sample = samples[i];
        let reference = candidate_q(sample.nodes_a.x) * nodes[sample.nodes_a.x].mass_loss.y;
        var curl = vec2<f32>(0.0);
        for (var local = 1u; local < 7u; local += 1u) {
            let node = sample_node(sample, local);
            let field = candidate_q(node) * nodes[node].mass_loss.y;
            curl += sample_curl(sample, local) * (field - reference);
        }
        let old = select(accepted_b(i), candidate_b(i), has_loss_stages());
        let next = old + control.evolution.y * control.clock_f32.x * curl;
        set_candidate_b(i, next);
        if !all(next >= vec2<f32>(-MAX_FINITE))
            || !all(next <= vec2<f32>(MAX_FINITE)) {
            reject(STATUS_NON_FINITE);
        }
        inject_at(complementary_offset() + i);
    }
    if i < control.counts_b.x {
        let record = tables[control.table_offsets.x + i].data;
        let left = record.x;
        let right = record.y;
        let auxiliary_word = record.z;
        let difference = candidate_q(left) * nodes[left].mass_loss.y
            - candidate_q(right) * nodes[right].mass_loss.y;
        let auxiliary = auxiliary_word - auxiliary_offset();
        let next = accepted_auxiliary(auxiliary) + control.clock_f32.x * difference;
        set_candidate_auxiliary(auxiliary, next);
        if !finite_scalar(next) { reject(STATUS_NON_FINITE); }
        inject_at(auxiliary_word);
    }
}

@compute @workgroup_size(128)
fn finish_loss_validate(@builtin(global_invocation_id) id: vec3<u32>) {
    let i = id.x;
    if stopped() { return; }
    if i >= control.counts_a.w { return; }
    let node_count = control.counts_a.x;
    let sample_count = control.counts_a.y;
    if i < node_count {
        let before = candidate_q(i);
        var next = before - before * nodes[i].mass_loss.z;
        scratch[i].values.z += 0.5 * (before * before - next * next) * nodes[i].mass_loss.y;
        if nodes[i].boundary.z != 0u {
            let owned = nodes[i].mass_loss.x
                * harmonic_value(nodes[i].prescribed, control.clock_f32.y + control.clock_f32.x);
            scratch[i].values.y += 0.5 * (owned * owned - next * next) * nodes[i].mass_loss.y;
            next = owned;
        }
        set_candidate_q(i, next);
        if !finite_scalar(next) { reject(STATUS_NON_FINITE); }
    } else if i < node_count + sample_count {
        let sample_index = i - node_count;
        let before = candidate_b(sample_index);
        let next = before - before * samples[sample_index].curl_6_loss.z;
        scratch[i].values.x += b_energy(sample_index, before) - b_energy(sample_index, next);
        set_candidate_b(sample_index, next);
        if !all(next >= vec2<f32>(-MAX_FINITE))
            || !all(next <= vec2<f32>(MAX_FINITE)) {
            reject(STATUS_NON_FINITE);
        }
    } else if !finite_scalar(state[i].values.y) {
        reject(STATUS_NON_FINITE);
    }
    let injection = atomicLoad(&status.injection);
    if injection != 0u && i == atomicLoad(&status.injection_index) {
        reject(injection);
    }
}

fn reduce_accounting_half(local: u32, value: vec4<f32>) -> vec4<f32> {
    mode_values[local] = value.xy;
    reduced_values[local] = value.z;
    solved_values[local] = value.w;
    workgroupBarrier();
    var width = WORKGROUP_SIZE / 2u;
    loop {
        if width == 0u { break; }
        if local < width {
            mode_values[local] += mode_values[local + width];
            reduced_values[local] += reduced_values[local + width];
            solved_values[local] += solved_values[local + width];
        }
        workgroupBarrier();
        width /= 2u;
    }
    return vec4<f32>(mode_values[0], reduced_values[0], solved_values[0]);
}

@compute @workgroup_size(128)
fn stage_accounting(@builtin(global_invocation_id) id: vec3<u32>) {
    let i = id.x;
    if stopped() || i >= accounting_item_count() { return; }
    var contribution = vec4<f32>(0.0);
    if i < control.counts_a.x {
        contribution = scratch[i].values;
    } else if i < control.counts_a.x + control.counts_a.y {
        let sample = i - control.counts_a.x;
        contribution.x = scratch[complementary_offset() + sample].values.x;
    } else {
        let mode = i - control.counts_a.x - control.counts_a.y;
        contribution = scratch[mode_accounting_offset() + mode].values;
    }
    let accepted = accounting_bank_offset(accepted_slot()) + i;
    let candidate = accounting_bank_offset(accepted_slot() ^ 1u) + i;
    scratch[candidate].values = scratch[accepted].values + contribution;
}

@compute @workgroup_size(128)
fn reduce_accounting(@builtin(local_invocation_id) id: vec3<u32>) {
    let local = id.x;
    var accounting_a = vec4<f32>(0.0);
    for (var node = local; node < control.counts_a.x; node += WORKGROUP_SIZE) {
        let item = scratch[accounting_bank_offset(accepted_slot()) + node].values;
        accounting_a.x += item.x;
        if nodes[node].boundary.x == NO_INDEX || has_prescribed_trace() {
            accounting_a.y += item.y;
        }
        accounting_a.z += item.z;
    }
    for (var sample = local; sample < control.counts_a.y; sample += WORKGROUP_SIZE) {
        let item = control.counts_a.x + sample;
        accounting_a.w += scratch[accounting_bank_offset(accepted_slot()) + item].values.x;
    }
    for (var mode = local; mode < control.counts_b.z; mode += WORKGROUP_SIZE) {
        if has_prescribed_trace() {
            let item_index = control.counts_a.x + control.counts_a.y + mode;
            let item = scratch[accounting_bank_offset(accepted_slot()) + item_index].values;
            accounting_a.y += item.x + item.w;
        }
    }
    let total_a = reduce_accounting_half(local, accounting_a);
    if local == 0u {
        control.accepted_accounting_a = control.candidate_accounting_a + total_a;
    }
    workgroupBarrier();

    var accounting_b = vec4<f32>(0.0);
    for (var node = local; node < control.counts_a.x; node += WORKGROUP_SIZE) {
        accounting_b.x += scratch[accounting_bank_offset(accepted_slot()) + node].values.w;
    }
    for (var mode = local; mode < control.counts_b.z; mode += WORKGROUP_SIZE) {
        let item_index = control.counts_a.x + control.counts_a.y + mode;
        accounting_b.x += scratch[accounting_bank_offset(accepted_slot()) + item_index].values.w;
    }
    let total_b = reduce_accounting_half(local, accounting_b);
    if local == 0u {
        control.accepted_accounting_b = control.candidate_accounting_b + total_b;
    }
}

@compute @workgroup_size(128)
fn commit_step(@builtin(global_invocation_id) id: vec3<u32>) {
    let i = id.x;
    if i != 0u { return; }
    let failure = atomicLoad(&status.candidate);
    if failure != 0u {
        atomicMax(&status.latch, failure);
        return;
    }
    control.event.z = accepted_slot() ^ 1u;
    control.clock_u32.z += 1u;
    control.clock_u32.w += 1u;
    control.clock_f32.y += control.clock_f32.x;
    control.clock_f32.z = control.clock_f32.y;
    control.event.x = control.event.y;
    publish_snapshot_metadata();
}
