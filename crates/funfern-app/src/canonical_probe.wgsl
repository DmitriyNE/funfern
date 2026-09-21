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
    temporal_primary_a: vec4<u32>, temporal_primary_b: vec4<u32>,
    temporal_complementary: vec4<u32>,
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
@group(0) @binding(6) var<storage, read> tables: array<TableWord>;

const TEMPORAL_COEFFICIENT_WORDS: u32 = 3u;
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
fn point_temporal_factor(stencil: PointStencil, primary: bool) -> f32 {
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
        table_float(word + 2u, 0u), spatial_phase, control.clock_f32.y);
}
fn fields(stencil: PointStencil) -> vec2<f32> {
    let a = stencil.nodes_a;
    let b = stencil.nodes_b;
    let current_a = vec4<f32>(
        accepted_q(a.x) * primary_inverse_mass(a.x, control.clock_f32.y),
        accepted_q(a.y) * primary_inverse_mass(a.y, control.clock_f32.y),
        accepted_q(a.z) * primary_inverse_mass(a.z, control.clock_f32.y),
        accepted_q(a.w) * primary_inverse_mass(a.w, control.clock_f32.y));
    let current_b = vec4<f32>(
        accepted_q(b.x) * primary_inverse_mass(b.x, control.clock_f32.y),
        accepted_q(b.y) * primary_inverse_mass(b.y, control.clock_f32.y),
        accepted_q(b.z) * primary_inverse_mass(b.z, control.clock_f32.y), 0.0);
    let previous_time = control.clock_f32.y - control.clock_f32.x;
    let previous_a = vec4<f32>(
        previous_q(a.x) * primary_inverse_mass(a.x, previous_time),
        previous_q(a.y) * primary_inverse_mass(a.y, previous_time),
        previous_q(a.z) * primary_inverse_mass(a.z, previous_time),
        previous_q(a.w) * primary_inverse_mass(a.w, previous_time));
    let previous_b = vec4<f32>(
        previous_q(b.x) * primary_inverse_mass(b.x, previous_time),
        previous_q(b.y) * primary_inverse_mass(b.y, previous_time),
        previous_q(b.z) * primary_inverse_mass(b.z, previous_time), 0.0);
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
        inverse.y * flux.x + inverse.z * flux.y)
        / point_temporal_factor(stencil, false);
}
fn absolute_time() -> f32 {
    return control.clock_origin.x + control.clock_origin.y + control.clock_f32.y;
}

@compute @workgroup_size(16)
fn sample_probes(@builtin(local_invocation_id) invocation: vec3<u32>) {
    let probe = invocation.x;
    if probe >= u32(probe_control.values.z) || stencils[probe].sample_valid.y == 0u { return; }
    let stencil = stencils[probe];
    if temporal_enabled() && stencil.sample_valid.z == 0u { return; }
    let primary = fields(stencil);
    let flux = complementary_flux(stencil);
    let complement = physical_complement(stencil, flux);
    let energy = 0.5 * (stencil.reference_inverse.x
        * point_temporal_factor(stencil, true) * primary.x * primary.x
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
    if temporal_enabled() && stencil.sample_valid.z == 0u {
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
