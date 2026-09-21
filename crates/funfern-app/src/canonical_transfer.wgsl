// Latest-state canonical generation transfer. Rust transfer layout version 2.
const TRANSFER_LAYOUT_VERSION: u32 = 2u;
const WORKGROUP_SIZE: u32 = 128u;
const PRIMARY_WORDS: u32 = 4u;
const VECTOR_WORDS: u32 = 4u;
const GAP_WORDS: u32 = 3u;
const NO_INDEX: u32 = 0xffffffffu;
const STATUS_LAYOUT: u32 = 1u;
const STATUS_TIMESTEP: u32 = 2u;
const STATUS_NON_FINITE: u32 = 4u;
const HANDOFF_RECEIPT_MAGIC: u32 = 0x48414e44u;
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
    handoff: atomic<u32>, transaction_1: atomic<u32>,
    transaction_2: atomic<u32>, transaction_3: atomic<u32>,
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
struct TransferWord { data: vec4<u32> }
struct ScratchWord { values: vec4<f32> }

@group(0) @binding(0) var<storage, read_write> old_control: Control;
@group(0) @binding(1) var<storage, read_write> old_state: array<StateWord>;
@group(0) @binding(2) var<storage, read_write> new_control: Control;
@group(0) @binding(3) var<storage, read_write> new_status: Status;
@group(0) @binding(4) var<storage, read_write> new_state: array<StateWord>;
@group(0) @binding(5) var<storage, read_write> new_nodes: array<Node>;
@group(0) @binding(6) var<storage, read_write> transfer: array<TransferWord>;
@group(0) @binding(7) var<storage, read_write> scratch: array<ScratchWord>;

var<workgroup> reduced: array<vec4<f32>, 128>;

fn header(index: u32) -> vec4<u32> { return transfer[index].data; }
fn source_node_count() -> u32 { return header(0u).y; }
fn source_sample_count() -> u32 { return header(0u).z; }
fn source_gap_count() -> u32 { return header(0u).w; }
fn source_outgoing_count() -> u32 { return header(1u).x; }
fn target_node_count() -> u32 { return header(1u).y; }
fn target_sample_count() -> u32 { return header(1u).z; }
fn target_gap_count() -> u32 { return header(1u).w; }
fn target_outgoing_count() -> u32 { return header(2u).x; }
fn target_component_count() -> u32 { return header(2u).y; }
fn source_component_count() -> u32 { return header(2u).z; }
fn old_slot() -> u32 { return old_control.event.z & 1u; }
fn new_slot() -> u32 { return new_control.event.z & 1u; }
fn source_auxiliary_offset() -> u32 { return source_node_count() + source_sample_count(); }
fn target_auxiliary_offset() -> u32 { return target_node_count() + target_sample_count(); }

fn reject(reason: u32) { atomicMax(&new_status.candidate, reason); }
fn stopped() -> bool {
    return atomicLoad(&new_status.candidate) != 0u
        || atomicLoad(&new_status.latch) != 0u;
}
fn finite_scalar(value: f32) -> bool {
    return value >= -MAX_FINITE && value <= MAX_FINITE;
}
fn transfer_float(word: u32, lane: u32) -> f32 {
    return bitcast<f32>(transfer[word].data[lane]);
}
fn packed_transfer_float(offset: u32, index: u32) -> f32 {
    return transfer_float(offset + index / 4u, index % 4u);
}
fn packed_transfer_u32(offset: u32, index: u32) -> u32 {
    return transfer[offset + index / 4u].data[index % 4u];
}
fn old_q(node: u32) -> f32 {
    return select(old_state[node].values.x, old_state[node].values.y, old_slot() != 0u);
}
fn old_b(sample: u32) -> vec2<f32> {
    let word = old_state[source_node_count() + sample].values;
    return select(word.xy, word.zw, old_slot() != 0u);
}
fn old_auxiliary(index: u32) -> f32 {
    let word = old_state[source_auxiliary_offset() + index].values;
    return select(word.x, word.y, old_slot() != 0u);
}
fn candidate_q(node: u32) -> f32 {
    return select(new_state[node].values.y, new_state[node].values.x, new_slot() != 0u);
}
fn set_candidate_q(node: u32, value: f32) {
    if new_slot() == 0u { new_state[node].values.y = value; }
    else { new_state[node].values.x = value; }
}
fn set_candidate_b(sample: u32, value: vec2<f32>) {
    let word = target_node_count() + sample;
    if new_slot() == 0u {
        new_state[word].values.z = value.x;
        new_state[word].values.w = value.y;
    } else {
        new_state[word].values.x = value.x;
        new_state[word].values.y = value.y;
    }
}
fn candidate_auxiliary(index: u32) -> f32 {
    let word = new_state[target_auxiliary_offset() + index].values;
    return select(word.y, word.x, new_slot() != 0u);
}
fn set_candidate_auxiliary(index: u32, value: f32) {
    let word = target_auxiliary_offset() + index;
    if new_slot() == 0u { new_state[word].values.y = value; }
    else { new_state[word].values.x = value; }
}
fn reduce_four(local: u32, value: vec4<f32>) -> vec4<f32> {
    reduced[local] = value;
    workgroupBarrier();
    var width = WORKGROUP_SIZE / 2u;
    loop {
        if width == 0u { break; }
        if local < width { reduced[local] += reduced[local + width]; }
        workgroupBarrier();
        width /= 2u;
    }
    return reduced[0];
}
fn component_scratch(component: u32) -> u32 {
    return arrayLength(&scratch) - 1u - component;
}
fn density_scratch() -> u32 {
    return arrayLength(&scratch) - 1u - target_component_count();
}
fn primary_identity() -> bool { return header(7u).y != 0u; }
fn vector_identity() -> bool { return header(7u).z != 0u; }
fn outgoing_identity() -> bool { return header(7u).w != 0u; }

@compute @workgroup_size(128)
fn transfer_primary(@builtin(global_invocation_id) id: vec3<u32>) {
    let target_index = id.x;
    if stopped() || target_index >= target_node_count() { return; }
    if header(0u).x != TRANSFER_LAYOUT_VERSION { reject(STATUS_LAYOUT); return; }
    if primary_identity() {
        let value = old_q(target_index);
        set_candidate_q(target_index, value);
        if !finite_scalar(value) { reject(STATUS_NON_FINITE); }
        return;
    }
    let base = header(2u).w + target_index * PRIMARY_WORDS;
    let indices_a = transfer[base].data;
    let metadata = transfer[base + 2u].data;
    let count = metadata.w & 255u;
    let exact = (metadata.w & 256u) != 0u;
    var value = 0.0;
    if exact {
        value = old_q(indices_a.x);
    } else {
        for (var slot = 0u; slot < min(count, 4u); slot += 1u) {
            value += old_q(indices_a[slot]) * transfer_float(base + 1u, slot);
        }
        let indices_b = metadata.xyz;
        for (var slot = 4u; slot < count; slot += 1u) {
            value += old_q(indices_b[slot - 4u]) * transfer_float(base + 3u, slot - 4u);
        }
    }
    set_candidate_q(target_index, value);
    if !finite_scalar(value) { reject(STATUS_NON_FINITE); }
}

@compute @workgroup_size(128)
fn transfer_vector(@builtin(global_invocation_id) id: vec3<u32>) {
    let target_index = id.x;
    if stopped() || target_index >= target_sample_count() { return; }
    if vector_identity() {
        let value = old_b(target_index);
        set_candidate_b(target_index, value);
        if !all(value >= vec2<f32>(-MAX_FINITE))
            || !all(value <= vec2<f32>(MAX_FINITE)) {
            reject(STATUS_NON_FINITE);
        }
        return;
    }
    let base = header(3u).x + target_index * VECTOR_WORDS;
    let indices_a = transfer[base].data;
    let metadata = transfer[base + 2u].data;
    let count = metadata.w & 255u;
    let exact = (metadata.w & 256u) != 0u;
    var value = vec2<f32>(0.0);
    if exact {
        value = old_b(indices_a.x);
    } else {
        for (var slot = 0u; slot < min(count, 4u); slot += 1u) {
            value += old_b(indices_a[slot]) * transfer_float(base + 1u, slot);
        }
        for (var slot = 4u; slot < count; slot += 1u) {
            value += old_b(metadata[slot - 4u]) * transfer_float(base + 3u, slot - 4u);
        }
    }
    set_candidate_b(target_index, value);
    if !all(value >= vec2<f32>(-MAX_FINITE)) || !all(value <= vec2<f32>(MAX_FINITE)) {
        reject(STATUS_NON_FINITE);
    }
}

@compute @workgroup_size(128)
fn transfer_gap(@builtin(global_invocation_id) id: vec3<u32>) {
    let target_index = id.x;
    if stopped() || target_index >= target_gap_count() { return; }
    let base = header(3u).y + target_index * GAP_WORDS;
    let record = transfer[base].data;
    let count = record.w & 255u;
    var value = 0.0;
    for (var slot = 0u; slot < count; slot += 1u) {
        value += old_auxiliary(record[slot]) * transfer_float(base + 1u, slot);
    }
    set_candidate_auxiliary(target_index, value);
    if !finite_scalar(value) { reject(STATUS_NON_FINITE); }
}

@compute @workgroup_size(128)
fn transfer_outgoing(@builtin(global_invocation_id) id: vec3<u32>) {
    let target_index = id.x;
    if stopped() || target_index >= target_outgoing_count() { return; }
    if outgoing_identity() {
        let value = old_auxiliary(source_gap_count() + target_index);
        set_candidate_auxiliary(target_gap_count() + target_index, value);
        if !finite_scalar(value) { reject(STATUS_NON_FINITE); }
        return;
    }
    var value = 0.0;
    let row = header(3u).z + target_index * ((source_outgoing_count() + 3u) / 4u);
    for (var source = 0u; source < source_outgoing_count(); source += 1u) {
        value += old_auxiliary(source_gap_count() + source)
            * packed_transfer_float(row, source);
    }
    set_candidate_auxiliary(target_gap_count() + target_index, value);
    if !finite_scalar(value) { reject(STATUS_NON_FINITE); }
}

@compute @workgroup_size(128)
fn reduce_density(@builtin(local_invocation_id) id: vec3<u32>) {
    let local = id.x;
    let halted = stopped();
    let identity = primary_identity();
    let participating = !halted && !identity;
    var square = 0.0;
    var count = 0.0;
    if participating {
        let inverse_support = header(3u).w;
        for (var node = local; node < source_node_count(); node += WORKGROUP_SIZE) {
            let density = old_q(node) * packed_transfer_float(inverse_support, node);
            square += density * density;
            count += 1.0;
        }
    }
    let total = reduce_four(local, vec4<f32>(square, count, 0.0, 0.0));
    if local != 0u || halted { return; }
    if identity {
        scratch[density_scratch()].values.x = 0.0;
    } else {
        scratch[density_scratch()].values.x = max(sqrt(total.x / max(total.y, 1.0)), 1.0e-12);
    }
}

@compute @workgroup_size(128)
fn reduce_components(
    @builtin(workgroup_id) group: vec3<u32>,
    @builtin(local_invocation_id) id: vec3<u32>,
) {
    let component = group.x;
    let local = id.x;
    let halted = stopped();
    let component_exists = component < target_component_count();
    let identity = primary_identity();
    let participating = !halted && component_exists && !identity;
    var record_enabled = false;
    var partial = vec4<f32>(0.0);
    if participating {
        let component_stride = header(7u).x;
        let component_record = header(4u).y + component * component_stride;
        record_enabled = transfer[component_record].data.x != 0u;
        if record_enabled {
            let labels = header(4u).x;
            for (var node = local; node < source_node_count(); node += WORKGROUP_SIZE) {
                let source_component = packed_transfer_u32(labels, node);
                let share = packed_transfer_float(component_record + 1u, source_component);
                partial.x += share * old_q(node);
            }
            let primary = header(2u).w;
            let floor = scratch[density_scratch()].values.x;
            for (var node = local; node < target_node_count(); node += WORKGROUP_SIZE) {
                let metadata = transfer[primary + node * PRIMARY_WORDS + 2u].data.w;
                let node_component = metadata >> 9u;
                if node_component != component { continue; }
                let value = candidate_q(node);
                partial.y += value;
                let exact = (metadata & 256u) != 0u;
                if !exact && new_nodes[node].boundary.z == 0u {
                    let support = transfer_float(primary + node * PRIMARY_WORDS + 3u, 3u);
                    partial.z += abs(value) + floor * support;
                    partial.w += support;
                }
            }
        }
    }
    let total = reduce_four(local, partial);
    if local != 0u { return; }
    if halted || !component_exists { return; }
    if identity || !record_enabled {
        scratch[component_scratch(component)].values = vec4<f32>(0.0);
        return;
    }
    let delta = total.x - total.y;
    let ratio = abs(delta) / max(total.z, 1.0e-30);
    if total.w == 0.0 {
        if abs(delta) > 5.0e-6 * max(max(abs(total.x), total.z), 1.0) {
            reject(STATUS_LAYOUT);
        }
    } else if ratio > 0.05 {
        reject(STATUS_TIMESTEP);
    }
    scratch[component_scratch(component)].values = vec4<f32>(delta, total.z, total.w, ratio);
}

@compute @workgroup_size(128)
fn correct_primary(@builtin(global_invocation_id) id: vec3<u32>) {
    let node = id.x;
    // Handoff commit clears this marker on both acceptance and rejection. A
    // second dispatch of this already-warm pipeline can then reuse the
    // consumed map buffer as the combined CPU receipt without introducing a
    // first-handoff pipeline-compilation stall.
    if atomicLoad(&new_status.handoff) == 0u {
        let target_nodes = new_control.counts_a.x;
        if node < target_nodes {
            transfer[node + 1u].data = bitcast<vec4<u32>>(new_state[node].values);
        }
        if node == 0u {
            let failure = max(
                atomicLoad(&new_status.candidate),
                atomicLoad(&new_status.latch));
            let accepted_slot = new_control.event.z & 1u;
            transfer[0].data = vec4<u32>(
                HANDOFF_RECEIPT_MAGIC,
                failure,
                atomicLoad(&new_status.transaction_1),
                (atomicLoad(&new_status.transaction_2) << 1u) | accepted_slot);
        }
        return;
    }
    if stopped() || node >= target_node_count() { return; }
    if primary_identity() { return; }
    let base = header(2u).w + node * PRIMARY_WORDS;
    let metadata = transfer[base + 2u].data.w;
    if (metadata & 256u) != 0u || new_nodes[node].boundary.z != 0u { return; }
    let component = metadata >> 9u;
    let correction = scratch[component_scratch(component)].values;
    if correction.z == 0.0 { return; }
    let support = transfer_float(base + 3u, 3u);
    let next = candidate_q(node) + correction.x * support / correction.z;
    set_candidate_q(node, next);
    if !finite_scalar(next) { reject(STATUS_NON_FINITE); }
}

@compute @workgroup_size(128)
fn reduce_auxiliary_energy(@builtin(local_invocation_id) id: vec3<u32>) {
    let local = id.x;
    let participating = !stopped();
    var energies = vec2<f32>(0.0);
    if participating {
        let source_gap_energy = header(5u).x;
        for (var gap = local; gap < source_gap_count(); gap += WORKGROUP_SIZE) {
            let value = old_auxiliary(gap);
            energies.x += 0.5 * packed_transfer_float(source_gap_energy, gap) * value * value;
        }
        for (var outgoing = local; outgoing < source_outgoing_count(); outgoing += WORKGROUP_SIZE) {
            let value = old_auxiliary(source_gap_count() + outgoing);
            energies.x += 0.5 * value * value;
        }
        for (var gap = local; gap < target_gap_count(); gap += WORKGROUP_SIZE) {
            let value = candidate_auxiliary(gap);
            let weight = transfer_float(header(3u).y + gap * GAP_WORDS + 2u, 0u);
            energies.y += 0.5 * weight * value * value;
        }
        for (var outgoing = local; outgoing < target_outgoing_count(); outgoing += WORKGROUP_SIZE) {
            let value = candidate_auxiliary(target_gap_count() + outgoing);
            energies.y += 0.5 * value * value;
        }
    }
    let total = reduce_four(local, vec4<f32>(energies, 0.0, 0.0));
    if participating && local == 0u {
        new_control.candidate_accounting_b.w += total.y - total.x;
    }
}
