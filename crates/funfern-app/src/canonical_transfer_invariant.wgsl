// Gate O: the complementary flux across a handoff between two generations
// that both carry the integrated field `r`. With `ḃ = ηC u` and `ṙ = u`, the
// difference `D = b − ηC r` does not move while `b` carries no loss, and the
// stiffness force reads `b` while the restoring force reads `r`, so a `D` the
// handoff introduced would be a stress the field could never shed. `D`
// crosses on the vector map instead of `b`, and the target's `b` is rebuilt
// about its own, already transferred `r`:
//     b = ηC r_target + V(b_source − ηC r_source),
// as `transfer_oscillator_flux` does on the reference, and in the same
// arrangement. It runs once, after
// `transfer_integrated`, and overwrites what `transfer_vector` wrote; with
// either side lacking `r` it leaves that untouched.
const VECTOR_WORDS: u32 = 4u;
const STATUS_NON_FINITE: u32 = 4u;
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
    accepted_accounting_c: vec4<f32>,
    candidate_accounting_c: vec4<f32>,
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
struct Sample {
    nodes_a: vec4<u32>,
    nodes_b: vec4<u32>,
    curls_01: vec4<f32>,
    curls_23: vec4<f32>,
    curls_45: vec4<f32>,
    curl_6_loss: vec4<f32>,
    constitutive: vec4<f32>,
}
struct TransferWord { data: vec4<u32> }

@group(0) @binding(0) var<storage, read_write> old_control: Control;
@group(0) @binding(1) var<storage, read_write> old_state: array<StateWord>;
@group(0) @binding(2) var<storage, read_write> old_samples: array<Sample>;
@group(0) @binding(3) var<storage, read_write> new_control: Control;
@group(0) @binding(4) var<storage, read_write> new_status: Status;
@group(0) @binding(5) var<storage, read_write> new_state: array<StateWord>;
@group(0) @binding(6) var<storage, read_write> new_samples: array<Sample>;
@group(0) @binding(7) var<storage, read_write> transfer: array<TransferWord>;

// The transfer table's header, as `canonical_transfer.wgsl` reads it.
fn header(index: u32) -> vec4<u32> { return transfer[index].data; }
fn source_node_count() -> u32 { return header(0u).y; }
fn source_sample_count() -> u32 { return header(0u).z; }
fn source_gap_count() -> u32 { return header(0u).w; }
fn source_outgoing_count() -> u32 { return header(1u).x; }
fn target_node_count() -> u32 { return header(1u).y; }
fn target_sample_count() -> u32 { return header(1u).z; }
fn target_gap_count() -> u32 { return header(1u).w; }
fn target_outgoing_count() -> u32 { return header(2u).x; }
fn source_integrated_count() -> u32 { return header(9u).y; }
fn target_integrated_count() -> u32 { return header(9u).z; }
fn vector_identity() -> bool { return header(7u).z != 0u; }
fn old_slot() -> u32 { return old_control.event.z & 1u; }
fn new_slot() -> u32 { return new_control.event.z & 1u; }
fn transfer_float(word: u32, lane: u32) -> f32 {
    return bitcast<f32>(transfer[word].data[lane]);
}
fn reject(reason: u32) { atomicMax(&new_status.candidate, reason); }
fn stopped() -> bool {
    return atomicLoad(&new_status.candidate) != 0u
        || atomicLoad(&new_status.latch) != 0u;
}

fn old_b(sample: u32) -> vec2<f32> {
    let word = old_state[source_node_count() + sample].values;
    return select(word.xy, word.zw, old_slot() != 0u);
}
fn old_integrated(node: u32) -> f32 {
    let index = source_node_count() + source_sample_count()
        + source_gap_count() + source_outgoing_count() + node;
    let word = old_state[index].values;
    return select(word.x, word.y, old_slot() != 0u);
}
fn candidate_integrated(node: u32) -> f32 {
    let index = target_node_count() + target_sample_count()
        + target_gap_count() + target_outgoing_count() + node;
    let word = new_state[index].values;
    return select(word.y, word.x, new_slot() != 0u);
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

// A sample's element nodes and curl coefficients, as the wave shader reads
// them.
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

// `ηC r` at one sample, in the difference form the drift and the reference's
// `compatible_flux` use, which annihilates a constant `r` exactly.
fn source_potential(index: u32) -> vec2<f32> {
    let sample = old_samples[index];
    let reference = old_integrated(sample.nodes_a.x);
    var curl = vec2<f32>(0.0);
    for (var local = 1u; local < 7u; local += 1u) {
        curl += sample_curl(sample, local)
            * (old_integrated(sample_node(sample, local)) - reference);
    }
    return old_control.evolution.y * curl;
}
fn target_potential(index: u32) -> vec2<f32> {
    let sample = new_samples[index];
    let reference = candidate_integrated(sample.nodes_a.x);
    var curl = vec2<f32>(0.0);
    for (var local = 1u; local < 7u; local += 1u) {
        curl += sample_curl(sample, local)
            * (candidate_integrated(sample_node(sample, local)) - reference);
    }
    return new_control.evolution.y * curl;
}

@compute @workgroup_size(128)
fn transfer_invariant(@builtin(global_invocation_id) id: vec3<u32>) {
    let target_index = id.x;
    if stopped() || target_index >= target_sample_count() { return; }
    if source_integrated_count() == 0u || target_integrated_count() == 0u { return; }
    // `V(b)` and `V(ηC r_source)` on the rows `transfer_vector` reads, in the
    // same order, and then `V(b) + (ηC r_target − V(ηC r_source))`. By
    // linearity that is `ηC r_target + V(D)`, and a sample the handoff leaves
    // as it was has its correction exactly zero, so an identity handoff
    // stays a copy bit for bit.
    var flux = vec2<f32>(0.0);
    var carried = vec2<f32>(0.0);
    if vector_identity() {
        flux = old_b(target_index);
        carried = source_potential(target_index);
    } else {
        let base = header(3u).x + target_index * VECTOR_WORDS;
        let indices_a = transfer[base].data;
        let metadata = transfer[base + 2u].data;
        let count = metadata.w & 255u;
        let exact = (metadata.w & 256u) != 0u;
        if exact {
            flux = old_b(indices_a.x);
            carried = source_potential(indices_a.x);
        } else {
            for (var slot = 0u; slot < min(count, 4u); slot += 1u) {
                let weight = transfer_float(base + 1u, slot);
                flux += old_b(indices_a[slot]) * weight;
                carried += source_potential(indices_a[slot]) * weight;
            }
            for (var slot = 4u; slot < count; slot += 1u) {
                let weight = transfer_float(base + 3u, slot - 4u);
                flux += old_b(metadata[slot - 4u]) * weight;
                carried += source_potential(metadata[slot - 4u]) * weight;
            }
        }
    }
    let value = flux + (target_potential(target_index) - carried);
    set_candidate_b(target_index, value);
    if !all(value >= vec2<f32>(-MAX_FINITE)) || !all(value <= vec2<f32>(MAX_FINITE)) {
        reject(STATUS_NON_FINITE);
    }
}
