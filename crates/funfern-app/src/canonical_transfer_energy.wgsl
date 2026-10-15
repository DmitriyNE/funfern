// Full-state edit accounting for canonical generation handoff.
const WORKGROUP_SIZE: u32 = 128u;
// The exponent bits of an f32 that is infinite or NaN. Finiteness is read
// from them rather than against a float bound: naga writes an f32 constant
// out in full digits, which Safari cannot read past 2^63 and Chrome rejects at
// f32::MAX, and a float comparison is one fast math may assume never meets a
// NaN.
const NON_FINITE_EXPONENT: u32 = 0x7f800000u;

fn finite_scalar(value: f32) -> bool {
    return (bitcast<u32>(value) & NON_FINITE_EXPONENT) != NON_FINITE_EXPONENT;
}

struct Control {
    counts_a: vec4<u32>, counts_b: vec4<u32>, counts_c: vec4<u32>,
    table_offsets: vec4<u32>, boundary_offsets: vec4<u32>, clock_u32: vec4<u32>,
    clock_f32: vec4<f32>, clock_origin: vec4<f32>, event: vec4<u32>,
    event_result: vec4<u32>, runtime_serials: vec4<u32>,
    runtime_slots: vec4<u32>,
    accepted_accounting_a: vec4<f32>, accepted_accounting_b: vec4<f32>,
    candidate_accounting_a: vec4<f32>, candidate_accounting_b: vec4<f32>,
    evolution: vec4<f32>,
    // Gate O: accepted and candidate active gain (x); the rest is reserved.
    accepted_accounting_c: vec4<f32>,
    candidate_accounting_c: vec4<f32>,
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

// The handoff's edit: the target's store less the source's, each valued by
// its own generation's maps in `handoff_target_energy` and
// `handoff_source_energy`, less what the target's pins exchanged. The bulk
// was valued here once, by the authored quadratic stores, which are neither
// generation's own on a time-driven one. The reserved words are cleared, so
// nothing of them stays on the new generation.
@compute @workgroup_size(1)
fn account_handoff() {
    let prescribed_exchange = new_control.candidate_accounting_a.y
        - old_control.accepted_accounting_a.y;
    let edit = new_control.candidate_accounting_c.z - old_control.accepted_accounting_c.y
        - prescribed_exchange;
    new_control.candidate_accounting_c.y = 0.0;
    new_control.candidate_accounting_c.z = 0.0;
    new_control.accepted_accounting_c.y = 0.0;
    new_control.accepted_accounting_c.z = 0.0;
    if finite_scalar(edit) {
        new_control.candidate_accounting_b.w += edit;
    } else {
        // WebGPU rejects a constant expression whose value is NaN. Keep the
        // payload runtime-dependent while preserving the failure sentinel.
        new_control.candidate_accounting_b.w =
            bitcast<f32>(0x7fc00000u | (bitcast<u32>(edit) & 1u));
    }
}
