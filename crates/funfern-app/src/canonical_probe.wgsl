struct Control {
    counts_a: vec4<u32>, counts_b: vec4<u32>, counts_c: vec4<u32>,
    table_offsets: vec4<u32>, boundary_offsets: vec4<u32>, clock_u32: vec4<u32>,
    clock_f32: vec4<f32>, clock_origin: vec4<f32>, event: vec4<u32>,
    event_result: vec4<u32>, runtime_serials: vec4<u32>, runtime_slots: vec4<u32>,
    accepted_accounting_a: vec4<f32>, accepted_accounting_b: vec4<f32>,
    candidate_accounting_a: vec4<f32>, candidate_accounting_b: vec4<f32>,
    evolution: vec4<f32>,
}
struct StateWord { values: vec4<f32> }
struct Node {
    mass_loss: vec4<f32>, ranges: vec4<u32>, boundary: vec4<u32>,
    prescribed: vec4<f32>, damping_support: vec4<f32>, stiffness: vec4<u32>,
}
struct PointStencil {
    nodes_a: vec4<u32>, nodes_b: vec4<u32>,
    primary_a: vec4<f32>, primary_b: vec4<f32>,
    complementary_a: vec4<f32>, complementary_b: vec4<f32>,
    sample_valid: vec4<u32>, reference_inverse: vec4<f32>, orientation: vec4<f32>,
}
struct ProbeControl { values: vec4<f32> }
// The third word is unused by point probes. Vector-overlay samples use it for
// metadata and keep the second word for the pre-maintenance complementary
// field needed by the presentation high-pass.
struct ProbeSample {
    primary: vec4<f32>, secondary: vec4<f32>, tertiary: vec4<f32>
}

@group(0) @binding(0) var<storage, read> control: Control;
@group(0) @binding(1) var<storage, read> state: array<StateWord>;
@group(0) @binding(2) var<storage, read> nodes: array<Node>;
@group(0) @binding(3) var<storage, read> stencils: array<PointStencil>;
@group(0) @binding(4) var<storage, read> probe_control: ProbeControl;
@group(0) @binding(5) var<storage, read_write> output: array<ProbeSample>;

fn accepted_q(node: u32) -> f32 {
    return select(state[node].values.x, state[node].values.y, (control.event.z & 1u) != 0u);
}
fn previous_q(node: u32) -> f32 {
    return select(state[node].values.y, state[node].values.x, (control.event.z & 1u) != 0u);
}
fn accepted_b(sample: u32) -> vec2<f32> {
    let value = state[control.counts_a.x + sample].values;
    return select(value.xy, value.zw, (control.event.z & 1u) != 0u);
}
fn previous_b(sample: u32) -> vec2<f32> {
    let value = state[control.counts_a.x + sample].values;
    return select(value.zw, value.xy, (control.event.z & 1u) != 0u);
}
fn fields(stencil: PointStencil) -> vec2<f32> {
    let a = stencil.nodes_a;
    let b = stencil.nodes_b;
    let current_a = vec4<f32>(
        accepted_q(a.x) * nodes[a.x].mass_loss.y,
        accepted_q(a.y) * nodes[a.y].mass_loss.y,
        accepted_q(a.z) * nodes[a.z].mass_loss.y,
        accepted_q(a.w) * nodes[a.w].mass_loss.y);
    let current_b = vec4<f32>(
        accepted_q(b.x) * nodes[b.x].mass_loss.y,
        accepted_q(b.y) * nodes[b.y].mass_loss.y,
        accepted_q(b.z) * nodes[b.z].mass_loss.y, 0.0);
    let previous_a = vec4<f32>(
        previous_q(a.x) * nodes[a.x].mass_loss.y,
        previous_q(a.y) * nodes[a.y].mass_loss.y,
        previous_q(a.z) * nodes[a.z].mass_loss.y,
        previous_q(a.w) * nodes[a.w].mass_loss.y);
    let previous_b = vec4<f32>(
        previous_q(b.x) * nodes[b.x].mass_loss.y,
        previous_q(b.y) * nodes[b.y].mass_loss.y,
        previous_q(b.z) * nodes[b.z].mass_loss.y, 0.0);
    let primary = dot(current_a, stencil.primary_a) + dot(current_b, stencil.primary_b);
    let old_primary = dot(previous_a, stencil.primary_a) + dot(previous_b, stencil.primary_b);
    return vec2<f32>(primary, (primary - old_primary) / control.clock_f32.x);
}
fn complementary_flux(stencil: PointStencil) -> vec2<f32> {
    let start = stencil.sample_valid.x;
    let x = vec4<f32>(accepted_b(start).x, accepted_b(start + 1u).x,
        accepted_b(start + 2u).x, accepted_b(start + 3u).x);
    let y = vec4<f32>(accepted_b(start).y, accepted_b(start + 1u).y,
        accepted_b(start + 2u).y, accepted_b(start + 3u).y);
    let tail_x = vec4<f32>(accepted_b(start + 4u).x, accepted_b(start + 5u).x, 0.0, 0.0);
    let tail_y = vec4<f32>(accepted_b(start + 4u).y, accepted_b(start + 5u).y, 0.0, 0.0);
    return vec2<f32>(
        dot(x, stencil.complementary_a) + dot(tail_x, stencil.complementary_b),
        dot(y, stencil.complementary_a) + dot(tail_y, stencil.complementary_b));
}
fn previous_complementary_flux(stencil: PointStencil) -> vec2<f32> {
    let start = stencil.sample_valid.x;
    let x = vec4<f32>(previous_b(start).x, previous_b(start + 1u).x,
        previous_b(start + 2u).x, previous_b(start + 3u).x);
    let y = vec4<f32>(previous_b(start).y, previous_b(start + 1u).y,
        previous_b(start + 2u).y, previous_b(start + 3u).y);
    let tail_x = vec4<f32>(previous_b(start + 4u).x,
        previous_b(start + 5u).x, 0.0, 0.0);
    let tail_y = vec4<f32>(previous_b(start + 4u).y,
        previous_b(start + 5u).y, 0.0, 0.0);
    return vec2<f32>(
        dot(x, stencil.complementary_a) + dot(tail_x, stencil.complementary_b),
        dot(y, stencil.complementary_a) + dot(tail_y, stencil.complementary_b));
}
fn physical_complement(stencil: PointStencil, flux: vec2<f32>) -> vec2<f32> {
    let inverse = stencil.reference_inverse.yzw;
    return vec2<f32>(inverse.x * flux.x + inverse.y * flux.y,
        inverse.y * flux.x + inverse.z * flux.y);
}
fn absolute_time() -> f32 {
    return control.clock_origin.x + control.clock_origin.y + control.clock_f32.y;
}

@compute @workgroup_size(16)
fn sample_probes(@builtin(local_invocation_id) invocation: vec3<u32>) {
    let probe = invocation.x;
    if probe >= u32(probe_control.values.z) || stencils[probe].sample_valid.y == 0u { return; }
    let stencil = stencils[probe];
    let primary = fields(stencil);
    let flux = complementary_flux(stencil);
    let complement = physical_complement(stencil, flux);
    let energy = 0.5 * (stencil.reference_inverse.x * primary.x * primary.x
        + dot(flux, complement));
    let flow = stencil.orientation.x * primary.x * vec2<f32>(-complement.y, complement.x);
    let stride = u32(probe_control.values.x);
    let frame = (control.clock_u32.w / stride) % u32(probe_control.values.y);
    let index = frame * 16u + probe;
    output[index].primary = vec4<f32>(primary, energy, absolute_time());
    output[index].secondary = vec4<f32>(length(complement), length(flow), 0.0, 0.0);
}

// This entry point uses the same binding layout as point probes, with binding
// 5 interpreted as a compact one-record-per-arrow buffer instead of a history
// ring. It runs once after the frame's accepted solver batch.
@compute @workgroup_size(64)
fn sample_vector_overlay(@builtin(global_invocation_id) invocation: vec3<u32>) {
    let sample = invocation.x;
    let count = u32(probe_control.values.z);
    if sample >= count { return; }
    let stencil = stencils[sample];
    if stencil.sample_valid.y == 0u {
        output[sample].primary = vec4<f32>(0.0);
        output[sample].secondary = vec4<f32>(0.0);
        output[sample].tertiary = bitcast<vec4<f32>>(
            vec4<u32>(control.clock_u32.w, 0u,
                bitcast<u32>(control.clock_origin.x),
                bitcast<u32>(control.clock_origin.y + control.clock_f32.y)));
        return;
    }
    let primary = fields(stencil);
    let flux = complementary_flux(stencil);
    let complement = physical_complement(stencil, flux);
    let previous_complement = physical_complement(
        stencil, previous_complementary_flux(stencil));
    let flow = stencil.orientation.x * primary.x * vec2<f32>(-complement.y, complement.x);
    output[sample].primary = vec4<f32>(complement, flow);
    output[sample].secondary = vec4<f32>(previous_complement, 0.0, 0.0);
    output[sample].tertiary = bitcast<vec4<f32>>(
        vec4<u32>(control.clock_u32.w, 1u,
            bitcast<u32>(control.clock_origin.x),
            bitcast<u32>(control.clock_origin.y + control.clock_f32.y)));
}
