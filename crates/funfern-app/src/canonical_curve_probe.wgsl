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
    temporal_primary_a: vec4<u32>, temporal_primary_b: vec4<u32>,
    temporal_complementary: vec4<u32>,
}
struct CurveStencil { point: PointStencil, normal_stride_valid: vec4<f32> }
struct ProbeControl { values: vec4<f32> }
struct ProbeSample { primary: vec4<f32>, secondary: vec4<f32> }

@group(0) @binding(0) var<storage, read> control: Control;
@group(0) @binding(1) var<storage, read> state: array<StateWord>;
@group(0) @binding(2) var<storage, read> nodes: array<Node>;
@group(0) @binding(3) var<storage, read> stencils: array<CurveStencil>;
@group(0) @binding(4) var<storage, read> probe_control: ProbeControl;
@group(0) @binding(5) var<storage, read_write> output: array<ProbeSample>;

fn accepted_q(node: u32) -> f32 {
    return select(state[node].values.x, state[node].values.y, (control.event.z & 1u) != 0u);
}
fn accepted_b(sample: u32) -> vec2<f32> {
    let value = state[control.counts_a.x + sample].values;
    return select(value.xy, value.zw, (control.event.z & 1u) != 0u);
}
fn primary_field(stencil: PointStencil) -> f32 {
    let a = stencil.nodes_a;
    let b = stencil.nodes_b;
    let values_a = vec4<f32>(
        accepted_q(a.x) * nodes[a.x].mass_loss.y,
        accepted_q(a.y) * nodes[a.y].mass_loss.y,
        accepted_q(a.z) * nodes[a.z].mass_loss.y,
        accepted_q(a.w) * nodes[a.w].mass_loss.y);
    let values_b = vec4<f32>(
        accepted_q(b.x) * nodes[b.x].mass_loss.y,
        accepted_q(b.y) * nodes[b.y].mass_loss.y,
        accepted_q(b.z) * nodes[b.z].mass_loss.y, 0.0);
    return dot(values_a, stencil.primary_a) + dot(values_b, stencil.primary_b);
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
fn physical_complement(stencil: PointStencil, flux: vec2<f32>) -> vec2<f32> {
    let inverse = stencil.reference_inverse.yzw;
    return vec2<f32>(inverse.x * flux.x + inverse.y * flux.y,
        inverse.y * flux.x + inverse.z * flux.y);
}
fn absolute_time() -> f32 {
    return control.clock_origin.x + control.clock_origin.y + control.clock_f32.y;
}

@compute @workgroup_size(64)
fn sample_curve_probes(@builtin(global_invocation_id) invocation: vec3<u32>) {
    let index = invocation.x;
    if index >= u32(probe_control.values.x) { return; }
    let record = stencils[index];
    let stride = u32(record.normal_stride_valid.z);
    if stride == 0u || control.clock_u32.w % stride != 0u { return; }
    let frame = (control.clock_u32.w / stride) % u32(probe_control.values.y);
    let slot = frame * u32(probe_control.values.z) + index;
    if record.normal_stride_valid.w < 0.5 || record.point.sample_valid.y == 0u {
        let nan = bitcast<f32>(0x7fc00000u | (index & 1u));
        output[slot].primary = vec4<f32>(nan, nan, nan, absolute_time());
        output[slot].secondary = vec4<f32>(nan);
        return;
    }
    let primary = primary_field(record.point);
    let flux = complementary_flux(record.point);
    let complement = physical_complement(record.point, flux);
    let energy = 0.5 * (record.point.reference_inverse.x * primary * primary
        + dot(flux, complement));
    let flow = record.point.orientation.x * primary * vec2<f32>(-complement.y, complement.x);
    output[slot].primary = vec4<f32>(
        primary, energy, dot(flow, record.normal_stride_valid.xy), absolute_time());
    output[slot].secondary = vec4<f32>(length(complement), 0.0, 0.0, 0.0);
}
