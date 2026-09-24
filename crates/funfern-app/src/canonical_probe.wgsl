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
const TEMPORAL_FIELD_KERR: u32 = 4u;
const TEMPORAL_FIELD_SATURABLE: u32 = 8u;
const TEMPORAL_FIELD_MASK: u32 = 12u;
const PROBE_INVERSE_TOLERANCE: f32 = 4.76837158e-7;
const PROBE_INVERSE_ITERATIONS: u32 = 40u;

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
// The executed field laws, as the solver evaluates them: `ḡ`, `ḡ + rḡ′`
// and the co-energy of one record at `r = |field|`.
fn field_kind(word: u32) -> u32 { return tables[word].data.w & TEMPORAL_FIELD_MASK; }
fn field_response(word: u32, r: f32) -> vec3<f32> {
    let chi = table_float(word + 3u, 0u);
    let square = r * r;
    switch field_kind(word) {
        case TEMPORAL_FIELD_KERR: {
            return vec3<f32>(1.0 + chi * square, 1.0 + 3.0 * chi * square,
                0.5 * square + 0.25 * chi * square * square);
        }
        case TEMPORAL_FIELD_SATURABLE: {
            let saturation = table_float(word + 3u, 1u);
            let sigma2 = saturation * saturation;
            let x = square / sigma2;
            let denominator = 1.0 + x;
            var excess: f32;
            if x < 0.03 {
                excess = x * x * (0.5 - x * (1.0 / 3.0 - x * (0.25 - x * 0.2)));
            } else {
                excess = x - log(denominator);
            }
            return vec3<f32>(1.0 + chi * square / denominator,
                1.0 + chi * square * (x + 3.0) / (denominator * denominator),
                0.5 * square + 0.5 * chi * sigma2 * sigma2 * excess);
        }
        default: { return vec3<f32>(1.0, 1.0, 0.5 * square); }
    }
}
fn record_factor(word: u32, local_time: f32) -> f32 {
    return temporal_factor(tables[word].data, tables[word + 1u].data,
        table_float(word + 2u, 0u), table_float(word + 2u, 1u), local_time);
}
// The node's field from its flux, through the same assembled map and the
// same bracketed solve the solver runs. A probe cannot fail the step; a solve
// that does not settle within the cap returns its last bracketed iterate,
// and the solver's own inverse at that stage has already raised the status.
fn probe_primary_field(node: u32, flux: f32, local_time: f32) -> f32 {
    if !temporal_enabled() { return flux * nodes[node].mass_loss.y; }
    let range = nodes[node].stiffness.zw;
    var nonlinear = false;
    var mass = 0.0;
    var floor_value = 0.0;
    var bound = 0.0;
    for (var record = 0u; record < range.y; record += 1u) {
        let word = range.x + record * TEMPORAL_COEFFICIENT_WORDS;
        let coefficient = table_float(word + 1u, 0u) * record_factor(word, local_time);
        mass += coefficient;
        if field_kind(word) != 0u {
            nonlinear = true;
            floor_value += coefficient * table_float(word + 3u, 3u);
            let own = table_float(word + 3u, 2u);
            if own > 0.0 && (bound == 0.0 || own < bound) { bound = own; }
        } else {
            floor_value += coefficient;
        }
    }
    if !nonlinear { return flux / mass; }
    let goal = abs(flux);
    if goal == 0.0 || !(floor_value > 0.0) { return 0.0; }
    var high = goal / floor_value;
    if bound > 0.0 && high > bound { high = bound; }
    var low = 0.0;
    var r = 0.5 * high;
    for (var iteration = 0u; iteration < PROBE_INVERSE_ITERATIONS; iteration += 1u) {
        var value = 0.0;
        var tangent = 0.0;
        for (var record = 0u; record < range.y; record += 1u) {
            let word = range.x + record * TEMPORAL_COEFFICIENT_WORDS;
            let coefficient = table_float(word + 1u, 0u) * record_factor(word, local_time);
            let response = field_response(word, r);
            value += coefficient * response.x * r;
            tangent += coefficient * response.y;
        }
        let residual = value - goal;
        if abs(residual) <= PROBE_INVERSE_TOLERANCE * goal { break; }
        if residual < 0.0 { low = r; } else { high = r; }
        if high - low <= 2.0 * 1.1920929e-7 * high { r = 0.5 * (low + high); break; }
        let newton = r - residual / tangent;
        r = select(0.5 * (low + high), newton, newton > low && newton < high);
    }
    return select(-r, r, flux >= 0.0);
}
// `v(b)` at one of the element's samples: the radial solve on a nonlinear
// record, `J b / factor` on a linear one.
fn sample_field(stencil: PointStencil, local: u32, flux: vec2<f32>, local_time: f32) -> vec2<f32> {
    let linear = apply_symmetric(stencil.sample_inverse[local].xyz, flux)
        / sample_temporal_factor(stencil, local, local_time);
    if !temporal_enabled() { return linear; }
    let word = stencil.temporal_complementary.x + local * TEMPORAL_COEFFICIENT_WORDS;
    if field_kind(word) == 0u { return linear; }
    let magnitude = length(flux);
    if magnitude == 0.0 { return vec2<f32>(0.0); }
    let coefficient = record_factor(word, local_time) / stencil.sample_inverse[local].x;
    let floor_value = coefficient * table_float(word + 3u, 3u);
    if !(floor_value > 0.0) { return linear; }
    var high = magnitude / floor_value;
    let bound = table_float(word + 3u, 2u);
    if bound > 0.0 && high > bound { high = bound; }
    var low = 0.0;
    var r = 0.5 * high;
    for (var iteration = 0u; iteration < PROBE_INVERSE_ITERATIONS; iteration += 1u) {
        let response = field_response(word, r);
        let residual = coefficient * response.x * r - magnitude;
        if abs(residual) <= PROBE_INVERSE_TOLERANCE * magnitude { break; }
        if residual < 0.0 { low = r; } else { high = r; }
        if high - low <= 2.0 * 1.1920929e-7 * high { r = 0.5 * (low + high); break; }
        let newton = r - residual / (coefficient * response.y);
        r = select(0.5 * (low + high), newton, newton > low && newton < high);
    }
    return flux * (r / magnitude);
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
        table_float(word + 1u * TEMPORAL_COEFFICIENT_WORDS + 2u, 1u),
        table_float(word + 2u * TEMPORAL_COEFFICIENT_WORDS + 2u, 1u),
        table_float(word + 3u * TEMPORAL_COEFFICIENT_WORDS + 2u, 1u)), stencil.complementary_a)
        + dot(vec4<f32>(
            table_float(word + 4u * TEMPORAL_COEFFICIENT_WORDS + 2u, 1u),
            table_float(word + 5u * TEMPORAL_COEFFICIENT_WORDS + 2u, 1u), 0.0, 0.0), stencil.complementary_b);
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
        field += sample_field(stencil, local, flux[local], local_time) * weights[local];
    }
    return field;
}
// The stored-energy densities of the interpolated fields under the laws at
// the probe point: `c (ḡ(r) r² − G(r))` per row, whose linear form is the
// `c r²/2` a field-linear medium reads.
fn field_store(word: u32, r: f32) -> f32 {
    let response = field_response(word, r);
    return response.x * r * r - response.z;
}
fn complementary_energy(stencil: PointStencil, field: vec2<f32>, local_time: f32) -> f32 {
    let factor = probe_temporal_factor(stencil, false, local_time);
    if temporal_enabled() && field_kind(stencil.temporal_complementary.x) != 0u {
        return factor * stencil.reference_inverse.y
            * field_store(stencil.temporal_complementary.x, length(field));
    }
    return 0.5 * factor * dot(field, apply_symmetric(stencil.reference_inverse.yzw, field));
}
fn primary_energy(stencil: PointStencil, value: f32, local_time: f32) -> f32 {
    let factor = probe_temporal_factor(stencil, true, local_time);
    if temporal_enabled() && field_kind(stencil.temporal_primary_a.x) != 0u {
        return stencil.reference_inverse.x * factor
            * field_store(stencil.temporal_primary_a.x, abs(value));
    }
    return 0.5 * stencil.reference_inverse.x * factor * value * value;
}
fn energy_flow(stencil: PointStencil, value: f32, field: vec2<f32>) -> vec2<f32> {
    return stencil.orientation.x * value * vec2<f32>(-field.y, field.x);
}
fn primary_field(stencil: PointStencil, local_time: f32) -> f32 {
    let a = stencil.nodes_a;
    let b = stencil.nodes_b;
    let values_a = vec4<f32>(
        probe_primary_field(a.x, accepted_q(a.x), local_time),
        probe_primary_field(a.y, accepted_q(a.y), local_time),
        probe_primary_field(a.z, accepted_q(a.z), local_time),
        probe_primary_field(a.w, accepted_q(a.w), local_time));
    let values_b = vec4<f32>(
        probe_primary_field(b.x, accepted_q(b.x), local_time),
        probe_primary_field(b.y, accepted_q(b.y), local_time),
        probe_primary_field(b.z, accepted_q(b.z), local_time), 0.0);
    return dot(values_a, stencil.primary_a) + dot(values_b, stencil.primary_b);
}
fn primary_field_and_rate(stencil: PointStencil) -> vec2<f32> {
    let a = stencil.nodes_a;
    let b = stencil.nodes_b;
    let previous_time = control.clock_f32.y - control.clock_f32.x;
    let value = primary_field(stencil, control.clock_f32.y);
    let previous_a = vec4<f32>(
        probe_primary_field(a.x, previous_q(a.x), previous_time),
        probe_primary_field(a.y, previous_q(a.y), previous_time),
        probe_primary_field(a.z, previous_q(a.z), previous_time),
        probe_primary_field(a.w, previous_q(a.w), previous_time));
    let previous_b_values = vec4<f32>(
        probe_primary_field(b.x, previous_q(b.x), previous_time),
        probe_primary_field(b.y, previous_q(b.y), previous_time),
        probe_primary_field(b.z, previous_q(b.z), previous_time), 0.0);
    let old_value = dot(previous_a, stencil.primary_a)
        + dot(previous_b_values, stencil.primary_b);
    return vec2<f32>(value, (value - old_value) / control.clock_f32.x);
}
fn absolute_time() -> f32 {
    return control.clock_origin.x + control.clock_origin.y + control.clock_f32.y;
}
// ---- end shared canonical point reconstruction ----

@compute @workgroup_size(16)
fn sample_probes(@builtin(local_invocation_id) invocation: vec3<u32>) {
    let probe = invocation.x;
    if probe >= u32(probe_control.values.z) || stencils[probe].sample_valid.y == 0u { return; }
    let stencil = stencils[probe];
    if temporal_enabled() && stencil.sample_valid.z == 0u { return; }
    let time = control.clock_f32.y;
    let primary = primary_field_and_rate(stencil);
    let complement = physical_complement(stencil, accepted_sample_flux(stencil), time);
    let energy = primary_energy(stencil, primary.x, time)
        + complementary_energy(stencil, complement, time);
    let flow = energy_flow(stencil, primary.x, complement);
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
    let time = control.clock_f32.y;
    let primary = primary_field_and_rate(stencil);
    let complement = physical_complement(stencil, accepted_sample_flux(stencil), time);
    // The spare lane holds the pre-filter state at a zero-duration boundary,
    // which is the same instant, so both use the current law.
    let previous_complement = physical_complement(
        stencil, previous_sample_flux(stencil), time);
    let flow = energy_flow(stencil, primary.x, complement);
    output[sample].primary = vec4<f32>(complement, flow);
    output[sample].secondary = vec4<f32>(previous_complement, 0.0, 0.0);
    output[sample].tertiary = bitcast<vec4<f32>>(
        vec4<u32>(control.clock_u32.w, 1u,
            bitcast<u32>(control.clock_origin.x),
            bitcast<u32>(control.clock_origin.y + control.clock_f32.y)));
}
