// Full-state edit accounting for canonical generation handoff.
const WORKGROUP_SIZE: u32 = 128u;
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
struct StateWord { values: vec4<f32> }
struct Node {
    mass_loss: vec4<f32>, ranges: vec4<u32>, boundary: vec4<u32>,
    prescribed: vec4<f32>, damping_support: vec4<f32>, stiffness: vec4<u32>,
}
struct Sample {
    nodes_a: vec4<u32>, nodes_b: vec4<u32>, curls_01: vec4<f32>,
    curls_23: vec4<f32>, curls_45: vec4<f32>, curl_6_loss: vec4<f32>,
    constitutive: vec4<f32>,
}

@group(0) @binding(0) var<storage, read_write> old_control: Control;
@group(0) @binding(1) var<storage, read_write> old_state: array<StateWord>;
@group(0) @binding(2) var<storage, read_write> old_nodes: array<Node>;
@group(0) @binding(3) var<storage, read_write> old_samples: array<Sample>;
@group(0) @binding(4) var<storage, read_write> new_control: Control;
@group(0) @binding(5) var<storage, read_write> new_state: array<StateWord>;
@group(0) @binding(6) var<storage, read_write> new_nodes: array<Node>;
@group(0) @binding(7) var<storage, read_write> new_samples: array<Sample>;

var<workgroup> reduced: array<vec2<f32>, 128>;

fn old_slot() -> u32 { return old_control.event.z & 1u; }
fn new_slot() -> u32 { return new_control.event.z & 1u; }
fn old_q(node: u32) -> f32 {
    return select(old_state[node].values.x, old_state[node].values.y, old_slot() != 0u);
}
fn old_b(sample: u32) -> vec2<f32> {
    let value = old_state[old_control.counts_a.x + sample].values;
    return select(value.xy, value.zw, old_slot() != 0u);
}
fn new_q(node: u32) -> f32 {
    return select(new_state[node].values.y, new_state[node].values.x, new_slot() != 0u);
}
fn new_b(sample: u32) -> vec2<f32> {
    let value = new_state[new_control.counts_a.x + sample].values;
    return select(value.zw, value.xy, new_slot() != 0u);
}
fn b_energy(sample: Sample, value: vec2<f32>) -> f32 {
    let tensor = sample.constitutive;
    let field = vec2<f32>(
        tensor.x * value.x + tensor.y * value.y,
        tensor.y * value.x + tensor.z * value.y);
    return 0.5 * tensor.w * dot(value, field);
}

@compute @workgroup_size(128)
fn account_handoff(@builtin(local_invocation_id) id: vec3<u32>) {
    let local = id.x;
    var energy = vec2<f32>(0.0);
    for (var node = local; node < old_control.counts_a.x; node += WORKGROUP_SIZE) {
        let q = old_q(node);
        energy.x += 0.5 * old_nodes[node].mass_loss.y * q * q;
    }
    for (var sample = local; sample < old_control.counts_a.y; sample += WORKGROUP_SIZE) {
        energy.x += b_energy(old_samples[sample], old_b(sample));
    }
    for (var node = local; node < new_control.counts_a.x; node += WORKGROUP_SIZE) {
        let q = new_q(node);
        energy.y += 0.5 * new_nodes[node].mass_loss.y * q * q;
    }
    for (var sample = local; sample < new_control.counts_a.y; sample += WORKGROUP_SIZE) {
        energy.y += b_energy(new_samples[sample], new_b(sample));
    }
    reduced[local] = energy;
    workgroupBarrier();
    var width = WORKGROUP_SIZE / 2u;
    loop {
        if width == 0u { break; }
        if local < width { reduced[local] += reduced[local + width]; }
        workgroupBarrier();
        width /= 2u;
    }
    if local != 0u { return; }
    let prescribed_exchange = new_control.candidate_accounting_a.y
        - old_control.accepted_accounting_a.y;
    let edit = reduced[0].y - reduced[0].x - prescribed_exchange;
    if edit >= -MAX_FINITE && edit <= MAX_FINITE {
        new_control.candidate_accounting_b.w += edit;
    } else {
        // WebGPU rejects a constant expression whose value is NaN. Keep the
        // payload runtime-dependent while preserving the failure sentinel.
        new_control.candidate_accounting_b.w = bitcast<f32>(0x7fc00000u | (local & 1u));
    }
}
