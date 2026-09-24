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
struct TableWord { data: vec4<u32> }
struct PointStencil {
    nodes_a: vec4<u32>, nodes_b: vec4<u32>,
    primary_a: vec4<f32>, primary_b: vec4<f32>,
    complementary_a: vec4<f32>, complementary_b: vec4<f32>,
    sample_valid: vec4<u32>, reference_inverse: vec4<f32>, orientation: vec4<f32>,
    sample_inverse: array<vec4<f32>, 6>,
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
@group(0) @binding(6) var<storage, read> tables: array<TableWord>;

// ---- shared canonical point reconstruction ----
// Byte-identical in canonical_probe.wgsl and canonical_curve_probe.wgsl, and
// asserted so by a test, because a consumer that quietly drifts from its
// neighbours reports a plausible wrong number instead of failing.
//
// The order here is the contract: a constitutive inverse is applied only at
// the element's own samples, where the solver owns one, and the resulting
// physical field is interpolated. Densities then use forward coefficients at
// the probe point. Inverting once at an interpolated flux would be cheaper
// and is what this replaced, but it invents an evaluation site the solver
// does not have.
const COMPLEMENTARY_SAMPLES: u32 = 6u;
const TEMPORAL_COEFFICIENT_WORDS: u32 = 4u;
const TEMPORAL_DRIVE_NONE: u32 = 0u;
const TEMPORAL_DRIVE_PUMP: u32 = 1u;
const TEMPORAL_DRIVE_CRYSTAL: u32 = 2u;
const TEMPORAL_DRIVE_TRAVELLING: u32 = 3u;
const TEMPORAL_HAS_ALTERNATE: u32 = 1u;
const TEMPORAL_INVERTED: u32 = 2u;

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
fn temporal_enabled() -> bool { return (control.runtime_slots.w & 1u) != 0u; }
fn table_float(word: u32, lane: u32) -> f32 {
    return bitcast<f32>(tables[word].data[lane]);
}
fn reduced_phase(value: f32) -> f32 { return atan2(sin(value), cos(value)); }
fn smootherstep(value: f32) -> f32 {
    return value * value * value * (value * (value * 6.0 - 15.0) + 10.0);
}
fn apply_symmetric(tensor: vec3<f32>, value: vec2<f32>) -> vec2<f32> {
    return vec2<f32>(tensor.x * value.x + tensor.y * value.y,
        tensor.y * value.x + tensor.z * value.y);
}
fn temporal_switch_blend(runtime_word: u32, local_time: f32) -> f32 {
    let start_blend = table_float(runtime_word + 1u, 0u);
    let target_blend = table_float(runtime_word + 1u, 1u);
    let start_time = table_float(runtime_word + 1u, 2u);
    let duration = table_float(runtime_word + 1u, 3u);
    if duration == 0.0 { return target_blend; }
    let z = clamp((local_time - start_time) / duration, 0.0, 1.0);
    return start_blend + (target_blend - start_blend) * smootherstep(z);
}
fn temporal_factor(
    metadata: vec4<u32>, values: vec4<u32>, shape: f32, spatial_phase: f32,
    local_time: f32,
) -> f32 {
    let runtime_header = tables[control.runtime_slots.z].data;
    let runtime_word = runtime_header.w + 6u * metadata.x
        + 3u * (control.runtime_slots.y & 1u);
    let phase = table_float(runtime_word, metadata.y)
        + bitcast<f32>(values.w) * local_time;
    let carrier = reduced_phase(phase);
    let depth = bitcast<f32>(values.z);
    var drive = 1.0;
    switch metadata.z {
        case TEMPORAL_DRIVE_NONE: {}
        case TEMPORAL_DRIVE_PUMP: { drive += depth * cos(carrier); }
        case TEMPORAL_DRIVE_CRYSTAL: {
            let cosine = cos(carrier);
            var square: f32;
            if abs(shape) < 0.001 {
                square = cosine * (1.0
                    + shape * shape * (1.0 - cosine * cosine) / 3.0);
            } else {
                square = tanh(shape * cosine) / tanh(shape);
            }
            drive += depth * square;
        }
        case TEMPORAL_DRIVE_TRAVELLING: {
            drive += depth * cos(carrier - spatial_phase);
        }
        default: { return 0.0; }
    }
    let blend = temporal_switch_blend(runtime_word, local_time);
    var switch_factor = 1.0;
    if (metadata.w & TEMPORAL_HAS_ALTERNATE) != 0u {
        switch_factor += blend * (bitcast<f32>(values.y) - 1.0);
    }
    let factor = drive * switch_factor;
    return select(factor, 1.0 / factor, (metadata.w & TEMPORAL_INVERTED) != 0u);
}
// The assembled nodal map, summed over every contribution that owns this
// node. This is the primary inverse the solver itself owns.
fn primary_inverse_mass(node: u32, local_time: f32) -> f32 {
    if !temporal_enabled() { return nodes[node].mass_loss.y; }
    let range = nodes[node].stiffness.zw;
    var mass = 0.0;
    for (var record = 0u; record < range.y; record += 1u) {
        let word = range.x + record * TEMPORAL_COEFFICIENT_WORDS;
        let metadata = tables[word].data;
        mass += table_float(word + 1u, 0u) * temporal_factor(
            metadata, tables[word + 1u].data, table_float(word + 2u, 0u),
            table_float(word + 2u, 1u), local_time);
    }
    return 1.0 / mass;
}
fn complementary_weights(stencil: PointStencil) -> array<f32, 6> {
    return array<f32, 6>(
        stencil.complementary_a.x, stencil.complementary_a.y,
        stencil.complementary_a.z, stencil.complementary_a.w,
        stencil.complementary_b.x, stencil.complementary_b.y);
}
fn accepted_sample_flux(stencil: PointStencil) -> array<vec2<f32>, 6> {
    let start = stencil.sample_valid.x;
    return array<vec2<f32>, 6>(
        accepted_b(start), accepted_b(start + 1u), accepted_b(start + 2u),
        accepted_b(start + 3u), accepted_b(start + 4u), accepted_b(start + 5u));
}
fn previous_sample_flux(stencil: PointStencil) -> array<vec2<f32>, 6> {
    let start = stencil.sample_valid.x;
    return array<vec2<f32>, 6>(
        previous_b(start), previous_b(start + 1u), previous_b(start + 2u),
        previous_b(start + 3u), previous_b(start + 4u), previous_b(start + 5u));
}
// The law at one of the element's own samples, evaluated at that sample's own
// material coordinates rather than at an interpolated phase.
fn sample_temporal_factor(stencil: PointStencil, local: u32, local_time: f32) -> f32 {
    if !temporal_enabled() { return 1.0; }
    let word = stencil.temporal_complementary.x + local * TEMPORAL_COEFFICIENT_WORDS;
    return temporal_factor(tables[word].data, tables[word + 1u].data,
        table_float(word + 2u, 0u), table_float(word + 2u, 1u), local_time);
}
// The same law at the probe point, for the pointwise densities only.
fn probe_temporal_factor(stencil: PointStencil, primary: bool, local_time: f32) -> f32 {
    if !temporal_enabled() { return 1.0; }
    var word = stencil.temporal_complementary.x;
    var spatial_phase = dot(vec4<f32>(
        table_float(word + 2u, 1u),
        table_float(word + 5u, 1u),
        table_float(word + 8u, 1u),
        table_float(word + 11u, 1u)), stencil.complementary_a)
        + dot(vec4<f32>(
            table_float(word + 14u, 1u),
            table_float(word + 17u, 1u), 0.0, 0.0), stencil.complementary_b);
    if primary {
        let words_a = stencil.temporal_primary_a;
        let words_b = stencil.temporal_primary_b;
        word = words_a.x;
        spatial_phase = dot(vec4<f32>(
            table_float(words_a.x + 2u, 1u),
            table_float(words_a.y + 2u, 1u),
            table_float(words_a.z + 2u, 1u),
            table_float(words_a.w + 2u, 1u)), stencil.primary_a)
            + dot(vec4<f32>(
                table_float(words_b.x + 2u, 1u),
                table_float(words_b.y + 2u, 1u),
                table_float(words_b.z + 2u, 1u), 0.0), stencil.primary_b);
    }
    return temporal_factor(tables[word].data, tables[word + 1u].data,
        table_float(word + 2u, 0u), spatial_phase, local_time);
}
fn physical_complement(
    stencil: PointStencil, flux: array<vec2<f32>, 6>, local_time: f32,
) -> vec2<f32> {
    let weights = complementary_weights(stencil);
    var field = vec2<f32>(0.0);
    for (var local = 0u; local < COMPLEMENTARY_SAMPLES; local += 1u) {
        let recovered = apply_symmetric(stencil.sample_inverse[local].xyz, flux[local])
            / sample_temporal_factor(stencil, local, local_time);
        field += recovered * weights[local];
    }
    return field;
}
fn complementary_energy(stencil: PointStencil, field: vec2<f32>, local_time: f32) -> f32 {
    return 0.5 * probe_temporal_factor(stencil, false, local_time)
        * dot(field, apply_symmetric(stencil.reference_inverse.yzw, field));
}
fn primary_energy(stencil: PointStencil, value: f32, local_time: f32) -> f32 {
    return 0.5 * stencil.reference_inverse.x
        * probe_temporal_factor(stencil, true, local_time) * value * value;
}
fn energy_flow(stencil: PointStencil, value: f32, field: vec2<f32>) -> vec2<f32> {
    return stencil.orientation.x * value * vec2<f32>(-field.y, field.x);
}
fn primary_field(stencil: PointStencil, local_time: f32) -> f32 {
    let a = stencil.nodes_a;
    let b = stencil.nodes_b;
    let values_a = vec4<f32>(
        accepted_q(a.x) * primary_inverse_mass(a.x, local_time),
        accepted_q(a.y) * primary_inverse_mass(a.y, local_time),
        accepted_q(a.z) * primary_inverse_mass(a.z, local_time),
        accepted_q(a.w) * primary_inverse_mass(a.w, local_time));
    let values_b = vec4<f32>(
        accepted_q(b.x) * primary_inverse_mass(b.x, local_time),
        accepted_q(b.y) * primary_inverse_mass(b.y, local_time),
        accepted_q(b.z) * primary_inverse_mass(b.z, local_time), 0.0);
    return dot(values_a, stencil.primary_a) + dot(values_b, stencil.primary_b);
}
fn primary_field_and_rate(stencil: PointStencil) -> vec2<f32> {
    let a = stencil.nodes_a;
    let b = stencil.nodes_b;
    let previous_time = control.clock_f32.y - control.clock_f32.x;
    let value = primary_field(stencil, control.clock_f32.y);
    let previous_a = vec4<f32>(
        previous_q(a.x) * primary_inverse_mass(a.x, previous_time),
        previous_q(a.y) * primary_inverse_mass(a.y, previous_time),
        previous_q(a.z) * primary_inverse_mass(a.z, previous_time),
        previous_q(a.w) * primary_inverse_mass(a.w, previous_time));
    let previous_b_values = vec4<f32>(
        previous_q(b.x) * primary_inverse_mass(b.x, previous_time),
        previous_q(b.y) * primary_inverse_mass(b.y, previous_time),
        previous_q(b.z) * primary_inverse_mass(b.z, previous_time), 0.0);
    let old_value = dot(previous_a, stencil.primary_a)
        + dot(previous_b_values, stencil.primary_b);
    return vec2<f32>(value, (value - old_value) / control.clock_f32.x);
}
fn absolute_time() -> f32 {
    return control.clock_origin.x + control.clock_origin.y + control.clock_f32.y;
}
// ---- end shared canonical point reconstruction ----

@compute @workgroup_size(64)
fn sample_curve_probes(@builtin(global_invocation_id) invocation: vec3<u32>) {
    let index = invocation.x;
    if index >= u32(probe_control.values.x) { return; }
    let record = stencils[index];
    let stride = u32(record.normal_stride_valid.z);
    if stride == 0u || control.clock_u32.w % stride != 0u { return; }
    let frame = (control.clock_u32.w / stride) % u32(probe_control.values.y);
    let slot = frame * u32(probe_control.values.z) + index;
    if record.normal_stride_valid.w < 0.5 || record.point.sample_valid.y == 0u
        || (temporal_enabled() && record.point.sample_valid.z == 0u) {
        let nan = bitcast<f32>(0x7fc00000u | (index & 1u));
        output[slot].primary = vec4<f32>(nan, nan, nan, absolute_time());
        output[slot].secondary = vec4<f32>(nan);
        return;
    }
    let time = control.clock_f32.y;
    let primary = primary_field(record.point, time);
    let complement = physical_complement(
        record.point, accepted_sample_flux(record.point), time);
    let energy = primary_energy(record.point, primary, time)
        + complementary_energy(record.point, complement, time);
    let flow = energy_flow(record.point, primary, complement);
    output[slot].primary = vec4<f32>(
        primary, energy, dot(flow, record.normal_stride_valid.xy), absolute_time());
    output[slot].secondary = vec4<f32>(length(complement), 0.0, 0.0, 0.0);
}
