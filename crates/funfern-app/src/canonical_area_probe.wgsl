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
struct AreaQuadrature {
    primary_a: vec4<f32>, primary_b: vec4<f32>,
    complementary_a: vec4<f32>, complementary_b: vec4<f32>,
    reference_inverse: vec4<f32>, weight: vec4<f32>,
}
struct AreaContribution {
    nodes_a: vec4<u32>, nodes_b: vec4<u32>, sample_valid: vec4<u32>,
    quadrature: array<AreaQuadrature, 12>,
}
struct AreaDescriptor { offset_count: vec4<u32>, areas: vec4<f32> }
struct ProbeControl { values: vec4<f32> }
struct ContributionSample { primary: vec4<f32> }
struct AreaSample { primary: vec4<f32>, secondary: vec4<f32>, tertiary: vec4<f32> }

@group(0) @binding(0) var<storage, read> control: Control;
@group(0) @binding(1) var<storage, read> state: array<StateWord>;
@group(0) @binding(2) var<storage, read> nodes: array<Node>;
@group(0) @binding(3) var<storage, read> contributions: array<AreaContribution>;
@group(0) @binding(4) var<storage, read> descriptors: array<AreaDescriptor>;
@group(0) @binding(5) var<storage, read> probe_control: ProbeControl;
@group(0) @binding(6) var<storage, read_write> scratch: array<ContributionSample>;
@group(0) @binding(7) var<storage, read_write> output: array<AreaSample>;

fn accepted_q(node: u32) -> f32 {
    return select(state[node].values.x, state[node].values.y, (control.event.z & 1u) != 0u);
}
fn accepted_b(sample: u32) -> vec2<f32> {
    let value = state[control.counts_a.x + sample].values;
    return select(value.xy, value.zw, (control.event.z & 1u) != 0u);
}
fn absolute_time() -> f32 {
    return control.clock_origin.x + control.clock_origin.y + control.clock_f32.y;
}

@compute @workgroup_size(64)
fn sample_area_elements(@builtin(global_invocation_id) invocation: vec3<u32>) {
    let element = invocation.x;
    if element >= u32(probe_control.values.w) { return; }
    let contribution = contributions[element];
    if contribution.sample_valid.y == 0u { return; }
    let a = contribution.nodes_a;
    let b = contribution.nodes_b;
    let primary_a = vec4<f32>(
        accepted_q(a.x) * nodes[a.x].mass_loss.y,
        accepted_q(a.y) * nodes[a.y].mass_loss.y,
        accepted_q(a.z) * nodes[a.z].mass_loss.y,
        accepted_q(a.w) * nodes[a.w].mass_loss.y);
    let primary_b = vec4<f32>(
        accepted_q(b.x) * nodes[b.x].mass_loss.y,
        accepted_q(b.y) * nodes[b.y].mass_loss.y,
        accepted_q(b.z) * nodes[b.z].mass_loss.y, 0.0);
    let start = contribution.sample_valid.x;
    let flux_x_a = vec4<f32>(accepted_b(start).x, accepted_b(start + 1u).x,
        accepted_b(start + 2u).x, accepted_b(start + 3u).x);
    let flux_y_a = vec4<f32>(accepted_b(start).y, accepted_b(start + 1u).y,
        accepted_b(start + 2u).y, accepted_b(start + 3u).y);
    let flux_x_b = vec4<f32>(accepted_b(start + 4u).x, accepted_b(start + 5u).x, 0.0, 0.0);
    let flux_y_b = vec4<f32>(accepted_b(start + 4u).y, accepted_b(start + 5u).y, 0.0, 0.0);
    var accumulated = vec4<f32>(0.0);
    for (var point = 0u; point < 12u; point += 1u) {
        let quadrature = contribution.quadrature[point];
        let primary = dot(primary_a, quadrature.primary_a) + dot(primary_b, quadrature.primary_b);
        let flux = vec2<f32>(
            dot(flux_x_a, quadrature.complementary_a) + dot(flux_x_b, quadrature.complementary_b),
            dot(flux_y_a, quadrature.complementary_a) + dot(flux_y_b, quadrature.complementary_b));
        let inverse = quadrature.reference_inverse.yzw;
        let complement = vec2<f32>(inverse.x * flux.x + inverse.y * flux.y,
            inverse.y * flux.x + inverse.z * flux.y);
        let weight = quadrature.weight.x;
        let energy = 0.5 * weight * (quadrature.reference_inverse.x * primary * primary
            + dot(flux, complement));
        accumulated += vec4<f32>(weight * primary, weight * primary * primary,
            weight * dot(complement, complement), energy);
    }
    scratch[element].primary = accumulated;
}

@compute @workgroup_size(16)
fn reduce_area_probes(@builtin(local_invocation_id) invocation: vec3<u32>) {
    let probe = invocation.x;
    if probe >= u32(probe_control.values.z) { return; }
    let descriptor = descriptors[probe];
    let stride = u32(probe_control.values.x);
    let frame = (control.clock_u32.w / stride) % u32(probe_control.values.y);
    let output_index = frame * 16u + probe;
    let nan = bitcast<f32>(0x7fc00000u | (probe & 1u));
    if descriptor.offset_count.y == 0u || descriptor.areas.x <= 0.0 {
        output[output_index].primary = vec4<f32>(nan);
        output[output_index].secondary = vec4<f32>(nan);
        output[output_index].tertiary = vec4<f32>(0.0);
        return;
    }
    var accumulated = vec4<f32>(0.0);
    let end = descriptor.offset_count.x + descriptor.offset_count.y;
    for (var index = descriptor.offset_count.x; index < end; index += 1u) {
        accumulated += scratch[index].primary;
    }
    let covered_area = descriptor.areas.x;
    let target_area = descriptor.areas.y;
    let total_energy = max(accumulated.w, 0.0);
    output[output_index].primary = vec4<f32>(
        accumulated.x / covered_area,
        sqrt(max(accumulated.y / covered_area, 0.0)),
        sqrt(max(accumulated.z / covered_area, 0.0)),
        total_energy / covered_area);
    output[output_index].secondary = vec4<f32>(total_energy, covered_area,
        select(0.0, clamp(covered_area / target_area, 0.0, 1.0), target_area > 0.0),
        absolute_time());
    output[output_index].tertiary = vec4<f32>(1.0, 0.0, 0.0, 0.0);
}
