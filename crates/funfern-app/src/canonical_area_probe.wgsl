struct Control {
    counts_a: vec4<u32>, counts_b: vec4<u32>, counts_c: vec4<u32>,
    table_offsets: vec4<u32>, boundary_offsets: vec4<u32>, clock_u32: vec4<u32>,
    clock_f32: vec4<f32>, clock_origin: vec4<f32>, event: vec4<u32>,
    event_result: vec4<u32>, runtime_serials: vec4<u32>, runtime_slots: vec4<u32>,
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
struct AreaQuadrature {
    primary_a: vec4<f32>, primary_b: vec4<f32>,
    complementary_a: vec4<f32>, complementary_b: vec4<f32>,
    weight: vec4<f32>,
}
struct TableWord { data: vec4<u32> }
struct AreaContribution {
    nodes_a: vec4<u32>, nodes_b: vec4<u32>, sample_valid: vec4<u32>,
    sample_inverse: array<vec4<f32>, 6>,
    node_shares_a: vec4<f32>, node_shares_b: vec4<f32>,
    temporal_primary_a: vec4<u32>, temporal_primary_b: vec4<u32>,
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
// Only the element pass binds the law tables; only the reduction binds the
// descriptors and the output ring. Two layouts over one set of numbers keeps
// each pass inside the portable eight-storage-buffer limit.
@group(0) @binding(8) var<storage, read> tables: array<TableWord>;

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
// Mirrors the shared block in canonical_probe.wgsl. The structures here are
// the area recorder's own, so the arithmetic is repeated rather than the text
// shared; a test pins the parts that must agree.
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

fn apply_symmetric(tensor: vec3<f32>, value: vec2<f32>) -> vec2<f32> {
    return vec2<f32>(tensor.x * value.x + tensor.y * value.y,
        tensor.y * value.x + tensor.z * value.y);
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
fn record_factor(word: u32, local_time: f32) -> f32 {
    return temporal_factor(tables[word].data, tables[word + 1u].data,
        table_float(word + 2u, 0u), table_float(word + 2u, 1u), local_time);
}
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
// `(v(b), stored energy)` at one of the element's samples, its weight
// included: the radial solve and `W(|b| r − c G(r))` on a nonlinear record,
// `J b / factor` and `½ W b·v` on a linear one.
fn area_sample(contribution: AreaContribution, local: u32, flux: vec2<f32>, local_time: f32) -> vec3<f32> {
    let inverse = contribution.sample_inverse[local];
    let linear = apply_symmetric(inverse.xyz, flux) / sample_factor(contribution, local, local_time);
    let linear_energy = 0.5 * inverse.w * dot(flux, linear);
    if !temporal_enabled() { return vec3<f32>(linear, linear_energy); }
    let word = contribution.temporal_primary_b.w + local * TEMPORAL_COEFFICIENT_WORDS;
    if field_kind(word) == 0u { return vec3<f32>(linear, linear_energy); }
    let magnitude = length(flux);
    if magnitude == 0.0 { return vec3<f32>(0.0); }
    let coefficient = record_factor(word, local_time) / inverse.x;
    let floor_value = coefficient * table_float(word + 3u, 3u);
    if !(floor_value > 0.0) { return vec3<f32>(linear, linear_energy); }
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
    let energy = inverse.w * (magnitude * r - coefficient * field_response(word, r).z);
    return vec3<f32>(flux * (r / magnitude), energy);
}
// The stored energy this element's contribution to local node `local` holds:
// `c (ḡ(U) U² − G(U))`, the node's energy split over its materials, whose
// linear form is `½ c U²`.
fn contribution_word(contribution: AreaContribution, local: u32) -> u32 {
    switch local {
        case 0u: { return contribution.temporal_primary_a.x; }
        case 1u: { return contribution.temporal_primary_a.y; }
        case 2u: { return contribution.temporal_primary_a.z; }
        case 3u: { return contribution.temporal_primary_a.w; }
        case 4u: { return contribution.temporal_primary_b.x; }
        case 5u: { return contribution.temporal_primary_b.y; }
        default: { return contribution.temporal_primary_b.z; }
    }
}
fn contribution_store(contribution: AreaContribution, local: u32, field: f32) -> f32 {
    if !temporal_enabled() { return 0.5 * field * field; }
    let word = contribution_word(contribution, local);
    let r = abs(field);
    let response = field_response(word, r);
    return response.x * r * r - response.z;
}
// This element's factor on its own contribution to local node `local`.
fn contribution_factor(contribution: AreaContribution, local: u32, local_time: f32) -> f32 {
    if !temporal_enabled() { return 1.0; }
    var word = contribution.temporal_primary_a.x;
    switch local {
        case 0u: { word = contribution.temporal_primary_a.x; }
        case 1u: { word = contribution.temporal_primary_a.y; }
        case 2u: { word = contribution.temporal_primary_a.z; }
        case 3u: { word = contribution.temporal_primary_a.w; }
        case 4u: { word = contribution.temporal_primary_b.x; }
        case 5u: { word = contribution.temporal_primary_b.y; }
        default: { word = contribution.temporal_primary_b.z; }
    }
    return record_factor(word, local_time);
}
fn sample_factor(contribution: AreaContribution, local: u32, local_time: f32) -> f32 {
    if !temporal_enabled() { return 1.0; }
    return record_factor(
        contribution.temporal_primary_b.w + local * TEMPORAL_COEFFICIENT_WORDS, local_time);
}

@compute @workgroup_size(64)
fn sample_area_elements(@builtin(global_invocation_id) invocation: vec3<u32>) {
    let element = invocation.x;
    if element >= u32(probe_control.values.w) { return; }
    let contribution = contributions[element];
    if contribution.sample_valid.y == 0u { return; }
    if temporal_enabled() && contribution.sample_valid.z == 0u { return; }
    let time = control.clock_f32.y;
    let a = contribution.nodes_a;
    let b = contribution.nodes_b;
    // Each node's field through the assembled map the solver inverts; on a
    // linear node that is the division by its mass.
    let primary_a = vec4<f32>(
        probe_primary_field(a.x, accepted_q(a.x), time),
        probe_primary_field(a.y, accepted_q(a.y), time),
        probe_primary_field(a.z, accepted_q(a.z), time),
        probe_primary_field(a.w, accepted_q(a.w), time));
    let primary_b = vec4<f32>(
        probe_primary_field(b.x, accepted_q(b.x), time),
        probe_primary_field(b.y, accepted_q(b.y), time),
        probe_primary_field(b.z, accepted_q(b.z), time), 0.0);
    // Recover the physical field where the solver owns an inverse, once per
    // sample, and interpolate it at the quadrature points below.
    let start = contribution.sample_valid.x;
    var sample_field: array<vec2<f32>, 6>;
    var sample_energy = 0.0;
    for (var local = 0u; local < 6u; local += 1u) {
        let sample = area_sample(contribution, local, accepted_b(start + local), time);
        sample_field[local] = sample.xy;
        sample_energy += sample.z;
    }
    let field_x_a = vec4<f32>(sample_field[0].x, sample_field[1].x,
        sample_field[2].x, sample_field[3].x);
    let field_y_a = vec4<f32>(sample_field[0].y, sample_field[1].y,
        sample_field[2].y, sample_field[3].y);
    let field_x_b = vec4<f32>(sample_field[4].x, sample_field[5].x, 0.0, 0.0);
    let field_y_b = vec4<f32>(sample_field[4].y, sample_field[5].y, 0.0, 0.0);
    var accumulated = vec4<f32>(0.0);
    for (var point = 0u; point < 12u; point += 1u) {
        let quadrature = contribution.quadrature[point];
        let primary = dot(primary_a, quadrature.primary_a) + dot(primary_b, quadrature.primary_b);
        let complement = vec2<f32>(
            dot(field_x_a, quadrature.complementary_a) + dot(field_x_b, quadrature.complementary_b),
            dot(field_y_a, quadrature.complementary_a) + dot(field_y_b, quadrature.complementary_b));
        let weight = quadrature.weight.x;
        accumulated += vec4<f32>(weight * primary, weight * primary * primary,
            weight * dot(complement, complement), 0.0);
    }

    // Reported energy is the solver's own discrete energy restricted to this
    // piece: each node's stored energy apportioned to this element by its own
    // contribution to that node's map, plus the element's sample energies.
    // Only this agrees with the accounting lanes at full coverage.
    let shares_a = contribution.node_shares_a * vec4<f32>(
        contribution_factor(contribution, 0u, time),
        contribution_factor(contribution, 1u, time),
        contribution_factor(contribution, 2u, time),
        contribution_factor(contribution, 3u, time));
    let shares_b = vec4<f32>(
        contribution.node_shares_b.x * contribution_factor(contribution, 4u, time),
        contribution.node_shares_b.y * contribution_factor(contribution, 5u, time),
        contribution.node_shares_b.z * contribution_factor(contribution, 6u, time),
        0.0);
    let stores_a = vec4<f32>(
        contribution_store(contribution, 0u, primary_a.x),
        contribution_store(contribution, 1u, primary_a.y),
        contribution_store(contribution, 2u, primary_a.z),
        contribution_store(contribution, 3u, primary_a.w));
    let stores_b = vec4<f32>(
        contribution_store(contribution, 4u, primary_b.x),
        contribution_store(contribution, 5u, primary_b.y),
        contribution_store(contribution, 6u, primary_b.z), 0.0);
    let energy = dot(stores_a, shares_a) + dot(stores_b, shares_b) + sample_energy;
    accumulated.w = contribution.node_shares_b.w * energy;
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
