// Runtime/clock half of canonical generation handoff.
const TRANSFER_LAYOUT_VERSION: u32 = 2u;
const NO_INDEX: u32 = 0xffffffffu;
const DRIVE_TARGET_PARAMETERS: u32 = 0x80000000u;
const DRIVE_INDEX_MASK: u32 = 0x7fffffffu;
const STATUS_LAYOUT: u32 = 1u;
// Leave serialization headroom below f32::MAX: Naga's decimal WGSL writer
// rounds the exact maximum upward, which Chrome correctly rejects.
const MAX_FINITE: f32 = 3.0e+38;

struct Control {
    counts_a: vec4<u32>, counts_b: vec4<u32>, counts_c: vec4<u32>,
    table_offsets: vec4<u32>, boundary_offsets: vec4<u32>, clock_u32: vec4<u32>,
    clock_f32: vec4<f32>, clock_origin: vec4<f32>, event: vec4<u32>,
    event_result: vec4<u32>, runtime_serials: vec4<u32>,
    runtime_slots: vec4<u32>,
    accepted_accounting_a: vec4<f32>, accepted_accounting_b: vec4<f32>,
    candidate_accounting_a: vec4<f32>, candidate_accounting_b: vec4<f32>,
    evolution: vec4<f32>,
}
struct Status {
    candidate: atomic<u32>, latch: atomic<u32>,
    injection: atomic<u32>, injection_index: atomic<u32>,
    handoff: atomic<u32>, transaction_1: atomic<u32>,
    transaction_2: atomic<u32>, transaction_3: atomic<u32>,
}
struct Node {
    mass_loss: vec4<f32>, ranges: vec4<u32>, boundary: vec4<u32>,
    prescribed: vec4<f32>, damping_support: vec4<f32>, stiffness: vec4<u32>,
}
struct TableWord { data: vec4<u32> }
struct TransferWord { data: vec4<u32> }

@group(0) @binding(0) var<storage, read_write> old_control: Control;
@group(0) @binding(1) var<storage, read_write> old_nodes: array<Node>;
@group(0) @binding(2) var<storage, read_write> old_tables: array<TableWord>;
@group(0) @binding(3) var<storage, read_write> new_control: Control;
@group(0) @binding(4) var<storage, read_write> new_status: Status;
@group(0) @binding(5) var<storage, read_write> new_nodes: array<Node>;
@group(0) @binding(6) var<storage, read_write> new_tables: array<TableWord>;
@group(0) @binding(7) var<storage, read_write> transfer: array<TransferWord>;

fn reject(reason: u32) { atomicMax(&new_status.candidate, reason); }
fn reduced_phase(value: f32) -> f32 { return atan2(sin(value), cos(value)); }
fn add_compensated(high: f32, low: f32, increment: f32) -> vec2<f32> {
    let sum = high + increment;
    let residual = (high - sum) + increment + low;
    let next_high = sum + residual;
    return vec2<f32>(next_high, (sum - next_high) + residual);
}
fn sinc(value: f32) -> f32 {
    if abs(value) < 0.0005 {
        let square = value * value;
        return 1.0 - square / 6.0 + square * square / 120.0;
    }
    return sin(value) / value;
}
fn old_float(word: u32, lane: u32) -> f32 {
    return bitcast<f32>(old_tables[word].data[lane]);
}
fn new_float(word: u32, lane: u32) -> f32 {
    return bitcast<f32>(new_tables[word].data[lane]);
}
fn mapped_index(offset: u32, index: u32) -> u32 {
    return transfer[offset + index / 4u].data[index % 4u];
}
fn write_material_runtime(
    root: u32,
    phase: vec4<f32>,
    switch_state: vec4<f32>,
    frequency: vec4<f32>,
) {
    let phase_bits = bitcast<vec4<u32>>(phase);
    let switch_bits = bitcast<vec4<u32>>(switch_state);
    let frequency_bits = bitcast<vec4<u32>>(frequency);
    for (var slot = 0u; slot < 2u; slot += 1u) {
        let base = root + 3u * slot;
        new_tables[base].data = phase_bits;
        new_tables[base + 1u].data = switch_bits;
        new_tables[base + 2u].data = frequency_bits;
    }
}
fn old_drive(base: u32, elapsed: f32) -> f32 {
    let parameters = vec4<f32>(
        old_float(base, 0u), old_float(base, 1u),
        old_float(base, 2u), old_float(base, 3u));
    let runtime = old_tables[base + 1u].data;
    if runtime.x == 0u {
        return parameters.x + parameters.y * sin(parameters.w + parameters.z * elapsed);
    }
    let half_phase = 0.5 * parameters.z * elapsed;
    return bitcast<f32>(runtime.y) + parameters.x * elapsed
        + parameters.y * elapsed * sinc(half_phase)
            * sin(parameters.w + half_phase);
}
fn write_drive(base: u32, parameters: vec4<f32>, runtime: vec4<u32>) {
    let parameter_bits = bitcast<vec4<u32>>(parameters);
    new_tables[base].data = parameter_bits;
    new_tables[base + 1u].data = runtime;
    new_tables[base + 2u].data = parameter_bits;
    new_tables[base + 3u].data = runtime;
}
fn retain_drive(old_base: u32, new_base: u32, elapsed: f32) {
    var parameters = vec4<f32>(
        old_float(old_base, 0u), old_float(old_base, 1u),
        old_float(old_base, 2u), old_float(old_base, 3u));
    var runtime = old_tables[old_base + 1u].data;
    if runtime.x != 0u {
        let half_phase = 0.5 * parameters.z * elapsed;
        let next_anchor = bitcast<f32>(runtime.y) + parameters.x * elapsed
            + parameters.y * elapsed * sinc(half_phase)
                * sin(parameters.w + half_phase);
        runtime.y = bitcast<u32>(next_anchor);
    }
    parameters.w = reduced_phase(parameters.w + parameters.z * elapsed);
    write_drive(new_base, parameters, runtime);
}
fn replace_drive(old_base: u32, new_base: u32, old_elapsed: f32) {
    var parameters = vec4<f32>(
        new_float(new_base, 0u), new_float(new_base, 1u),
        new_float(new_base, 2u), new_float(new_base, 3u));
    var runtime = new_tables[new_base + 1u].data;
    let old_parameters = vec4<f32>(
        old_float(old_base, 0u), old_float(old_base, 1u),
        old_float(old_base, 2u), old_float(old_base, 3u));
    parameters.w = reduced_phase(old_parameters.w + old_parameters.z * old_elapsed);
    if runtime.x != 0u {
        runtime.y = bitcast<u32>(old_drive(old_base, old_elapsed));
    }
    write_drive(new_base, parameters, runtime);
}
fn start_drive(base: u32, absolute_elapsed: f32) {
    var parameters = vec4<f32>(
        new_float(base, 0u), new_float(base, 1u),
        new_float(base, 2u), new_float(base, 3u));
    let runtime = new_tables[base + 1u].data;
    // A genuinely new legacy drive starts with zero integrated rate at the
    // handoff instant; only its acceleration phase advances to that instant.
    parameters.w = reduced_phase(parameters.w + parameters.z * absolute_elapsed);
    write_drive(base, parameters, runtime);
}

@compute @workgroup_size(128)
fn transfer_runtime(@builtin(global_invocation_id) id: vec3<u32>) {
    let i = id.x;
    if transfer[0].data.x != TRANSFER_LAYOUT_VERSION { reject(STATUS_LAYOUT); return; }
    let target_nodes = transfer[1].data.y;
    let source_drives = transfer[5].data.z;
    let target_drives = transfer[5].data.w;
    let current_origin = add_compensated(
        old_control.clock_origin.x, old_control.clock_origin.y, old_control.clock_f32.y);
    let preparation_delta = (current_origin.x - new_control.clock_origin.x)
        + (current_origin.y - new_control.clock_origin.y);
    if i < target_nodes && new_nodes[i].boundary.z != 0u {
        let mapping = mapped_index(transfer[4].data.z, i);
        if mapping != NO_INDEX {
            let source = mapping & DRIVE_INDEX_MASK;
            let old_signal = old_nodes[source].prescribed;
            if (mapping & DRIVE_TARGET_PARAMETERS) != 0u {
                new_nodes[i].prescribed.w = reduced_phase(
                    old_signal.w + old_signal.z * old_control.clock_f32.y);
            } else {
                var signal = old_signal;
                signal.w = reduced_phase(signal.w + signal.z * old_control.clock_f32.y);
                new_nodes[i].prescribed = signal;
            }
        } else {
            new_nodes[i].prescribed.w = reduced_phase(
                new_nodes[i].prescribed.w + new_nodes[i].prescribed.z * preparation_delta);
        }
    }
    if i < target_drives {
        let mapping = mapped_index(transfer[4].data.w, i);
        let new_base = new_control.table_offsets.y + 4u * i;
        if mapping != NO_INDEX {
            let source = mapping & DRIVE_INDEX_MASK;
            let old_base = old_control.table_offsets.y + 4u * source;
            let slot = 2u * (old_control.runtime_slots.x & 1u);
            if (mapping & DRIVE_TARGET_PARAMETERS) != 0u {
                replace_drive(old_base + slot, new_base, old_control.clock_f32.y);
            } else {
                retain_drive(old_base + slot, new_base, old_control.clock_f32.y);
            }
        } else {
            start_drive(new_base, preparation_delta);
        }
    }
    let material_header = transfer[8].data;
    let target_materials = material_header.z;
    if material_header.w != 0u && i < target_materials {
        let mapping = transfer[material_header.x + i].data;
        let new_header = new_tables[new_control.runtime_slots.z].data;
        let new_root = new_header.w + 6u * i;
        let target_phase = bitcast<vec4<f32>>(new_tables[new_root].data);
        var phase = vec4<f32>(0.0);
        var switch_state = bitcast<vec4<f32>>(new_tables[new_root + 1u].data);
        let frequency = bitcast<vec4<f32>>(new_tables[new_root + 2u].data);
        if mapping.x != NO_INDEX {
            let old_header = old_tables[old_control.runtime_slots.z].data;
            let old_slot = old_control.runtime_slots.y & 1u;
            let old_root = old_header.w + 6u * mapping.x + 3u * old_slot;
            let old_phase = bitcast<vec4<f32>>(old_tables[old_root].data);
            let old_switch = bitcast<vec4<f32>>(old_tables[old_root + 1u].data);
            let old_frequency = bitcast<vec4<f32>>(old_tables[old_root + 2u].data);
            for (var lane = 0u; lane < 4u; lane += 1u) {
                phase[lane] = select(
                    reduced_phase(target_phase[lane] + frequency[lane] * preparation_delta),
                    reduced_phase(old_phase[lane] + old_frequency[lane] * old_control.clock_f32.y),
                    (mapping.y & (1u << lane)) != 0u);
            }
            switch_state = vec4<f32>(
                old_switch.x,
                old_switch.y,
                old_switch.z - old_control.clock_f32.y,
                old_switch.w);
        } else {
            for (var lane = 0u; lane < 4u; lane += 1u) {
                phase[lane] = reduced_phase(
                    target_phase[lane] + frequency[lane] * preparation_delta);
            }
            switch_state.z -= preparation_delta;
        }
        write_material_runtime(new_root, phase, switch_state, frequency);
    }
    if i != 0u { return; }
    let epoch_low = old_control.clock_u32.x + 1u;
    let epoch_high = old_control.clock_u32.y + select(0u, 1u, epoch_low == 0u);
    if epoch_low == 0u && epoch_high == 0u { reject(STATUS_LAYOUT); return; }
    new_control.clock_u32 = vec4<u32>(
        epoch_low, epoch_high, 0u, old_control.clock_u32.w);
    new_control.clock_f32.y = 0.0;
    new_control.clock_f32.z = 0.0;
    new_control.clock_origin = vec4<f32>(current_origin, current_origin);
    new_control.event.x = old_control.event.x;
    new_control.event.y = old_control.event.x;
    new_control.event.z = 0u;
    new_control.event_result = old_control.event_result;
    let requested_serials = transfer[6].data;
    new_control.runtime_serials = vec4<u32>(
        select(old_control.runtime_serials.x, requested_serials.x, requested_serials.x != 0u),
        select(old_control.runtime_serials.y, requested_serials.y, requested_serials.y != 0u),
        select(old_control.runtime_serials.z, requested_serials.z, requested_serials.z != 0u),
        select(old_control.runtime_serials.w, requested_serials.w, requested_serials.w != 0u));
    new_control.accepted_accounting_a = old_control.accepted_accounting_a;
    new_control.accepted_accounting_b = old_control.accepted_accounting_b;
    new_control.candidate_accounting_a = old_control.accepted_accounting_a;
    new_control.candidate_accounting_b = old_control.accepted_accounting_b;
    if source_drives > old_control.counts_c.z
        || (material_header.w != 0u
            && (material_header.y == 0u || material_header.z == 0u
                || old_control.runtime_slots.w == 0u
                || new_control.runtime_slots.w == 0u))
        || !all(current_origin >= vec2<f32>(-MAX_FINITE))
        || !all(current_origin <= vec2<f32>(MAX_FINITE)) {
        reject(STATUS_LAYOUT);
    }
}
