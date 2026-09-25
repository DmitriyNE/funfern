// Canonical direct-state f32 solver. Rust layout version 5.
const LAYOUT_VERSION: u32 = 5u;
const STATE_WORD_STRIDE: u32 = 16u;
const NODE_STRIDE: u32 = 96u;
const SAMPLE_STRIDE: u32 = 112u;
const TABLE_WORD_STRIDE: u32 = 16u;
const WORKGROUP_SIZE: u32 = 128u;
const MAX_TRACE: u32 = 1024u;
const MODE_WORDS: u32 = 12u;
const NO_INDEX: u32 = 0xffffffffu;
const FORCE_KIND_GAP: u32 = 1u;
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
// Safeguarded Newton for the constitutive inverse. The tolerance is the
// device criterion fixed in Stage 8 (`F32_INVERSE_TOLERANCE`); the cap turns
// a pathological solve into a detected failure, never an accepted guess.
const INVERSE_TOLERANCE: f32 = 4.76837158e-7;
const INVERSE_ITERATIONS: u32 = 40u;
const SNAPSHOT_METADATA_MAGIC: f32 = 8675309.0;

const STATUS_LAYOUT: u32 = 1u;
const STATUS_TIMESTEP: u32 = 2u;
const STATUS_INVERSE_DOMAIN: u32 = 3u;
const STATUS_NON_FINITE: u32 = 4u;
const STATUS_INVERSE_CONVERGENCE: u32 = 5u;
// Gate O: a node's integrated field passed its restoring law's declared bound.
const STATUS_RESTORING_DOMAIN: u32 = 6u;
const TEMPORAL_LOSS_VAN_DER_POL: u32 = 16u;
const RESTORING_KLEIN_GORDON: u32 = 1u;
const RESTORING_SINE_GORDON: u32 = 2u;
const RESTORING_PHI4: u32 = 3u;
// Leave serialization headroom below f32::MAX: Naga's decimal WGSL writer
// rounds the exact maximum upward, which Chrome correctly rejects.
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
    // Gate O: accepted and candidate active gain (x); the rest is reserved.
    accepted_accounting_c: vec4<f32>,
    candidate_accounting_c: vec4<f32>,
}

struct Status {
    candidate: atomic<u32>,
    latch: atomic<u32>,
    injection: atomic<u32>,
    injection_index: atomic<u32>,
    handoff: atomic<u32>,
    transaction_1: atomic<u32>,
    transaction_2: atomic<u32>,
    transaction_3: atomic<u32>,
}

struct StateWord { values: vec4<f32> }

struct Node {
    mass_loss: vec4<f32>,
    ranges: vec4<u32>,
    boundary: vec4<u32>,
    prescribed: vec4<f32>,
    damping_support: vec4<f32>,
    stiffness: vec4<u32>,
}

struct Sample {
    nodes_a: vec4<u32>,
    nodes_b: vec4<u32>,
    curls_01: vec4<f32>,
    curls_23: vec4<f32>,
    curls_45: vec4<f32>,
    curl_6_loss: vec4<f32>,
    constitutive: vec4<f32>,
}

struct TableWord { data: vec4<u32> }
struct ScratchWord { values: vec4<f32> }

@group(0) @binding(0) var<storage, read_write> control: Control;
@group(0) @binding(1) var<storage, read_write> status: Status;
@group(0) @binding(2) var<storage, read_write> state: array<StateWord>;
@group(0) @binding(3) var<storage, read_write> nodes: array<Node>;
@group(0) @binding(4) var<storage, read_write> samples: array<Sample>;
@group(0) @binding(5) var<storage, read_write> tables: array<TableWord>;
@group(0) @binding(6) var<storage, read_write> scratch: array<ScratchWord>;
@group(0) @binding(7) var<storage, read_write> boundary: array<TableWord>;

// Only the accounting reduction needs workgroup memory. Boundary transforms
// use globally ordered dispatches so large traces occupy the whole device.
var<workgroup> mode_values: array<vec2<f32>, 128>;
var<workgroup> reduced_values: array<f32, 128>;
var<workgroup> solved_values: array<f32, 128>;
// A control flag a workgroup branches on around a barrier. `control` is
// read-write storage, so WGSL's uniformity analysis takes anything loaded
// from it as possibly different per invocation, and a barrier under such a
// branch fails the whole module in the browser, though naga accepts it.
// `workgroupUniformLoad` of this copy is uniform by definition.
var<workgroup> uniform_flag: u32;

fn stopped() -> bool {
    return atomicLoad(&status.latch) != 0u || atomicLoad(&status.candidate) != 0u;
}

fn reject(reason: u32) {
    atomicMax(&status.candidate, reason);
}

fn finite_scalar(value: f32) -> bool {
    return value >= -MAX_FINITE && value <= MAX_FINITE;
}

fn finite_vector(value: vec4<f32>) -> bool {
    return all(value >= vec4<f32>(-MAX_FINITE))
        && all(value <= vec4<f32>(MAX_FINITE));
}

fn primary_offset() -> u32 { return 0u; }
fn complementary_offset() -> u32 { return control.counts_a.x; }
fn auxiliary_offset() -> u32 { return control.counts_a.x + control.counts_a.y; }
fn mode_temporary_offset() -> u32 { return control.counts_a.w; }
fn mode_accounting_offset() -> u32 { return control.counts_a.w + control.counts_b.z; }
fn mode_modal_offset() -> u32 {
    return control.counts_a.w + 2u * control.counts_b.z;
}
fn trace_solution_offset() -> u32 {
    return control.counts_a.w + 3u * control.counts_b.z;
}
fn scratch_count() -> u32 {
    return trace_solution_offset() + control.counts_b.y;
}
fn accounting_item_count() -> u32 {
    return control.counts_a.x + control.counts_a.y + control.counts_b.z;
}
fn accounting_bank_offset(slot: u32) -> u32 {
    return scratch_count() + slot * accounting_item_count();
}
// One word per trace, after both accounting banks, on a generation whose wall
// carries a field law: `(Q_k, ū_k, 2g_k, last Newton step)`.
fn nonlinear_trace_offset() -> u32 {
    return scratch_count() + 2u * accounting_item_count();
}
fn nonlinear_trace() -> bool { return (control.boundary_offsets.w & 16u) != 0u; }
// Any record carries a field law.
fn field_laws() -> bool { return (control.boundary_offsets.w & 32u) != 0u; }
// Per-site words of every time-driven generation. A field-dependent one
// caches in them, one solve per site and stage, the nodal field at the
// drift's midpoint and each sample's secant `r/(j|b|)` at the kick's instant,
// so every sample reads a node's field, and every node a sample's secant,
// without solving it again. The grid filter borrows the other lanes for its
// frozen maps at the event instant.
fn node_field_offset() -> u32 { return nonlinear_trace_offset() + control.counts_b.y; }
fn sample_secant_offset() -> u32 { return node_field_offset() + control.counts_a.x; }
fn has_loss_stages() -> bool { return (control.boundary_offsets.w & 1u) != 0u; }
// A time-driven generation carrying loss reads its rates from loss records.
fn loss_records() -> bool { return (control.boundary_offsets.w & 64u) != 0u; }
// Gate O: the generation carries the integrated field `r`, one auxiliary lane
// per node at the tail of the auxiliary block.
fn restoring() -> bool { return (control.boundary_offsets.w & 128u) != 0u; }
fn integrated_index(node: u32) -> u32 {
    return control.counts_a.z - control.counts_a.x + node;
}
fn use_force_cache() -> bool { return (control.boundary_offsets.w & 4u) != 0u; }
fn has_prescribed_trace() -> bool { return (control.boundary_offsets.w & 8u) != 0u; }
fn accepted_slot() -> u32 { return control.event.z & 1u; }
fn event_operation() -> u32 { return control.event.z >> 8u; }
fn live_event() -> bool { return (control.event.z & 2u) != 0u; }

const TEMPORAL_RUNTIME_WORDS_PER_SLOT: u32 = 3u;

// The accepted material runtime travels in the state buffer, after the
// metadata word, rather than beside it. A Switch origin is stamped at a
// commit boundary and a carrier is re-anchored at one, so a consumer needs
// them from the same copy as the fields they explain; a separately arriving
// runtime copy would be status, not snapshot identity.
fn publish_snapshot_metadata() {
    state[control.counts_a.w].values = vec4<f32>(
        SNAPSHOT_METADATA_MAGIC,
        f32(accepted_slot()),
        f32(control.clock_u32.w & 0xffffu),
        f32(control.clock_u32.w >> 16u));
    if !temporal_enabled() { return; }
    let header = tables[control.runtime_slots.z].data;
    let shape = tables[control.runtime_slots.z + 1u].data;
    let slot = control.runtime_slots.y & 1u;
    for (var record = 0u; record < shape.x; record += 1u) {
        let from_word = header.w + record * shape.z
            + slot * TEMPORAL_RUNTIME_WORDS_PER_SLOT;
        let into_word = control.counts_a.w + 1u
            + record * TEMPORAL_RUNTIME_WORDS_PER_SLOT;
        for (var word = 0u; word < TEMPORAL_RUNTIME_WORDS_PER_SLOT; word += 1u) {
            state[into_word + word].values =
                bitcast<vec4<f32>>(tables[from_word + word].data);
        }
    }
}

fn inject_at(state_word: u32) {
    let injection = atomicLoad(&status.injection);
    if injection != 0u && state_word == atomicLoad(&status.injection_index) {
        reject(injection);
    }
}

fn accepted_q(node: u32) -> f32 {
    return select(state[node].values.x, state[node].values.y, accepted_slot() != 0u);
}
fn candidate_q(node: u32) -> f32 {
    return select(state[node].values.y, state[node].values.x, accepted_slot() != 0u);
}
fn set_candidate_q(node: u32, value: f32) {
    if accepted_slot() == 0u { state[node].values.y = value; }
    else { state[node].values.x = value; }
}
fn accepted_force(node: u32) -> f32 {
    return select(state[node].values.z, state[node].values.w, accepted_slot() != 0u);
}
fn candidate_force(node: u32) -> f32 {
    return select(state[node].values.w, state[node].values.z, accepted_slot() != 0u);
}
fn set_candidate_force(node: u32, value: f32) {
    if accepted_slot() == 0u { state[node].values.w = value; }
    else { state[node].values.z = value; }
}
fn accepted_b(sample: u32) -> vec2<f32> {
    return select(
        state[complementary_offset() + sample].values.xy,
        state[complementary_offset() + sample].values.zw,
        accepted_slot() != 0u);
}
fn candidate_b(sample: u32) -> vec2<f32> {
    return select(
        state[complementary_offset() + sample].values.zw,
        state[complementary_offset() + sample].values.xy,
        accepted_slot() != 0u);
}
fn set_candidate_b(sample: u32, value: vec2<f32>) {
    let word = complementary_offset() + sample;
    if accepted_slot() == 0u {
        state[word].values.z = value.x;
        state[word].values.w = value.y;
    } else {
        state[word].values.x = value.x;
        state[word].values.y = value.y;
    }
}
fn accepted_auxiliary(index: u32) -> f32 {
    let value = state[auxiliary_offset() + index].values;
    return select(value.x, value.y, accepted_slot() != 0u);
}
fn candidate_auxiliary(index: u32) -> f32 {
    let value = state[auxiliary_offset() + index].values;
    return select(value.y, value.x, accepted_slot() != 0u);
}
fn set_candidate_auxiliary(index: u32, value: f32) {
    let word = auxiliary_offset() + index;
    if accepted_slot() == 0u { state[word].values.y = value; }
    else { state[word].values.x = value; }
}

fn harmonic_value(signal: vec4<f32>, local_time: f32) -> f32 {
    return signal.x + signal.y * sin(signal.w + signal.z * local_time);
}

fn sinc(value: f32) -> f32 {
    if abs(value) < 0.0005 {
        let square = value * value;
        return 1.0 - square / 6.0 + square * square / 120.0;
    }
    return sin(value) / value;
}

fn reduced_phase(value: f32) -> f32 {
    return atan2(sin(value), cos(value));
}

fn add_compensated(high: f32, low: f32, increment: f32) -> vec2<f32> {
    let sum = high + increment;
    let residual = (high - sum) + increment + low;
    let next_high = sum + residual;
    return vec2<f32>(next_high, (sum - next_high) + residual);
}

fn table_float(word: u32, lane: u32) -> f32 {
    return bitcast<f32>(tables[word].data[lane]);
}

fn temporal_enabled() -> bool {
    return (control.runtime_slots.w & 1u) != 0u;
}

fn smootherstep(value: f32) -> f32 {
    return value * value * value
        * (value * (value * 6.0 - 15.0) + 10.0);
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

fn temporal_factor(coefficient_word: u32, local_time: f32) -> f32 {
    let metadata = tables[coefficient_word].data;
    let runtime_header = tables[control.runtime_slots.z].data;
    let runtime_word = runtime_header.w + 6u * metadata.x
        + 3u * (control.runtime_slots.y & 1u);
    let phase = table_float(runtime_word, metadata.y)
        + table_float(coefficient_word + 1u, 3u) * local_time;
    let carrier = reduced_phase(phase);
    let depth = table_float(coefficient_word + 1u, 2u);
    let shape = table_float(coefficient_word + 2u, 0u);
    let spatial_phase = table_float(coefficient_word + 2u, 1u);
    var drive = 1.0;
    switch metadata.z {
        case TEMPORAL_DRIVE_NONE: {}
        case TEMPORAL_DRIVE_PUMP: {
            drive += depth * cos(carrier);
        }
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
        // Packed metadata admits only the four cases above. Zero makes any
        // corrupted record fail the subsequent positive-factor/non-finite
        // candidate validation without embedding a NaN constant, which WebGPU
        // shader modules reject even in an unreachable branch.
        default: { return 0.0; }
    }
    let blend = temporal_switch_blend(runtime_word, local_time);
    var switch_factor = 1.0;
    if (metadata.w & TEMPORAL_HAS_ALTERNATE) != 0u {
        let alternate = table_float(coefficient_word + 1u, 1u);
        switch_factor += blend * (alternate - 1.0);
    }
    let factor = drive * switch_factor;
    return select(factor, 1.0 / factor, (metadata.w & TEMPORAL_INVERTED) != 0u);
}

fn temporal_primary_mass(node: u32, local_time: f32) -> f32 {
    let range = nodes[node].stiffness.zw;
    var mass = 0.0;
    for (var record = 0u; record < range.y; record += 1u) {
        let word = range.x + record * TEMPORAL_COEFFICIENT_WORDS;
        mass += table_float(word + 1u, 0u) * temporal_factor(word, local_time);
    }
    return mass;
}

fn temporal_inverse_primary_mass(node: u32, local_time: f32) -> f32 {
    return 1.0 / temporal_primary_mass(node, local_time);
}

// `1 − e^{−x}`, kept accurate as `x → 0`, where f32 would cancel it away.
// The share multiplies the flux every stage, so what matters is its absolute
// error, and the direct form's is half an f32 unit of one whatever `x` is,
// with the same sign stage after stage. Its series therefore runs up to
// 0.02, as its siblings' do: switching at 1e-3 left a common half-step rate
// such as 3.3e-3 (1/s at h = 6.6e-3) biased by 2e-8 a stage, a linear drift
// of 4e-5 in 1000 steps against the reference.
fn one_minus_exp_neg(x: f32) -> f32 {
    return -exp_minus_one(-x);
}

// A node's loss rate at `local_time`: its materials' rates weighed by the
// masses in force, `Σ c_r(t) γ_r(t) / Σ c_r(t)`. A node shared by a pumped
// lossy material and a still one is not the rate either has alone, and the
// weight moves with the pump. Each loss record sits as far into the loss
// block as its coefficient record sits into the coefficient block.
fn node_loss_rate(node: u32, local_time: f32) -> f32 {
    let header = tables[control.runtime_slots.z].data;
    let losses = tables[control.runtime_slots.z + 2u].data;
    let range = nodes[node].stiffness.zw;
    var mass = 0.0;
    var weighted = 0.0;
    for (var record = 0u; record < range.y; record += 1u) {
        let word = range.x + record * TEMPORAL_COEFFICIENT_WORDS;
        let loss = losses.x + (word - header.x);
        let coefficient = table_float(word + 1u, 0u) * temporal_factor(word, local_time);
        mass += coefficient;
        weighted += coefficient * table_float(loss + 1u, 0u) * temporal_factor(loss, local_time);
    }
    return weighted / mass;
}

fn sample_loss_rate(sample: u32, local_time: f32) -> f32 {
    let header = tables[control.runtime_slots.z].data;
    let losses = tables[control.runtime_slots.z + 2u].data;
    let loss = losses.y + (samples[sample].nodes_b.w - header.z);
    return table_float(loss + 1u, 0u) * temporal_factor(loss, local_time);
}

// Gate O: a node carrying van der Pol, whose loss stage is the exact
// Bernoulli map and whose energy change is active gain.
fn node_is_active(node: u32) -> bool { return nodes[node].boundary.w != 0u; }

// Gate O: where each sample's short-wave viscosity `τ` sits, four to a word,
// or 0 when no self-oscillating law acts. The drift leaves `τ η C u` in the
// sample's scratch `zw`, and an active node's second kick gathers it; see
// `short_wave_viscosity` on the reference.
fn short_wave_offset() -> u32 {
    if !temporal_enabled() { return 0u; }
    return tables[control.runtime_slots.z + 2u].data.w;
}

// `e^{−x}`'s complement over `x`, `(1 − e^{−x})/x`, by its series where f32
// would cancel the direct form away.
fn one_minus_exp_neg_over(x: f32) -> f32 {
    if abs(x) < 0.02 {
        return 1.0 - x * (0.5 - x * (1.0 / 6.0 - x * (1.0 / 24.0 - x / 120.0)));
    }
    return (1.0 - exp(-x)) / x;
}

// A van der Pol node over one half step, exactly: `Q̇ = −(β + kQ²)Q` is the
// Bernoulli equation `ẏ = −2(β + ky)y` in `y = Q²`, whose solution keeps
// `Q`'s sign. `β` and `k = α/M²` weigh the node's materials by the masses in
// force at the half interval's midpoint, as the reference does.
fn active_loss_map(node: u32, flux: f32, second: bool) -> f32 {
    let header = tables[control.runtime_slots.z].data;
    let losses = tables[control.runtime_slots.z + 2u].data;
    let range = nodes[node].stiffness.zw;
    let duration = 0.5 * control.clock_f32.x;
    let rate_time = control.clock_f32.y + select(0.25, 0.75, second) * control.clock_f32.x;
    var mass = 0.0;
    var beta = 0.0;
    var alpha = 0.0;
    for (var record = 0u; record < range.y; record += 1u) {
        let word = range.x + record * TEMPORAL_COEFFICIENT_WORDS;
        let loss = losses.x + (word - header.x);
        let coefficient = table_float(word + 1u, 0u) * temporal_factor(word, rate_time);
        let rate = table_float(loss + 1u, 0u);
        mass += coefficient;
        if (tables[loss].data.w & TEMPORAL_LOSS_VAN_DER_POL) != 0u {
            beta -= coefficient * rate;
            alpha += coefficient * rate * table_float(loss + 3u, 0u);
        } else {
            beta += coefficient * rate * temporal_factor(loss, rate_time);
        }
    }
    beta /= mass;
    let k = alpha / (mass * mass * mass);
    let x = 2.0 * beta * duration;
    let s = k * flux * flux * 2.0 * duration * one_minus_exp_neg_over(x);
    // `Q·e^{−x/2}/√(1 + s)` as `Q + Q·(d₁ + d₂ + d₁d₂)`: the multiplier sits
    // within a few parts in a thousand of one and rounds the same way every
    // stage, so it is formed as its small departures, as the passive map is.
    let d1 = exp_minus_one(-0.5 * x);
    let root = sqrt(1.0 + s);
    let d2 = -s / (root * (1.0 + root));
    return flux + flux * (d1 + d2 + d1 * d2);
}

// `e^{z} − 1`, by its series where f32 would cancel the direct form away.
// Past 0.02 the direct form is off by up to half an f32 unit of one, with the
// same sign every stage: a rate that fast drifts from the reference by about
// 1.2e-7 a step, a rate error of a few 1e-5/s. Not fully accurate, and left
// so; see "Worth checking sometime" in `docs/plan.md`.
fn exp_minus_one(z: f32) -> f32 {
    if abs(z) < 0.02 {
        return z * (1.0 + z * (0.5 + z * (1.0 / 6.0 + z * (1.0 / 24.0 + z / 120.0))));
    }
    return exp(z) - 1.0;
}

// The share one half-step of loss takes away. Each half map stands for its
// own half interval, so its rate is read at that interval's midpoint, as the
// reference reads it; packed fractions serve generations whose rates cannot
// move.
fn node_loss_fraction(node: u32, second: bool) -> f32 {
    if !loss_records() { return nodes[node].mass_loss.z; }
    let rate_time = control.clock_f32.y + select(0.25, 0.75, second) * control.clock_f32.x;
    return one_minus_exp_neg(0.5 * control.clock_f32.x * node_loss_rate(node, rate_time));
}

fn sample_loss_fraction(sample: u32, second: bool) -> f32 {
    if !loss_records() { return samples[sample].curl_6_loss.z; }
    let rate_time = control.clock_f32.y + select(0.25, 0.75, second) * control.clock_f32.x;
    return one_minus_exp_neg(0.5 * control.clock_f32.x * sample_loss_rate(sample, rate_time));
}

// Stored energy of a node and of a sample at `local_time`, through the maps
// in force: what a loss stage takes away is charged at those, not at the
// authored linear stores.
fn stage_primary_energy(node: u32, flux: f32, local_time: f32) -> f32 {
    if temporal_enabled() { return temporal_primary_energy(node, flux, local_time); }
    return 0.5 * flux * flux * nodes[node].mass_loss.y;
}

fn stage_complementary_energy(sample: u32, flux: vec2<f32>, local_time: f32) -> f32 {
    if temporal_enabled() { return temporal_complementary_energy(sample, flux, local_time); }
    return b_energy(sample, flux);
}

fn temporal_complementary_factor(sample: u32, local_time: f32) -> f32 {
    return temporal_factor(samples[sample].nodes_b.w, local_time);
}

// ---------------------------------------------------------------------------
// Field-dependent response: the executed Kerr and saturable maps
// ---------------------------------------------------------------------------
//
// A record's field law sits in its flag bits; its fourth word holds
// `(χ, saturation, amplitude bound or 0, minimum ḡ)`. Every executed law is
// even, so it reads `r = |field|`. These mirror `FieldLawValues` in the core.

fn field_kind(word: u32) -> u32 {
    return tables[word].data.w & TEMPORAL_FIELD_MASK;
}

// `ḡ(r)`, `ḡ + rḡ′` and the co-energy `∫₀ʳ ḡ(s)s ds`, as one vector.
fn field_response(word: u32, r: f32) -> vec3<f32> {
    let chi = table_float(word + 3u, 0u);
    let square = r * r;
    switch field_kind(word) {
        case TEMPORAL_FIELD_KERR: {
            return vec3<f32>(
                1.0 + chi * square,
                1.0 + 3.0 * chi * square,
                0.5 * square + 0.25 * chi * square * square);
        }
        case TEMPORAL_FIELD_SATURABLE: {
            let saturation = table_float(word + 3u, 1u);
            let sigma2 = saturation * saturation;
            let x = square / sigma2;
            let denominator = 1.0 + x;
            // `x − ln(1 + x)` cancels for small `x`; its series takes over
            // well before f32 loses the difference.
            var excess: f32;
            if x < 0.03 {
                excess = x * x * (0.5 - x * (1.0 / 3.0 - x * (0.25 - x * 0.2)));
            } else {
                excess = x - log(denominator);
            }
            return vec3<f32>(
                1.0 + chi * square / denominator,
                1.0 + chi * square * (x + 3.0) / (denominator * denominator),
                0.5 * square + 0.5 * chi * sigma2 * sigma2 * excess);
        }
        default: { return vec3<f32>(1.0, 1.0, 0.5 * square); }
    }
}

fn node_is_nonlinear(node: u32) -> bool {
    if !field_laws() { return false; }
    let range = nodes[node].stiffness.zw;
    for (var record = 0u; record < range.y; record += 1u) {
        if field_kind(range.x + record * TEMPORAL_COEFFICIENT_WORDS) != 0u {
            return true;
        }
    }
    return false;
}

// The node's assembled map at `r`: `(P(r), P′(r), ∫₀ʳ P)`, summed over every
// material that meets there.
fn primary_site(node: u32, local_time: f32, r: f32) -> vec3<f32> {
    let range = nodes[node].stiffness.zw;
    var total = vec3<f32>(0.0);
    for (var record = 0u; record < range.y; record += 1u) {
        let word = range.x + record * TEMPORAL_COEFFICIENT_WORDS;
        let coefficient = table_float(word + 1u, 0u) * temporal_factor(word, local_time);
        let response = field_response(word, r);
        total += coefficient * vec3<f32>(response.x * r, response.y, response.z);
    }
    return total;
}

// `(Σ c ḡ_min, tightest amplitude bound or 0)`: the solve's bracket.
fn primary_bracket(node: u32, local_time: f32) -> vec2<f32> {
    let range = nodes[node].stiffness.zw;
    var floor_value = 0.0;
    var bound = 0.0;
    for (var record = 0u; record < range.y; record += 1u) {
        let word = range.x + record * TEMPORAL_COEFFICIENT_WORDS;
        let coefficient = table_float(word + 1u, 0u) * temporal_factor(word, local_time);
        var minimum = 1.0;
        if field_kind(word) != 0u {
            minimum = table_float(word + 3u, 3u);
            let own = table_float(word + 3u, 2u);
            if own > 0.0 && (bound == 0.0 || own < bound) { bound = own; }
        }
        floor_value += coefficient * minimum;
    }
    return vec2<f32>(floor_value, bound);
}

// `r` solving `value(r) = goal` on `[0, high]` for an increasing map
// whose value, tangent and bound the caller supplies through `primary_site`
// or the single-record complementary form below. Returns -1 on failure, with
// the status already raised.
fn solve_primary_radius(node: u32, local_time: f32, goal: f32) -> f32 {
    let bracket = primary_bracket(node, local_time);
    if !(bracket.x > 0.0) {
        reject(STATUS_INVERSE_DOMAIN);
        return -1.0;
    }
    var high = goal / bracket.x;
    if bracket.y > 0.0 && high > bracket.y {
        if primary_site(node, local_time, bracket.y).x < goal * (1.0 - INVERSE_TOLERANCE) {
            reject(STATUS_INVERSE_DOMAIN);
            return -1.0;
        }
        high = bracket.y;
    }
    var low = 0.0;
    var r = 0.5 * high;
    for (var iteration = 0u; iteration < INVERSE_ITERATIONS; iteration += 1u) {
        let site = primary_site(node, local_time, r);
        let residual = site.x - goal;
        if abs(residual) <= INVERSE_TOLERANCE * goal { return r; }
        if residual < 0.0 { low = r; } else { high = r; }
        if high - low <= 2.0 * 1.1920929e-7 * high { return 0.5 * (low + high); }
        let newton = r - residual / site.y;
        r = select(0.5 * (low + high), newton, newton > low && newton < high);
    }
    reject(STATUS_INVERSE_CONVERGENCE);
    return -1.0;
}

// The primary field `U = P⁻¹(Q)` at `local_time`. A node without a field law
// keeps the linear division, arithmetic unchanged.
fn temporal_primary_field(node: u32, flux: f32, local_time: f32) -> f32 {
    if !node_is_nonlinear(node) {
        return flux * temporal_inverse_primary_mass(node, local_time);
    }
    let goal = abs(flux);
    if goal == 0.0 { return 0.0; }
    let r = solve_primary_radius(node, local_time, goal);
    if r < 0.0 { return 0.0; }
    return select(-r, r, flux >= 0.0);
}

// The flux a node holds at `field`: `sign·P(|field|)`. A field past a declared
// bound has no flux on this branch of the map and is refused, as the inverse
// refuses the flux that would hold it.
fn temporal_primary_flux_of_field(node: u32, field: f32, local_time: f32) -> f32 {
    if !node_is_nonlinear(node) {
        return temporal_primary_mass(node, local_time) * field;
    }
    let bound = primary_bracket(node, local_time).y;
    if bound > 0.0 && abs(field) > bound {
        reject(STATUS_INVERSE_DOMAIN);
        return 0.0;
    }
    let magnitude = primary_site(node, local_time, abs(field)).x;
    return select(-magnitude, magnitude, field >= 0.0);
}

// `(ū, dū/dQ)` for the kick from `old` to `flux` at one node: the mean of
// `U` over the interval by two-point Gauss-Legendre, which is the reference's
// discrete gradient `ΔT/ΔQ` to an error of the fourth power of the step, and
// its exact derivative in `flux`. The quotient itself would lose to f32
// cancellation what the rule does not.
fn trace_gradient(node: u32, old: f32, flux: f32, local_time: f32) -> vec2<f32> {
    let middle = 0.5 * (old + flux);
    let half = 0.5 * (flux - old) * 0.57735027;
    let left = temporal_primary_field(node, middle - half, local_time);
    let right = temporal_primary_field(node, middle + half, local_time);
    let left_slope = 1.0 / primary_site(node, local_time, abs(left)).y;
    let right_slope = 1.0 / primary_site(node, local_time, abs(right)).y;
    let inner = 0.5 - 0.5 * 0.57735027;
    let outer = 0.5 + 0.5 * 0.57735027;
    return vec2<f32>(
        0.5 * (left + right),
        0.5 * (left_slope * inner + right_slope * outer));
}

// Stored energy of one node holding `flux`: `|Q|·r − ∫₀ʳ P`, and the linear
// `Q²/2m` where no law follows the field.
fn temporal_primary_energy(node: u32, flux: f32, local_time: f32) -> f32 {
    if !node_is_nonlinear(node) {
        return 0.5 * flux * flux * temporal_inverse_primary_mass(node, local_time);
    }
    let r = abs(temporal_primary_field(node, flux, local_time));
    return abs(flux) * r - primary_site(node, local_time, r).z;
}

// `r/(j|b|)` at one sample: the scalar that turns the force entry's folded
// `W curlᵀ J b` into `W curlᵀ v(b)`. For a linear record it is `1/factor`,
// exactly the division it replaces.
fn temporal_complementary_secant(sample: u32, flux: vec2<f32>, local_time: f32) -> f32 {
    let word = samples[sample].nodes_b.w;
    let factor = temporal_factor(word, local_time);
    if field_kind(word) == 0u { return 1.0 / factor; }
    let magnitude = length(flux);
    if magnitude == 0.0 { return 1.0 / factor; }
    let reference = samples[sample].constitutive.x;
    let coefficient = factor / reference;
    let r = solve_complementary_radius(word, coefficient, magnitude);
    if r < 0.0 { return 0.0; }
    return r / (reference * magnitude);
}

// The single-record radial solve `c ḡ(r) r = |b|`.
fn solve_complementary_radius(word: u32, coefficient: f32, goal: f32) -> f32 {
    let floor_value = coefficient * table_float(word + 3u, 3u);
    if !(floor_value > 0.0) {
        reject(STATUS_INVERSE_DOMAIN);
        return -1.0;
    }
    var high = goal / floor_value;
    let bound = table_float(word + 3u, 2u);
    if bound > 0.0 && high > bound {
        if coefficient * field_response(word, bound).x * bound
            < goal * (1.0 - INVERSE_TOLERANCE) {
            reject(STATUS_INVERSE_DOMAIN);
            return -1.0;
        }
        high = bound;
    }
    var low = 0.0;
    var r = 0.5 * high;
    for (var iteration = 0u; iteration < INVERSE_ITERATIONS; iteration += 1u) {
        let response = field_response(word, r);
        let residual = coefficient * response.x * r - goal;
        if abs(residual) <= INVERSE_TOLERANCE * goal { return r; }
        if residual < 0.0 { low = r; } else { high = r; }
        if high - low <= 2.0 * 1.1920929e-7 * high { return 0.5 * (low + high); }
        let newton = r - residual / (coefficient * response.y);
        r = select(0.5 * (low + high), newton, newton > low && newton < high);
    }
    reject(STATUS_INVERSE_CONVERGENCE);
    return -1.0;
}

// Stored energy of one sample holding `flux`, its weight included:
// `W(|b| r − c G(r))`, and the linear `½W b·Jb / factor` otherwise.
fn temporal_complementary_energy(sample: u32, flux: vec2<f32>, local_time: f32) -> f32 {
    let word = samples[sample].nodes_b.w;
    if field_kind(word) == 0u {
        return b_energy(sample, flux) / temporal_factor(word, local_time);
    }
    let magnitude = length(flux);
    if magnitude == 0.0 { return 0.0; }
    let reference = samples[sample].constitutive.x;
    let coefficient = temporal_factor(word, local_time) / reference;
    let r = solve_complementary_radius(word, coefficient, magnitude);
    if r < 0.0 { return 0.0; }
    let response = field_response(word, r);
    return samples[sample].constitutive.w * (magnitude * r - coefficient * response.z);
}

fn boundary_float(word: u32, lane: u32) -> f32 {
    return bitcast<f32>(boundary[word].data[lane]);
}

fn uploaded_temporal_factor(coefficient_word: u32, local_time: f32) -> f32 {
    let metadata = boundary[coefficient_word].data;
    let runtime_header = tables[control.runtime_slots.z].data;
    let runtime_word = runtime_header.w + 6u * metadata.x
        + 3u * ((control.runtime_slots.y & 1u) ^ 1u);
    let phase = table_float(runtime_word, metadata.y)
        + boundary_float(coefficient_word + 1u, 3u) * local_time;
    let carrier = reduced_phase(phase);
    let depth = boundary_float(coefficient_word + 1u, 2u);
    let shape = boundary_float(coefficient_word + 2u, 0u);
    let spatial_phase = boundary_float(coefficient_word + 2u, 1u);
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
        switch_factor += blend
            * (boundary_float(coefficient_word + 1u, 1u) - 1.0);
    }
    let factor = drive * switch_factor;
    return select(factor, 1.0 / factor,
        (metadata.w & TEMPORAL_INVERTED) != 0u);
}

fn packed_boundary_scalar(word_offset: u32, scalar_index: u32) -> f32 {
    return boundary_float(word_offset + scalar_index / 4u, scalar_index % 4u);
}

fn source_drive(drive: u32, local_time: f32) -> f32 {
    let base = control.table_offsets.y + 4u * drive + 2u * (control.runtime_slots.x & 1u);
    let signal = vec4<f32>(
        table_float(base, 0u), table_float(base, 1u),
        table_float(base, 2u), table_float(base, 3u));
    let kind = tables[base + 1u].data.x;
    if kind == 0u {
        return harmonic_value(signal, local_time);
    }
    let rate_anchor = table_float(base + 1u, 1u);
    let half_phase = 0.5 * signal.z * local_time;
    return rate_anchor + signal.x * local_time
        + signal.y * local_time * sinc(half_phase) * sin(signal.w + half_phase);
}

fn source_rate(node: u32, local_time: f32) -> f32 {
    let range = nodes[node].ranges.zw;
    var result = 0.0;
    for (var entry = range.x; entry < range.x + range.y; entry += 1u) {
        let drive = tables[entry].data.x;
        result += table_float(entry, 2u) * source_drive(drive, local_time);
    }
    return result;
}

fn gathered_force(node: u32, second: bool) -> f32 {
    return gathered_forces(node, second, false).x;
}

// The node's force and, with `short_wave`, the gather of the short-wave
// stresses the drift left, through the same entries and the same map.
fn gathered_forces(node: u32, second: bool, short_wave: bool) -> vec2<f32> {
    let range = nodes[node].ranges.xy;
    let driven = temporal_enabled();
    let force_time = control.clock_f32.y
        + select(0.0, control.clock_f32.x, second);
    var result = 0.0;
    var stress = 0.0;
    for (var entry = range.x; entry < range.x + range.y; entry += 1u) {
        let index = tables[entry].data.x;
        let kind = tables[entry].data.y;
        let coefficient_x = table_float(entry, 2u);
        if kind == FORCE_KIND_GAP {
            let auxiliary = index - auxiliary_offset();
            result += coefficient_x * select(
                accepted_auxiliary(auxiliary), candidate_auxiliary(auxiliary),
                second || has_loss_stages());
        } else {
            let flux = select(
                accepted_b(index), candidate_b(index), second || has_loss_stages());
            var inverse_factor = 1.0;
            if field_laws() {
                inverse_factor = scratch[sample_secant_offset() + index].values.x;
            } else if driven {
                inverse_factor = 1.0 / temporal_complementary_factor(index, force_time);
            }
            let coefficient = vec2<f32>(coefficient_x, table_float(entry, 3u));
            result += inverse_factor * dot(coefficient, flux);
            if short_wave {
                stress += inverse_factor
                    * dot(coefficient, scratch[complementary_offset() + index].values.zw);
            }
        }
    }
    return vec2<f32>(result, stress);
}

fn gap_force(node: u32, second: bool) -> f32 {
    let range = nodes[node].ranges.xy;
    var result = 0.0;
    for (var entry = range.x; entry < range.x + range.y; entry += 1u) {
        if tables[entry].data.y == FORCE_KIND_GAP {
            let auxiliary = tables[entry].data.x - auxiliary_offset();
            result += table_float(entry, 2u) * select(
                accepted_auxiliary(auxiliary), candidate_auxiliary(auxiliary),
                second || has_loss_stages());
        }
    }
    return result;
}

// The node's restoring record for its coefficient record at `word`: each sits
// a quarter as far into its block as the coefficient sits into its own.
fn restoring_record(word: u32) -> vec4<u32> {
    let header = tables[control.runtime_slots.z].data;
    let block = tables[control.runtime_slots.z + 2u].data.z;
    return tables[block + (word - header.x) / TEMPORAL_COEFFICIENT_WORDS].data;
}

// `Σ m₀ V′(r)` over the node's materials, at their authored lumped masses.
// A φ⁴ node past its declared bound fails the step; nothing is clipped.
fn restoring_force_at(node: u32, r: f32) -> f32 {
    let range = nodes[node].stiffness.zw;
    var result = 0.0;
    for (var record = 0u; record < range.y; record += 1u) {
        let entry = restoring_record(range.x + record * TEMPORAL_COEFFICIENT_WORDS);
        let mass = bitcast<f32>(entry.y);
        let coefficient = bitcast<f32>(entry.z);
        if entry.x == RESTORING_KLEIN_GORDON {
            result += mass * coefficient * r;
        } else if entry.x == RESTORING_SINE_GORDON {
            result += mass * coefficient * sin(r);
        } else if entry.x == RESTORING_PHI4 {
            if abs(r) > bitcast<f32>(entry.w) { reject(STATUS_RESTORING_DOMAIN); }
            result += mass * coefficient * (r * r - 1.0) * r;
        }
    }
    return result;
}

// `Σ m₀ V(r)`. Sine-Gordon's `1 − cos r` is formed as `2 sin²(r/2)`, which f32
// would otherwise cancel away near the vacuum.
fn restoring_potential_at(node: u32, r: f32) -> f32 {
    let range = nodes[node].stiffness.zw;
    var result = 0.0;
    for (var record = 0u; record < range.y; record += 1u) {
        let entry = restoring_record(range.x + record * TEMPORAL_COEFFICIENT_WORDS);
        let mass = bitcast<f32>(entry.y);
        let coefficient = bitcast<f32>(entry.z);
        if entry.x == RESTORING_KLEIN_GORDON {
            result += 0.5 * mass * coefficient * r * r;
        } else if entry.x == RESTORING_SINE_GORDON {
            let half = sin(0.5 * r);
            result += 2.0 * mass * coefficient * half * half;
        } else if entry.x == RESTORING_PHI4 {
            let well = r * r - 1.0;
            result += 0.25 * mass * coefficient * well * well;
        }
    }
    return result;
}

// The restoring force a kick holds: at the accepted `r` in the first, at the
// drifted candidate in the second. A loss stage has copied the accepted lane
// into the candidate before the first kick, as it does for the gaps.
fn restoring_force(node: u32, second: bool) -> f32 {
    if !restoring() { return 0.0; }
    let index = integrated_index(node);
    return restoring_force_at(node, select(
        accepted_auxiliary(index), candidate_auxiliary(index),
        second || has_loss_stages()));
}

fn force(node: u32, second: bool) -> f32 {
    if use_force_cache() {
        let cached = select(accepted_force(node), candidate_force(node), second);
        return cached + gap_force(node, second) + restoring_force(node, second);
    }
    return gathered_force(node, second) + restoring_force(node, second);
}

// `time` is the instant whose nodal mass converts `Q` into a field. A driven
// medium's mass moves, so a caller inside the drift passes that stage's own
// midpoint; on a fixed generation the authored inverse mass is the same thing
// at every instant and the branch costs nothing.
fn stiffness_force(node: u32, time: f32) -> f32 {
    let range = nodes[node].stiffness.xy;
    var inverse_mass = nodes[node].mass_loss.y;
    if temporal_enabled() {
        inverse_mass = temporal_inverse_primary_mass(node, time);
    }
    let row_field = candidate_q(node) * inverse_mass;
    var result = 0.0;
    for (var entry = range.x; entry < range.x + range.y; entry += 1u) {
        let column = tables[entry].data.x;
        var column_inverse_mass = nodes[column].mass_loss.y;
        if temporal_enabled() {
            column_inverse_mass = temporal_inverse_primary_mass(column, time);
        }
        let column_field = candidate_q(column) * column_inverse_mass;
        result += table_float(entry, 2u) * (column_field - row_field);
    }
    return result;
}

fn constitutive_force(node: u32) -> f32 {
    let range = nodes[node].ranges.xy;
    let driven = temporal_enabled();
    var result = 0.0;
    for (var entry = range.x; entry < range.x + range.y; entry += 1u) {
        if tables[entry].data.y != FORCE_KIND_GAP {
            let sample = tables[entry].data.x;
            let coefficient = vec2<f32>(
                table_float(entry, 2u), table_float(entry, 3u));
            var inverse_factor = 1.0;
            if driven {
                inverse_factor = 1.0 / temporal_complementary_factor(
                    sample, control.clock_f32.y);
            }
            result += inverse_factor * dot(coefficient, accepted_b(sample));
        }
    }
    return result;
}

fn candidate_constitutive_force(node: u32) -> f32 {
    let range = nodes[node].ranges.xy;
    let driven = temporal_enabled();
    var result = 0.0;
    for (var entry = range.x; entry < range.x + range.y; entry += 1u) {
        if tables[entry].data.y != FORCE_KIND_GAP {
            let sample = tables[entry].data.x;
            let coefficient = vec2<f32>(
                table_float(entry, 2u), table_float(entry, 3u));
            var inverse_factor = 1.0;
            if driven {
                inverse_factor = 1.0 / temporal_complementary_factor(
                    sample, control.clock_f32.y);
            }
            result += inverse_factor * dot(coefficient, candidate_b(sample));
        }
    }
    return result;
}

fn stiffness_of_scratch(node: u32, lane: u32) -> f32 {
    let range = nodes[node].stiffness.xy;
    let row_field = scratch[node].values[lane] * nodes[node].mass_loss.y;
    var result = 0.0;
    for (var entry = range.x; entry < range.x + range.y; entry += 1u) {
        let column = tables[entry].data.x;
        let column_field = scratch[column].values[lane] * nodes[column].mass_loss.y;
        result += table_float(entry, 2u) * (column_field - row_field);
    }
    return result;
}

fn b_energy(sample_index: u32, value: vec2<f32>) -> f32 {
    let constitutive = samples[sample_index].constitutive;
    let field = vec2<f32>(
        constitutive.x * value.x + constitutive.y * value.y,
        constitutive.y * value.x + constitutive.z * value.y);
    return 0.5 * constitutive.w * dot(value, field);
}

fn instantaneous_b_energy(sample_index: u32, value: vec2<f32>) -> f32 {
    let reference = b_energy(sample_index, value);
    if temporal_enabled() {
        return reference / temporal_complementary_factor(
            sample_index, control.clock_f32.y);
    }
    return reference;
}

// The filter's inner map at a node, cached by its site pass: `m⁻¹(t)`, or
// on a field-dependent node the tangent inverse `A = 1/P′(U)`.
fn filter_node_weight(node: u32) -> f32 {
    return scratch[node_field_offset() + node].values.z;
}

fn filter_compatible_flux(sample_index: u32, lane: u32, divide_mass: bool) -> vec2<f32> {
    let sample = samples[sample_index];
    let reference_node = sample_node(sample, 0u);
    var reference = scratch[reference_node].values[lane];
    if divide_mass {
        reference *= filter_node_weight(reference_node);
    }
    var result = vec2<f32>(0.0);
    for (var local = 1u; local < 7u; local += 1u) {
        let node = sample_node(sample, local);
        var field = scratch[node].values[lane];
        if divide_mass {
            field *= filter_node_weight(node);
        }
        result += sample_curl(sample, local) * (field - reference);
    }
    return control.evolution.y * result;
}

// `J_b x` in the folded units the force entries carry: `σx + (τ−σ)(n·x)n`,
// with the secant `σ = r/(j|b|)` and the radial tangent `τ` cached by the
// site pass at the accepted flux. A linear record has `σ = τ = 1/factor`,
// which is the frozen-time filter's division.
fn filter_tangent(sample: u32, flux: vec2<f32>) -> vec2<f32> {
    let cached = scratch[sample_secant_offset() + sample].values;
    let b = accepted_b(sample);
    let magnitude = length(b);
    if magnitude == 0.0 { return cached.y * flux; }
    let n = b / magnitude;
    return cached.y * flux + (cached.z - cached.y) * dot(n, flux) * n;
}

fn filter_gather(node: u32, second_pair: bool) -> f32 {
    let range = nodes[node].ranges.xy;
    var result = 0.0;
    for (var entry = range.x; entry < range.x + range.y; entry += 1u) {
        if tables[entry].data.y != FORCE_KIND_GAP {
            let sample = tables[entry].data.x;
            let value = scratch[control.counts_a.x + sample].values;
            let flux = select(value.xy, value.zw, second_pair);
            let coefficient = vec2<f32>(
                table_float(entry, 2u), table_float(entry, 3u));
            result += dot(coefficient, flux);
        }
    }
    return result;
}

// `Cᵀ W v(b)` at the accepted flux, through the secants the site pass cached.
fn filter_temporal_force(node: u32) -> f32 {
    let range = nodes[node].ranges.xy;
    var result = 0.0;
    for (var entry = range.x; entry < range.x + range.y; entry += 1u) {
        if tables[entry].data.y != FORCE_KIND_GAP {
            let sample = tables[entry].data.x;
            let coefficient = vec2<f32>(
                table_float(entry, 2u), table_float(entry, 3u));
            result += scratch[sample_secant_offset() + sample].values.y
                * dot(coefficient, accepted_b(sample));
        }
    }
    return result;
}

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

// In this entry point binding 7 is the transient upload buffer rather than
// the immutable outgoing-boundary table. Subsequent event stages do not read
// boundary data, so the same eight-binding layout remains portable.
@compute @workgroup_size(128)
fn live_event_stage(@builtin(global_invocation_id) id: vec3<u32>) {
    let i = id.x;
    if stopped() { return; }
    let upload = boundary[0u].data;
    let operation = upload.x;
    if i == 0u {
        control.event.y = upload.y;
        control.event.z = accepted_slot() | 2u | (operation << 8u);
        control.event.w = upload.w;
    }
    if operation == 1u || operation == 4u {
        if i < control.counts_a.x && i < upload.z {
            scratch[i].values.x = bitcast<f32>(boundary[1u + i].data.x);
        }
        return;
    }
    if operation == 3u {
        if i < control.counts_a.x {
            nodes[i].mass_loss.w = bitcast<f32>(boundary[1u + i].data.x);
        } else if i < control.counts_a.x + control.counts_a.y {
            let sample = i - control.counts_a.x;
            samples[sample].curl_6_loss.w = bitcast<f32>(
                boundary[1u + control.counts_a.x + sample].data.x);
        }
        return;
    }
    if (operation == 5u || operation == 6u) && i < control.counts_c.z {
        let source_slot = control.runtime_slots.x & 1u;
        let accepted_base = control.table_offsets.y + 4u * i + 2u * source_slot;
        let candidate_base = control.table_offsets.y + 4u * i
            + 2u * (source_slot ^ 1u);
        let upload_base = 1u + 4u * i;
        var parameters = bitcast<vec4<f32>>(boundary[upload_base].data);
        var runtime = boundary[upload_base + 1u].data;
        let old_parameters = vec4<f32>(
            table_float(accepted_base, 0u), table_float(accepted_base, 1u),
            table_float(accepted_base, 2u), table_float(accepted_base, 3u));
        let current_phase = reduced_phase(
            old_parameters.w + old_parameters.z * control.clock_f32.y);
        parameters.w = reduced_phase(
            current_phase - parameters.z * control.clock_f32.y);
        if runtime.x != 0u {
            let elapsed = control.clock_f32.y;
            let half_phase = 0.5 * parameters.z * elapsed;
            let new_integral = parameters.x * elapsed
                + parameters.y * elapsed * sinc(half_phase)
                    * sin(parameters.w + half_phase);
            runtime.y = bitcast<u32>(
                source_drive(i, control.clock_f32.y) - new_integral);
        }
        tables[candidate_base].data = bitcast<vec4<u32>>(parameters);
        tables[candidate_base + 1u].data = runtime;
        if !finite_vector(parameters) { reject(STATUS_NON_FINITE); }
    }
    if operation == 6u && i < control.counts_a.x {
        let drive_words = 4u * control.counts_c.z;
        let start = nodes[i].ranges.z;
        let count = nodes[i].ranges.w;
        for (var slot = 0u; slot < count; slot += 1u) {
            let entry = start + slot;
            let source_index = entry - control.counts_c.x;
            let weight = bitcast<f32>(boundary[1u + drive_words + source_index].data.x);
            tables[entry].data.w = bitcast<u32>(weight);
            if !finite_scalar(weight) { reject(STATUS_NON_FINITE); }
        }
    }
    if operation == 7u {
        if !temporal_enabled() { reject(STATUS_LAYOUT); return; }
        let header = tables[control.runtime_slots.z].data;
        let runtime_count = tables[control.runtime_slots.z + 1u].data.x;
        if upload.z >= runtime_count || upload.w > 1u {
            reject(STATUS_LAYOUT);
            return;
        }
        if i < runtime_count {
            let root = header.w + 6u * i;
            let accepted = root + 3u * (control.runtime_slots.y & 1u);
            let candidate = root + 3u * ((control.runtime_slots.y & 1u) ^ 1u);
            tables[candidate].data = tables[accepted].data;
            tables[candidate + 1u].data = tables[accepted + 1u].data;
            tables[candidate + 2u].data = tables[accepted + 2u].data;
            if i == upload.z {
                let target_blend = f32(upload.w);
                let duration = boundary_float(1u, 0u);
                var start = temporal_switch_blend(accepted, control.clock_f32.y);
                if duration == 0.0 { start = target_blend; }
                tables[candidate + 1u].data = bitcast<vec4<u32>>(vec4<f32>(
                    start, target_blend, control.clock_f32.y, duration));
                if !finite_scalar(start) || !finite_scalar(duration)
                    || duration < 0.0 {
                    reject(STATUS_LAYOUT);
                }
            }
        }
    }
    if operation == 8u {
        if !temporal_enabled() { reject(STATUS_LAYOUT); return; }
        let header = tables[control.runtime_slots.z].data;
        let table_metadata = tables[control.runtime_slots.z + 1u].data;
        let event_metadata = boundary[1u].data;
        if upload.z != header.y || upload.w != control.counts_a.y
            || event_metadata.x != table_metadata.x
            || event_metadata.y != TEMPORAL_COEFFICIENT_WORDS {
            reject(STATUS_LAYOUT);
            return;
        }
        if i < table_metadata.x {
            let frequency_offset = 2u
                + TEMPORAL_COEFFICIENT_WORDS * (upload.z + upload.w);
            let target_frequency = bitcast<vec4<f32>>(
                boundary[frequency_offset + i].data);
            let root = header.w + 6u * i;
            let accepted = root + 3u * (control.runtime_slots.y & 1u);
            let candidate = root + 3u * ((control.runtime_slots.y & 1u) ^ 1u);
            let old_phase = bitcast<vec4<f32>>(tables[accepted].data);
            let old_frequency = bitcast<vec4<f32>>(tables[accepted + 2u].data);
            var next_phase: vec4<f32>;
            for (var lane = 0u; lane < 4u; lane += 1u) {
                let current = reduced_phase(
                    old_phase[lane] + old_frequency[lane] * control.clock_f32.y);
                next_phase[lane] = reduced_phase(
                    current - target_frequency[lane] * control.clock_f32.y);
            }
            tables[candidate].data = bitcast<vec4<u32>>(next_phase);
            tables[candidate + 1u].data = tables[accepted + 1u].data;
            tables[candidate + 2u].data = bitcast<vec4<u32>>(target_frequency);
            if !finite_vector(next_phase) || !finite_vector(target_frequency)
                || any(target_frequency < vec4<f32>(0.0)) {
                reject(STATUS_LAYOUT);
            }
        }
    }
}

// Periodic grid damping is resident solver maintenance, not a host-authored
// event. Start the same staged paired filter without touching event serials or
// runtime ownership; the matching commit below accepts it atomically on the
// GPU before the next ordinary step in this command buffer.
@compute @workgroup_size(1)
fn resident_filter_begin() {
    if stopped() { return; }
    control.event.z = accepted_slot() | (2u << 8u);
    control.event.w = bitcast<u32>(1.0);
}

@compute @workgroup_size(128)
fn event_begin(@builtin(global_invocation_id) id: vec3<u32>) {
    let i = id.x;
    if stopped() || event_operation() == 0u { return; }
    if i == 0u {
        control.candidate_accounting_a = control.accepted_accounting_a;
        control.candidate_accounting_b = control.accepted_accounting_b;
        control.candidate_accounting_c = control.accepted_accounting_c;
    }
    // State and the cumulative step-accounting bank share the accepted lane.
    // A zero-duration event flips that lane, so begin a fresh contribution
    // bank from the consolidated control totals rather than exposing the
    // previous candidate bank as accepted.
    if i < accounting_item_count() {
        let candidate = accounting_bank_offset(accepted_slot() ^ 1u) + i;
        scratch[candidate].values = vec4<f32>(0.0);
    }
    if i < control.counts_a.x {
        set_candidate_q(i, accepted_q(i));
        set_candidate_force(i, accepted_force(i));
    } else if i < control.counts_a.x + control.counts_a.y {
        let sample = i - complementary_offset();
        set_candidate_b(sample, accepted_b(sample));
    } else if i < control.counts_a.w {
        let auxiliary = i - auxiliary_offset();
        set_candidate_auxiliary(auxiliary, accepted_auxiliary(auxiliary));
    }
    if i < control.counts_c.z
        && event_operation() != 5u
        && event_operation() != 6u {
        let source_slot = control.runtime_slots.x & 1u;
        let accepted_base = control.table_offsets.y + 4u * i + 2u * source_slot;
        let candidate_base = control.table_offsets.y + 4u * i
            + 2u * (source_slot ^ 1u);
        tables[candidate_base].data = tables[accepted_base].data;
        tables[candidate_base + 1u].data = tables[accepted_base + 1u].data;
    }
    inject_at(i);
}

@compute @workgroup_size(128)
fn event_simple_stage(@builtin(global_invocation_id) id: vec3<u32>) {
    let i = id.x;
    if stopped() { return; }
    let operation = event_operation();
    if operation == 8u {
        let upload = boundary[0u].data;
        let event_metadata = boundary[1u].data;
        if i == 0u {
            let maximum_dt = bitcast<f32>(event_metadata.z);
            if !finite_scalar(maximum_dt) || maximum_dt <= 0.0
                || control.clock_f32.x > maximum_dt {
                reject(STATUS_TIMESTEP);
            }
        }
        if i < control.counts_a.x {
            let header = tables[control.runtime_slots.z].data;
            let range = nodes[i].stiffness.zw;
            var mass = 0.0;
            for (var record = 0u; record < range.y; record += 1u) {
                let source = 2u + (range.x - header.x)
                    + record * TEMPORAL_COEFFICIENT_WORDS;
                mass += boundary_float(source + 1u, 0u)
                    * uploaded_temporal_factor(source, control.clock_f32.y);
            }
            if !finite_scalar(mass) || mass <= 0.0 { reject(STATUS_INVERSE_DOMAIN); }
        } else if i < control.counts_a.x + control.counts_a.y {
            let sample = i - control.counts_a.x;
            let source = 2u + TEMPORAL_COEFFICIENT_WORDS
                * (upload.z + sample);
            let factor = uploaded_temporal_factor(source, control.clock_f32.y);
            if !finite_scalar(factor) || factor <= 0.0 {
                reject(STATUS_INVERSE_DOMAIN);
            }
        }
        return;
    }
    if i < control.counts_a.x {
        if operation == 1u || operation == 4u {
            // A pulse is authored as a field increment, and the canonical state
            // is the integrated nodal flux, so it is scaled by the nodal mass
            // here rather than by the host. The event lands at a complete-step
            // boundary that the host cannot see ahead to, and a driven medium's
            // mass has moved by then: on a pump whose factor bottoms near a
            // fifth, an amplitude scaled by the authored mass arrives several
            // times too large or too small depending on the phase it meets.
            // A maintenance correction is already integrated and is added as is.
            var increment = scratch[i].values.x;
            if operation == 1u {
                var mass = nodes[i].mass_loss.x;
                if temporal_enabled() {
                    mass = temporal_primary_mass(i, control.clock_f32.y);
                }
                if !finite_scalar(mass) || mass <= 0.0 {
                    reject(STATUS_INVERSE_DOMAIN);
                }
                increment = mass * increment;
            }
            var next = accepted_q(i) + increment;
            if nodes[i].boundary.z != 0u {
                next = accepted_q(i);
            }
            set_candidate_q(i, next);
            if !finite_scalar(next) { reject(STATUS_NON_FINITE); }
        } else if operation == 3u {
            let fraction = nodes[i].mass_loss.w;
            if !finite_scalar(fraction) || fraction < 0.0 || fraction > 1.0 {
                reject(STATUS_LAYOUT);
            }
        }
    } else if operation == 3u && i < control.counts_a.x + control.counts_a.y {
        let sample = i - complementary_offset();
        let fraction = samples[sample].curl_6_loss.w;
        if !finite_scalar(fraction) || fraction < 0.0 || fraction > 1.0 {
            reject(STATUS_LAYOUT);
        }
    }
}

// `(accepted, candidate)` stored energy of one node and one sample at the
// event instant, through the nonlinear maps on a field-dependent generation.
fn filter_node_energies(node: u32, candidate: f32) -> vec2<f32> {
    let time = control.clock_f32.y;
    let accepted = accepted_q(node);
    if field_laws() {
        return vec2<f32>(
            temporal_primary_energy(node, accepted, time),
            temporal_primary_energy(node, candidate, time));
    }
    let inverse_mass = temporal_inverse_primary_mass(node, time);
    return 0.5 * inverse_mass * vec2<f32>(accepted * accepted, candidate * candidate);
}

fn filter_sample_energies(sample: u32, candidate: vec2<f32>) -> vec2<f32> {
    if field_laws() {
        let time = control.clock_f32.y;
        return vec2<f32>(
            temporal_complementary_energy(sample, accepted_b(sample), time),
            temporal_complementary_energy(sample, candidate, time));
    }
    return vec2<f32>(
        instantaneous_b_energy(sample, accepted_b(sample)),
        instantaneous_b_energy(sample, candidate));
}

// The filter's maps frozen at the event instant, evaluated once per site:
// each node's field and inverse mass or tangent inverse, and each sample's
// secant and radial tangent (gate F on a field-dependent medium, the plain
// time-driven factors otherwise). The step caches share these words but
// refill them before they are read, so borrowing them between steps costs
// nothing.
@compute @workgroup_size(128)
fn filter_temporal_sites(@builtin(global_invocation_id) id: vec3<u32>) {
    let i = id.x;
    if stopped() || !temporal_enabled() { return; }
    let time = control.clock_f32.y;
    if i < control.counts_a.x {
        let field = temporal_primary_field(i, accepted_q(i), time);
        var weight = temporal_inverse_primary_mass(i, time);
        if node_is_nonlinear(i) {
            weight = 1.0 / primary_site(i, time, abs(field)).y;
        }
        scratch[node_field_offset() + i].values.y = field;
        scratch[node_field_offset() + i].values.z = weight;
        return;
    }
    let sample = i - control.counts_a.x;
    if sample >= control.counts_a.y { return; }
    let word = samples[sample].nodes_b.w;
    let factor = temporal_factor(word, time);
    var secant = 1.0 / factor;
    var tangent = secant;
    let magnitude = length(accepted_b(sample));
    if field_kind(word) != 0u && magnitude > 0.0 {
        let reference = samples[sample].constitutive.x;
        let r = solve_complementary_radius(word, factor / reference, magnitude);
        if r >= 0.0 {
            secant = r / (reference * magnitude);
            tangent = 1.0 / (factor * field_response(word, r).y);
        }
    }
    scratch[sample_secant_offset() + sample].values.y = secant;
    scratch[sample_secant_offset() + sample].values.z = tangent;
}

@compute @workgroup_size(128)
fn filter_first(@builtin(global_invocation_id) id: vec3<u32>) {
    let node = id.x;
    if stopped() || node >= control.counts_a.x { return; }
    if temporal_enabled() {
        scratch[node].values.x = scratch[node_field_offset() + node].values.y;
        // Gate O: the total force on the integrated field, `F + R`.
        var total = filter_temporal_force(node);
        if restoring() {
            total += restoring_force_at(node, accepted_auxiliary(integrated_index(node)));
        }
        scratch[node].values.y = total;
        return;
    }
    scratch[node].values.x = stiffness_force(node, control.clock_f32.y);
    scratch[node].values.y = constitutive_force(node);
}

@compute @workgroup_size(128)
fn filter_second(@builtin(global_invocation_id) id: vec3<u32>) {
    let i = id.x;
    if stopped() { return; }
    if temporal_enabled() {
        if i >= control.counts_a.y { return; }
        let first = filter_tangent(i, filter_compatible_flux(i, 0u, false));
        let second = filter_tangent(i, filter_compatible_flux(i, 1u, true));
        scratch[control.counts_a.x + i].values = vec4<f32>(first, second);
        return;
    }
    if i >= control.counts_a.x { return; }
    let node = i;
    scratch[node].values.z = stiffness_of_scratch(node, 0u);
    scratch[node].values.w = stiffness_of_scratch(node, 1u);
}

@compute @workgroup_size(128)
fn filter_temporal_gather(@builtin(global_invocation_id) id: vec3<u32>) {
    let node = id.x;
    if stopped() || !temporal_enabled() || node >= control.counts_a.x { return; }
    scratch[node].values.z = filter_gather(node, false);
    scratch[node].values.w = filter_gather(node, true);
}

@compute @workgroup_size(128)
fn filter_temporal_samples(@builtin(global_invocation_id) id: vec3<u32>) {
    let sample = id.x;
    if stopped() || !temporal_enabled() || sample >= control.counts_a.y { return; }
    let primary_flux = filter_tangent(sample, filter_compatible_flux(sample, 2u, true));
    let correction = filter_compatible_flux(sample, 3u, true);
    let next = accepted_b(sample) - bitcast<f32>(control.event.w)
        * control.evolution.x * correction;
    // The commit test's two energies of this sample, formed here in parallel
    // so the single-workgroup validation only sums them. The pair lanes are
    // free again: the finalize reads only the primary flux beside them.
    scratch[control.counts_a.x + sample].values =
        vec4<f32>(primary_flux, filter_sample_energies(sample, next));
    set_candidate_b(sample, next);
    if !all(next >= vec2<f32>(-MAX_FINITE))
        || !all(next <= vec2<f32>(MAX_FINITE)) {
        reject(STATUS_NON_FINITE);
    }
}

@compute @workgroup_size(128)
fn filter_finalize(@builtin(global_invocation_id) id: vec3<u32>) {
    let i = id.x;
    if stopped() { return; }
    let scale = bitcast<f32>(control.event.w) * control.evolution.x;
    if temporal_enabled() {
        if i >= control.counts_a.x { return; }
        // A pin holds its data at this endpoint; there is nothing to filter
        // and no exchange to account.
        var next = accepted_q(i) - scale * filter_gather(i, false);
        if nodes[i].boundary.z != 0u { next = accepted_q(i); }
        set_candidate_q(i, next);
        var energies = filter_node_energies(i, next);
        // Gate O: `r` takes the correction `b` takes through `ηC`,
        // `δψ = −s A K A (F + R)`, read from the gathered lane before the
        // energies overwrite it; its store joins the commit test.
        if restoring() {
            let index = integrated_index(i);
            let old_r = accepted_auxiliary(index);
            let next_r = old_r - scale * filter_node_weight(i) * scratch[i].values.w;
            set_candidate_auxiliary(index, next_r);
            energies += vec2<f32>(
                restoring_potential_at(i, old_r), restoring_potential_at(i, next_r));
            if !finite_scalar(next_r) { reject(STATUS_NON_FINITE); }
        }
        scratch[i].values.z = energies.x;
        scratch[i].values.w = energies.y;
        let next_force = candidate_constitutive_force(i);
        set_candidate_force(i, next_force);
        if !finite_scalar(next) || !finite_scalar(next_force) {
            reject(STATUS_NON_FINITE);
        }
        return;
    }
    if i < control.counts_a.x {
        var next = accepted_q(i) - scale * scratch[i].values.z;
        if nodes[i].boundary.z != 0u { next = accepted_q(i); }
        set_candidate_q(i, next);
        let next_force = accepted_force(i) - scale * stiffness_of_scratch(i, 3u);
        set_candidate_force(i, next_force);
        if !finite_scalar(next) || !finite_scalar(next_force) {
            reject(STATUS_NON_FINITE);
        }
        return;
    }
    if i < control.counts_a.x + control.counts_a.y {
        let sample_index = i - complementary_offset();
        let sample = samples[sample_index];
        let reference = scratch[sample.nodes_a.x].values.w
            * nodes[sample.nodes_a.x].mass_loss.y;
        var correction = vec2<f32>(0.0);
        for (var local = 1u; local < 7u; local += 1u) {
            let node = sample_node(sample, local);
            let field = scratch[node].values.w * nodes[node].mass_loss.y;
            correction += sample_curl(sample, local) * (field - reference);
        }
        let next = accepted_b(sample_index) - scale * control.evolution.y * correction;
        set_candidate_b(sample_index, next);
        if !all(next >= vec2<f32>(-MAX_FINITE))
            || !all(next <= vec2<f32>(MAX_FINITE)) {
            reject(STATUS_NON_FINITE);
        }
    }
}

@compute @workgroup_size(128)
fn event_validate(@builtin(local_invocation_id) id: vec3<u32>) {
    let local = id.x;
    let participating = !stopped();
    // A time-driven filter formed every site's energies in its own parallel
    // passes; the rest are cheap enough to form here.
    let precomputed = temporal_enabled() && event_operation() == 2u;
    var energies = vec2<f32>(0.0);
    if participating {
        for (var node = local; node < control.counts_a.x; node += WORKGROUP_SIZE) {
            if precomputed {
                energies += scratch[node].values.zw;
                continue;
            }
            var inverse_mass = nodes[node].mass_loss.y;
            if temporal_enabled() {
                inverse_mass = temporal_inverse_primary_mass(node, control.clock_f32.y);
            }
            let accepted = accepted_q(node);
            let candidate = candidate_q(node);
            energies += 0.5 * inverse_mass * vec2<f32>(accepted * accepted, candidate * candidate);
        }
        for (var sample = local; sample < control.counts_a.y; sample += WORKGROUP_SIZE) {
            if precomputed {
                energies += scratch[control.counts_a.x + sample].values.zw;
                continue;
            }
            energies.x += instantaneous_b_energy(sample, accepted_b(sample));
            energies.y += instantaneous_b_energy(sample, candidate_b(sample));
        }
    }
    let total = reduce_boundary_vector(local, energies);
    if !participating || local != 0u { return; }
    if !all(total >= vec2<f32>(-MAX_FINITE))
        || !all(total <= vec2<f32>(MAX_FINITE)) {
        reject(STATUS_NON_FINITE);
        return;
    }
    let change = total.y - total.x;
    switch event_operation() {
        case 1u: { control.candidate_accounting_a.x += change; }
        case 2u: {
            let tolerance = 5.0e-6 * max(total.x, 1.0);
            if change > tolerance { reject(STATUS_TIMESTEP); }
            control.candidate_accounting_b.y += max(-change, 0.0);
        }
        case 4u: { control.candidate_accounting_b.z += change; }
        default: {}
    }
}

@compute @workgroup_size(128)
fn event_accept_tables(@builtin(global_invocation_id) id: vec3<u32>) {
    let i = id.x;
    if atomicLoad(&status.candidate) != 0u || atomicLoad(&status.latch) != 0u { return; }
    if event_operation() == 8u {
        let upload = boundary[0u].data;
        let header = tables[control.runtime_slots.z].data;
        let primary_words = TEMPORAL_COEFFICIENT_WORDS * upload.z;
        let coefficient_words = primary_words
            + TEMPORAL_COEFFICIENT_WORDS * upload.w;
        if i < coefficient_words {
            var target_word = header.x + i;
            if i >= primary_words {
                target_word = header.z + i - primary_words;
            }
            tables[target_word].data = boundary[2u + i].data;
        }
        return;
    }
    if event_operation() == 6u {
        if i < control.counts_a.x {
            let start = nodes[i].ranges.z;
            let count = nodes[i].ranges.w;
            for (var slot = 0u; slot < count; slot += 1u) {
                let entry = start + slot;
                tables[entry].data.z = tables[entry].data.w;
            }
        }
        return;
    }
    if event_operation() != 3u { return; }
    if i < control.counts_a.x {
        nodes[i].mass_loss.z = nodes[i].mass_loss.w;
    } else if i < control.counts_a.x + control.counts_a.y {
        let sample = i - complementary_offset();
        samples[sample].curl_6_loss.z = samples[sample].curl_6_loss.w;
    }
}

@compute @workgroup_size(1)
fn commit_event() {
    let failure = atomicLoad(&status.candidate);
    if failure != 0u {
        if live_event() {
            control.event_result = vec4<u32>(
                control.event.y, event_operation(), control.event.y, failure);
            control.event.z = accepted_slot();
            atomicStore(&status.candidate, 0u);
        } else {
            atomicMax(&status.latch, failure);
        }
        return;
    }
    let operation = event_operation();
    control.event.z = accepted_slot() ^ 1u;
    control.event.x = control.event.y;
    control.accepted_accounting_a = control.candidate_accounting_a;
    control.accepted_accounting_b = control.candidate_accounting_b;
    control.accepted_accounting_c = control.candidate_accounting_c;
    if operation == 3u {
        var flags = control.boundary_offsets.w & 8u;
        flags |= control.event.w & 1u;
        flags |= 2u;
        flags |= control.event.w & 4u;
        control.boundary_offsets.w = flags;
        control.runtime_serials.y = control.event.y;
    } else if operation == 1u {
        control.runtime_serials.z = control.event.y;
    } else if operation == 4u {
        control.runtime_serials.w = control.event.y;
    } else if operation == 5u || operation == 6u {
        control.runtime_serials.x = control.event.y;
        control.runtime_slots.x ^= 1u;
    } else if operation == 7u {
        control.runtime_serials.y = control.event.y;
        control.runtime_slots.y ^= 1u;
    } else if operation == 8u {
        control.runtime_serials.y = control.event.y;
        control.runtime_slots.y ^= 1u;
        control.clock_f32.w = bitcast<f32>(boundary[1u].data.z);
    }
    control.event_result = vec4<u32>(
        control.event.y, operation, control.event.y, 0u);
    publish_snapshot_metadata();
}

@compute @workgroup_size(1)
fn resident_filter_commit() {
    let failure = atomicLoad(&status.candidate);
    // On a field-dependent medium the tangent polynomial is dissipative only
    // to first order, and so is the total-force correction on a nonlinear
    // restoring law, so a candidate that gains energy, or leaves the map's
    // domain, is a filter not taken rather than a fault: the accepted lane
    // stays and stepping continues. A linear filter cannot gain energy, and
    // anything non-finite is a fault on either.
    if failure != 0u && (field_laws() || restoring()) && (failure == STATUS_TIMESTEP
        || failure == STATUS_INVERSE_DOMAIN || failure == STATUS_INVERSE_CONVERGENCE) {
        control.event.z = accepted_slot();
        atomicStore(&status.candidate, 0u);
        return;
    }
    if failure != 0u {
        atomicMax(&status.latch, failure);
        return;
    }
    control.event.z = accepted_slot() ^ 1u;
    control.accepted_accounting_a = control.candidate_accounting_a;
    control.accepted_accounting_b = control.candidate_accounting_b;
    control.accepted_accounting_c = control.candidate_accounting_c;
    publish_snapshot_metadata();
}

@compute @workgroup_size(128)
fn handoff_finalize(@builtin(global_invocation_id) id: vec3<u32>) {
    let i = id.x;
    if stopped() { return; }
    if i < control.counts_a.x {
        let before = candidate_q(i);
        var next = before;
        var exchange = 0.0;
        let nonlinear = temporal_enabled() && node_is_nonlinear(i);
        if nodes[i].boundary.z != 0u {
            if nonlinear {
                next = temporal_primary_flux_of_field(
                    i, harmonic_value(nodes[i].prescribed, 0.0), 0.0);
                exchange = temporal_primary_energy(i, next, 0.0)
                    - temporal_primary_energy(i, before, 0.0);
            } else {
                next = nodes[i].mass_loss.x * harmonic_value(nodes[i].prescribed, 0.0);
                exchange = 0.5 * (next * next - before * before) * nodes[i].mass_loss.y;
            }
            set_candidate_q(i, next);
        }
        // A generation whose map follows its field admits the transferred
        // flux only where the map can hold it. A node past its declared
        // bound rejects the handoff here, and the running generation stays;
        // left to the first step, it would fail after the old one was gone.
        if nonlinear {
            temporal_primary_field(i, next, 0.0);
        }
        let next_force = candidate_constitutive_force(i);
        set_candidate_force(i, next_force);
        scratch[i].values.y = exchange;
        if !finite_scalar(next) || !finite_scalar(next_force) {
            reject(STATUS_NON_FINITE);
        }
        inject_at(i);
        return;
    }
    if i < control.counts_a.x + control.counts_a.y {
        let sample = i - control.counts_a.x;
        let value = candidate_b(sample);
        if !all(value >= vec2<f32>(-MAX_FINITE))
            || !all(value <= vec2<f32>(MAX_FINITE)) {
            reject(STATUS_NON_FINITE);
        }
        if temporal_enabled() && field_kind(samples[sample].nodes_b.w) != 0u {
            temporal_complementary_secant(sample, value, 0.0);
        }
        inject_at(i);
        return;
    }
    if i < control.counts_a.w {
        let auxiliary = i - auxiliary_offset();
        let value = candidate_auxiliary(auxiliary);
        if !finite_scalar(value) {
            reject(STATUS_NON_FINITE);
        }
        // Gate O: a transferred `r` past a φ⁴ bound rejects the handoff here,
        // and the running generation stays.
        let first_integrated = control.counts_a.z - control.counts_a.x;
        if restoring() && auxiliary >= first_integrated {
            restoring_force_at(auxiliary - first_integrated, value);
        }
        inject_at(i);
    }
}

@compute @workgroup_size(128)
fn handoff_reduce_prescribed(@builtin(local_invocation_id) id: vec3<u32>) {
    let local = id.x;
    let participating = !stopped();
    var exchange = 0.0;
    if participating {
        for (var node = local; node < control.counts_a.x; node += WORKGROUP_SIZE) {
            exchange += scratch[node].values.y;
        }
    }
    let total = reduce_boundary_scalar(local, exchange);
    if participating && local == 0u {
        control.candidate_accounting_a.y += total;
    }
}

@compute @workgroup_size(128)
fn handoff_clear_scratch(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x < arrayLength(&scratch) { scratch[id.x].values = vec4<f32>(0.0); }
}

@compute @workgroup_size(1)
fn handoff_commit() {
    var failure = atomicLoad(&status.candidate);
    if !finite_vector(control.candidate_accounting_a)
        || !finite_vector(control.candidate_accounting_b)
        || !finite_vector(control.candidate_accounting_c) {
        failure = max(failure, STATUS_NON_FINITE);
    }
    if failure != 0u {
        atomicMax(&status.latch, failure);
        atomicStore(&status.handoff, 0u);
        return;
    }
    control.event.z = accepted_slot() ^ 1u;
    control.accepted_accounting_a = control.candidate_accounting_a;
    control.accepted_accounting_b = control.candidate_accounting_b;
    control.accepted_accounting_c = control.candidate_accounting_c;
    atomicStore(&status.transaction_1, control.clock_u32.w);
    atomicStore(&status.transaction_2, control.clock_u32.z);
    atomicStore(&status.handoff, 0u);
    publish_snapshot_metadata();
}

@compute @workgroup_size(128)
fn rebase_clock_records(@builtin(global_invocation_id) id: vec3<u32>) {
    let i = id.x;
    if stopped() { return; }
    let elapsed = control.clock_f32.y;
    if i < control.counts_a.x && nodes[i].boundary.z != 0u {
        nodes[i].prescribed.w = reduced_phase(
            nodes[i].prescribed.w + nodes[i].prescribed.z * elapsed);
    }
    if i < control.counts_c.z {
        let root = control.table_offsets.y + 4u * i;
        let base = root + 2u * (control.runtime_slots.x & 1u);
        let kind = tables[base + 1u].data.x;
        var next_anchor = 0.0;
        if kind != 0u {
            next_anchor = source_drive(i, elapsed);
        }
        let phase = reduced_phase(
            table_float(base, 3u) + table_float(base, 2u) * elapsed);
        tables[root].data = tables[base].data;
        tables[root + 2u].data = tables[base].data;
        tables[root].data.w = bitcast<u32>(phase);
        tables[root + 2u].data.w = bitcast<u32>(phase);
        if kind != 0u {
            tables[root + 1u].data = tables[base + 1u].data;
            tables[root + 3u].data = tables[base + 1u].data;
            tables[root + 1u].data.y = bitcast<u32>(next_anchor);
            tables[root + 3u].data.y = bitcast<u32>(next_anchor);
        }
    }
    if temporal_enabled() {
        let header = tables[control.runtime_slots.z].data;
        let runtime_count = tables[control.runtime_slots.z + 1u].data.x;
        if i < runtime_count {
            let root = header.w + 6u * i;
            let accepted = root + 3u * (control.runtime_slots.y & 1u);
            var phase = tables[accepted].data;
            let frequency = tables[accepted + 2u].data;
            for (var lane = 0u; lane < 4u; lane += 1u) {
                phase[lane] = bitcast<u32>(reduced_phase(
                    bitcast<f32>(phase[lane])
                    + bitcast<f32>(frequency[lane]) * elapsed));
            }
            var switch_runtime = tables[accepted + 1u].data;
            switch_runtime.z = bitcast<u32>(
                bitcast<f32>(switch_runtime.z) - elapsed);
            tables[root].data = phase;
            tables[root + 1u].data = switch_runtime;
            tables[root + 2u].data = frequency;
            tables[root + 3u].data = phase;
            tables[root + 4u].data = switch_runtime;
            tables[root + 5u].data = frequency;
        }
    }
}

@compute @workgroup_size(1)
fn commit_clock_rebase() {
    if stopped() { return; }
    let origin = add_compensated(
        control.clock_origin.x, control.clock_origin.y, control.clock_f32.y);
    let low = control.clock_u32.x + 1u;
    if low == 0u { control.clock_u32.y += 1u; }
    control.clock_u32.x = low;
    control.clock_u32.z = 0u;
    control.clock_origin = vec4<f32>(origin, origin);
    control.clock_f32.y = 0.0;
    control.clock_f32.z = 0.0;
    if temporal_enabled() { control.runtime_slots.y = 0u; }
    publish_snapshot_metadata();
}

@compute @workgroup_size(128)
fn start_loss(@builtin(global_invocation_id) id: vec3<u32>) {
    let i = id.x;
    if stopped() { return; }
    if i < scratch_count() {
        scratch[i].values = vec4<f32>(0.0);
    }
    if i >= control.counts_a.w { return; }
    let node_count = control.counts_a.x;
    let sample_count = control.counts_a.y;
    if i < node_count {
        let old = accepted_q(i);
        var owned = old;
        var exchange = 0.0;
        // The fixed path holds a pin at both ends of its dissipation, as its
        // reference does. A driven generation's reference pins only in its
        // kicks, at their own stage instants and through the map in force
        // there, and decays the pinned flux with everything else; pinning here
        // at the authored mass would disagree with it at every pinned node.
        if nodes[i].boundary.z != 0u && !temporal_enabled() {
            owned = nodes[i].mass_loss.x
                * harmonic_value(nodes[i].prescribed, control.clock_f32.y);
            exchange = 0.5 * (owned * owned - old * old) * nodes[i].mass_loss.y;
        }
        var next = owned - owned * node_loss_fraction(i, false);
        if node_is_active(i) { next = active_loss_map(i, owned, false); }
        set_candidate_q(i, next);
        if use_force_cache() {
            set_candidate_force(i, accepted_force(i));
        }
        scratch[i].values.y = exchange;
        let stage_time = control.clock_f32.y;
        scratch[i].values.z = stage_primary_energy(i, owned, stage_time)
            - stage_primary_energy(i, next, stage_time);
    } else if i < node_count + sample_count {
        let sample_index = i - node_count;
        let old = accepted_b(sample_index);
        let next = old - old * sample_loss_fraction(sample_index, false);
        set_candidate_b(sample_index, next);
        let stage_time = control.clock_f32.y;
        scratch[i].values.x = stage_complementary_energy(sample_index, old, stage_time)
            - stage_complementary_energy(sample_index, next, stage_time);
    } else {
        let auxiliary = i - auxiliary_offset();
        set_candidate_auxiliary(auxiliary, accepted_auxiliary(auxiliary));
    }
}

fn kick_node(node: u32, second: bool) {
    if stopped() || node >= control.counts_a.x { return; }
    if !second && !has_loss_stages() {
        scratch[node].values = vec4<f32>(0.0);
    }
    let duration = control.evolution.z;
    let source_time = control.clock_f32.y + select(0.0, control.clock_f32.x, second);
    // A driven stage pins and reads its mass at its own endpoint, which is
    // where the temporal-work quadrature has an integration point. The fixed
    // path pins the first kick half a step in, which is free to choose when
    // the mass is constant and is not free when it moves.
    let target_time = control.clock_f32.y
        + select(select(duration, 0.0, temporal_enabled()), control.clock_f32.x, second);
    let source = source_rate(node, source_time);
    // Gate O: an active node's second kick also gathers the short-wave
    // stresses; a law-carrying generation keeps no force cache, so the
    // gather is the one the force needs anyway.
    let short_wave = second && node_is_active(node) && short_wave_offset() != 0u;
    var held_force: f32;
    var viscous = 0.0;
    if short_wave {
        let forces = gathered_forces(node, second, true);
        held_force = forces.x + restoring_force(node, second);
        viscous = forces.y;
    } else {
        held_force = force(node, second);
    }
    let net = source - held_force;
    if nodes[node].boundary.x != NO_INDEX {
        return;
    }
    let old = select(
        accepted_q(node), candidate_q(node), second || has_loss_stages());
    if temporal_enabled() && node_is_nonlinear(node) {
        kick_nonlinear_node(node, old, net, source, held_force, duration, target_time);
        if second { inject_at(node); }
        return;
    }
    // The wall's admittance, the prescribed pin and both energy lanes divide
    // by the nodal mass, and a driven medium's mass moves. The damping itself
    // stays as assembled: that is the frozen reference impedance, which is a
    // documented approximation rather than an oversight.
    var mass = nodes[node].mass_loss.x;
    var inverse_mass = nodes[node].mass_loss.y;
    if temporal_enabled() {
        mass = temporal_primary_mass(node, target_time);
        inverse_mass = 1.0 / mass;
    }
    let damping = nodes[node].damping_support.x;
    let ratio = 0.5 * duration * damping * inverse_mass;
    var next = ((1.0 - ratio) * old + duration * net) / (1.0 + ratio);
    if nodes[node].boundary.z != 0u {
        next = mass * harmonic_value(nodes[node].prescribed, target_time);
    }
    var midpoint = 0.5 * (old + next) * inverse_mass;
    // A driven pin's field at its stage is its signal, so its work into the
    // bulk is the trapezoid of `g·F` the midpoint drift exchanges. The flux
    // jump's quotient would mix in the other endpoint's mass.
    if temporal_enabled() && nodes[node].boundary.z != 0u {
        midpoint = harmonic_value(nodes[node].prescribed, target_time);
    }
    let source_work = duration * midpoint * source;
    let force_work = duration * midpoint * held_force;
    let boundary_loss = duration * damping * midpoint * midpoint;
    let energy_change = 0.5 * (next * next - old * old) * inverse_mass;
    scratch[node].values.x += source_work;
    scratch[node].values.w += boundary_loss;
    if nodes[node].boundary.z != 0u {
        scratch[node].values.y += energy_change - source_work + force_work + boundary_loss;
    }
    // The short-wave viscosity over the whole step, on the drift's midpoint
    // gradient, after the kick as the reference applies it. What it takes is
    // the self-oscillating law's, charged to the loss lane an active node's
    // reduction books as gain. A pin holds.
    if short_wave && nodes[node].boundary.z == 0u {
        let damped = next - control.clock_f32.x * viscous;
        scratch[node].values.z += 0.5 * (next * next - damped * damped) * inverse_mass;
        next = damped;
    }
    set_candidate_q(node, next);
    if !finite_scalar(next) { reject(STATUS_NON_FINITE); }
    if second { inject_at(node); }
}

// The kick at a node whose map follows its field. It is explicit in the bulk
// and implicit in the discrete gradient on an absorbing wall; a pin is
// written through the forward map, `Q = P(g)`. The energy lanes charge source and force work at the field of
// the kick's mean flux, a second-order stand-in for the reference's exact
// discrete gradient: these lanes are diagnostics, and an f32 energy quotient
// would lose more to cancellation than the midpoint rule does.
fn kick_nonlinear_node(
    node: u32, old: f32, net: f32, source: f32, held_force: f32,
    duration: f32, target_time: f32,
) {
    let pinned = nodes[node].boundary.z != 0u;
    let damping = nodes[node].damping_support.x;
    var next = old + duration * net;
    var field: f32;
    if pinned {
        field = harmonic_value(nodes[node].prescribed, target_time);
        next = temporal_primary_flux_of_field(node, field, target_time);
    } else if damping != 0.0 {
        // An absorbing wall makes the kick implicit in the discrete gradient:
        // `f(Q) = Q − old − τ·net + τd·ū(old, Q) = 0`. `f` rises with slope at
        // least one, so a trial point `x` and `x − f(x)` bracket the root.
        let admittance = duration * damping;
        let first = next - old - duration * net
            + admittance * trace_gradient(node, old, next, target_time).x;
        var low = min(next, next - first);
        var high = max(next, next - first);
        var converged = first == 0.0;
        for (var iteration = 0u; iteration < INVERSE_ITERATIONS && !converged; iteration += 1u) {
            let gradient = trace_gradient(node, old, next, target_time);
            let residual = next - old - duration * net + admittance * gradient.x;
            if residual > 0.0 { high = min(high, next); } else { low = max(low, next); }
            var candidate = next - residual / (1.0 + admittance * gradient.y);
            if !(candidate >= low && candidate <= high) { candidate = 0.5 * (low + high); }
            let step = abs(candidate - next);
            next = candidate;
            converged = step <= 4.0 * 1.1920929e-7 * max(abs(next), abs(old));
        }
        if !converged { reject(STATUS_INVERSE_CONVERGENCE); }
        field = trace_gradient(node, old, next, target_time).x;
    } else if source != 0.0 {
        field = temporal_primary_field(node, 0.5 * (old + next), target_time);
    } else {
        // Nothing to charge: the field would only weight a zero source, so
        // the solve that finds it is skipped.
        field = 0.0;
    }
    let source_work = duration * field * source;
    let force_work = duration * field * held_force;
    set_candidate_q(node, next);
    scratch[node].values.x += source_work;
    scratch[node].values.w += duration * damping * field * field;
    if pinned {
        let energy_change = temporal_primary_energy(node, next, target_time)
            - temporal_primary_energy(node, old, target_time);
        scratch[node].values.y += energy_change - source_work + force_work
            + duration * damping * field * field;
    }
    if !finite_scalar(next) { reject(STATUS_NON_FINITE); }
}

@compute @workgroup_size(128)
fn kick_first(@builtin(global_invocation_id) id: vec3<u32>) {
    kick_node(id.x, false);
}

@compute @workgroup_size(128)
fn kick_second(@builtin(global_invocation_id) id: vec3<u32>) {
    kick_node(id.x, true);
}

fn mode_word(mode: u32, word: u32) -> u32 {
    return control.table_offsets.w + mode * MODE_WORDS + word;
}

fn trace_coefficient(mode: u32, trace: u32) -> f32 {
    return packed_boundary_scalar(
        control.boundary_offsets.x,
        mode * control.counts_b.y + trace);
}

fn trace_coefficient_transposed(trace: u32, mode: u32) -> f32 {
    let scalar_count = control.counts_b.y + control.counts_b.z;
    let transposed_offset = control.boundary_offsets.y + (scalar_count + 3u) / 4u;
    return packed_boundary_scalar(
        transposed_offset,
        trace * control.counts_b.z + mode);
}

// `D + Gamma` for a trace row: the mass-free diagonal of the trace system. The
// stage divides it by its own nodal mass, which is what lets one preparation
// serve a moving one.
fn trace_diagonal(trace: u32) -> f32 {
    return packed_boundary_scalar(control.boundary_offsets.y, trace);
}

// `a_k - 1` for a mode: everything the trace system holds beyond its diagonal.
// Zero on modes with no pole block, and of order `(h decay)^2` on the rest,
// because the three-pole residues sum to zero.
fn modal_correction(mode: u32) -> f32 {
    return packed_boundary_scalar(
        control.boundary_offsets.y, control.counts_b.y + mode);
}

// The instant a boundary stage reads its nodal mass at, which is the one the
// local kick pins and divides by at the same stage.
fn boundary_instant(second: bool) -> f32 {
    return control.clock_f32.y
        + select(
            select(control.evolution.z, 0.0, temporal_enabled()),
            control.clock_f32.x,
            second);
}

fn trace_inverse_mass(trace_word: vec4<u32>, instant: f32) -> f32 {
    if temporal_enabled() {
        return temporal_inverse_primary_mass(trace_word.x, instant);
    }
    return bitcast<f32>(trace_word.y);
}

fn reduce_boundary_scalar(local: u32, value: f32) -> f32 {
    reduced_values[local] = value;
    workgroupBarrier();
    var width = WORKGROUP_SIZE / 2u;
    loop {
        if width == 0u { break; }
        if local < width {
            reduced_values[local] += reduced_values[local + width];
        }
        workgroupBarrier();
        width /= 2u;
    }
    return reduced_values[0];
}

fn reduce_boundary_max(local: u32, value: f32) -> f32 {
    reduced_values[local] = value;
    workgroupBarrier();
    var width = WORKGROUP_SIZE / 2u;
    loop {
        if width == 0u { break; }
        if local < width {
            reduced_values[local] = max(reduced_values[local], reduced_values[local + width]);
        }
        workgroupBarrier();
        width /= 2u;
    }
    return reduced_values[0];
}

fn reduce_boundary_vector(local: u32, value: vec2<f32>) -> vec2<f32> {
    mode_values[local] = value;
    workgroupBarrier();
    var width = WORKGROUP_SIZE / 2u;
    loop {
        if width == 0u { break; }
        if local < width {
            mode_values[local] += mode_values[local + width];
        }
        workgroupBarrier();
        width /= 2u;
    }
    return mode_values[0];
}

fn boundary_prepare(mode: u32, local: u32, second: bool) {
    let participating = !stopped() && mode < control.counts_b.z;
    let trace_count = control.counts_b.y;
    var partial_modal = 0.0;
    if participating {
        let instant = boundary_instant(second);
        for (var trace = local; trace < trace_count; trace += WORKGROUP_SIZE) {
            let trace_word = boundary[control.table_offsets.z + trace].data;
            let node = trace_word.x;
            let current = select(
                accepted_q(node), candidate_q(node), second || has_loss_stages());
            var old_field = current * trace_inverse_mass(trace_word, instant);
            if nonlinear_trace() {
                old_field = bitcast<f32>(trace_word.y);
            }
            partial_modal += trace_coefficient(mode, trace) * old_field;
        }
    }
    let modal = reduce_boundary_scalar(local, partial_modal);
    if !participating || local != 0u { return; }
    let mode_header = boundary[mode_word(mode, 0u)].data;
    if !second && !has_loss_stages() {
        scratch[mode_accounting_offset() + mode].values = vec4<f32>(0.0);
    }
    var memory = 0.0;
    var solved = vec3<f32>(0.0);
    var coupling = 0.0;
    if mode_header.z != 0u {
        let auxiliary = mode_header.y;
        let old_z = vec3<f32>(
            select(accepted_auxiliary(auxiliary), candidate_auxiliary(auxiliary), second || has_loss_stages()),
            select(accepted_auxiliary(auxiliary + 1u), candidate_auxiliary(auxiliary + 1u), second || has_loss_stages()),
            select(accepted_auxiliary(auxiliary + 2u), candidate_auxiliary(auxiliary + 2u), second || has_loss_stages()));
        let residue = vec3<f32>(
            boundary_float(mode_word(mode, 1u), 0u),
            boundary_float(mode_word(mode, 1u), 1u),
            boundary_float(mode_word(mode, 1u), 2u));
        memory = dot(residue, old_z);
        let input_gain = vec3<f32>(
            boundary_float(mode_word(mode, 2u), 0u),
            boundary_float(mode_word(mode, 2u), 1u),
            boundary_float(mode_word(mode, 2u), 2u));
        var right_z = old_z + control.evolution.w * input_gain * modal;
        for (var row = 0u; row < 3u; row += 1u) {
            let generator = vec3<f32>(
                boundary_float(mode_word(mode, 3u + row), 0u),
                boundary_float(mode_word(mode, 3u + row), 1u),
                boundary_float(mode_word(mode, 3u + row), 2u));
            right_z[row] += control.evolution.w * dot(generator, old_z);
        }
        for (var row = 0u; row < 3u; row += 1u) {
            let inverse = vec3<f32>(
                boundary_float(mode_word(mode, 6u + row), 0u),
                boundary_float(mode_word(mode, 6u + row), 1u),
                boundary_float(mode_word(mode, 6u + row), 2u));
            solved[row] = dot(inverse, right_z);
        }
        let aqz = vec3<f32>(
            boundary_float(mode_word(mode, 9u), 0u),
            boundary_float(mode_word(mode, 9u), 1u),
            boundary_float(mode_word(mode, 9u), 2u));
        coupling = dot(aqz, solved);
    }
    scratch[mode_temporary_offset() + mode].values = vec4<f32>(solved, coupling);
    scratch[mode_modal_offset() + mode].values = vec4<f32>(modal, memory, 0.0, 0.0);
}

fn boundary_reduce(trace: u32, local: u32, second: bool) {
    let participating = !stopped() && trace < control.counts_b.y;
    var partial = vec2<f32>(0.0);
    if participating {
        for (var mode = local; mode < control.counts_b.z; mode += WORKGROUP_SIZE) {
            let modal_memory = scratch[mode_modal_offset() + mode].values.xy;
            let coefficient = trace_coefficient_transposed(trace, mode);
            partial.x += coefficient * (modal_memory.x + modal_memory.y);
            partial.y += coefficient * scratch[mode_temporary_offset() + mode].values.w;
        }
    }
    let coupling = reduce_boundary_vector(local, partial);
    if !participating || local != 0u { return; }
    let trace_word_index = control.table_offsets.z + trace;
    let trace_word = boundary[trace_word_index].data;
    let node = trace_word.x;
    var inverse_mass = trace_inverse_mass(trace_word, boundary_instant(second));
    let damping = bitcast<f32>(trace_word.z);
    let old = select(
        accepted_q(node), candidate_q(node), second || has_loss_stages());
    var old_field = inverse_mass * old;
    // On a nonlinear wall each Newton iteration is this same linear kick at
    // the per-node mass `1/(2g_k)` with the old field replaced by `2c_k`,
    // `c_k = ū_k − g_k Q_k`: the reference's `nonlinear_outgoing_kick_with`.
    // A linear node has `g = 1/2m` and `c = Q_old/2m`, the values it replaces.
    if nonlinear_trace() {
        inverse_mass = scratch[nonlinear_trace_offset() + trace].values.z;
        old_field = bitcast<f32>(trace_word.y);
    }
    let derivative = -damping * old_field - coupling.x;
    let source_time = control.clock_f32.y
        + select(0.0, control.clock_f32.x, second);
    let source = source_rate(node, source_time);
    let held_force = force(node, second);
    var reduced = old + control.evolution.w * derivative
        + control.evolution.z * (source - held_force);
    if nodes[node].boundary.z != 0u {
        let target_time = control.clock_f32.y
            + select(control.evolution.z, control.clock_f32.x, second);
        reduced = nodes[node].mass_loss.x
            * harmonic_value(nodes[node].prescribed, target_time);
    } else {
        reduced -= coupling.y;
    }
    boundary[trace_word_index].data.w = bitcast<u32>(reduced);
    // Seed the trace solve while this row's mass is already in hand. A
    // prescribed row owns its own value from the start, which is what replacing
    // its row of the matrix by an identity row meant.
    let held = nodes[node].boundary.z != 0u;
    let scale = select(
        1.0 / (1.0 + control.evolution.w * trace_diagonal(trace) * inverse_mass),
        1.0,
        held);
    scratch[trace_solution_offset() + trace].values =
        vec4<f32>(select(0.0, reduced, held), inverse_mass, scale, f32(held));
}

// The reduced trace system is `I + (h/2) K M^-1` with
// `K = diag(D + Gamma) + sum_k (a_k - 1) t_k t_k^T`, and none of `K` depends on
// the nodal mass: eliminating the pole blocks only rescales a mode by a number
// built from its decay and the step. So one preparation serves every stage and
// the mass arrives at the solve, which is what lets a driven medium sit behind
// this wall at all.
//
// Solving it is a diagonal-preconditioned sweep rather than a factorization,
// because the three-pole DtN residues `6/7, -8/7, 2/7` sum to zero: that makes
// `a_k - 1` of order `(h decay_k)^2`, so the correction is a small perturbation
// of the diagonal and the sweep contracts by `max_k |a_k - 1|` per pass
// whatever the mass is. The host encoded one pair of dispatches per pass, from
// a count it derived from that bound.
//
// A pass is two reductions with a barrier between them, and the only barrier
// that spans the whole trace is the one between dispatches - so each half is
// its own kernel, and each stays as wide as the boundary is. The two halves
// read and write different arrays, so neither needs a second lane.
fn boundary_sweep_modal(mode: u32, local: u32) {
    let participating = !stopped() && mode < control.counts_b.z;
    var partial = 0.0;
    if participating {
        for (var trace = local; trace < control.counts_b.y; trace += WORKGROUP_SIZE) {
            let slot = scratch[trace_solution_offset() + trace].values;
            partial += trace_coefficient(mode, trace) * slot.x * slot.y;
        }
    }
    let modal = reduce_boundary_scalar(local, partial);
    if participating && local == 0u {
        scratch[mode_modal_offset() + mode].values.z =
            control.evolution.w * modal_correction(mode) * modal;
    }
}

fn boundary_sweep_trace(trace: u32, local: u32) {
    let participating = !stopped() && trace < control.counts_b.y;
    var partial = 0.0;
    if participating {
        for (var mode = local; mode < control.counts_b.z; mode += WORKGROUP_SIZE) {
            partial += trace_coefficient_transposed(trace, mode)
                * scratch[mode_modal_offset() + mode].values.z;
        }
    }
    let coupled = reduce_boundary_scalar(local, partial);
    if participating && local == 0u {
        let slot = scratch[trace_solution_offset() + trace].values;
        let reduced = bitcast<f32>(boundary[control.table_offsets.z + trace].data.w);
        scratch[trace_solution_offset() + trace].values.x =
            select((reduced - coupled) * slot.z, reduced, slot.w != 0.0);
    }
}

// A fixed generation's trace inverse, packed straight after the transposed
// trace matrix. Only a generation whose mass cannot move carries one, and only
// when no trace row is prescribed, so the seeded right-hand side is the whole
// of what it acts on.
fn direct_trace_offset() -> u32 {
    let scalar_count = control.counts_b.y + control.counts_b.z;
    let transposed_offset = control.boundary_offsets.y + (scalar_count + 3u) / 4u;
    return transposed_offset + (control.counts_b.y * control.counts_b.z + 3u) / 4u;
}

// One row of `(I + (h/2) K M^-1)^-1` against the reduced right-hand side that
// `boundary_reduce` left in each trace word: the sweeps' answer in one pass.
fn boundary_direct_trace(trace: u32, local: u32) {
    let trace_count = control.counts_b.y;
    let participating = !stopped() && trace < trace_count;
    var partial = 0.0;
    if participating {
        let row = trace * trace_count;
        for (var column = local; column < trace_count; column += WORKGROUP_SIZE) {
            let reduced = bitcast<f32>(boundary[control.table_offsets.z + column].data.w);
            partial += packed_boundary_scalar(direct_trace_offset(), row + column) * reduced;
        }
    }
    let solved = reduce_boundary_scalar(local, partial);
    if participating && local == 0u {
        scratch[trace_solution_offset() + trace].values.x = solved;
    }
}

fn boundary_finalize(i: u32, local: u32, second: bool) {
    let trace_count = control.counts_b.y;
    let mode_count = control.counts_b.z;
    let participating = !stopped() && i < max(trace_count, mode_count);
    let duration = control.evolution.z;
    var partial_modal = 0.0;
    if participating && i < mode_count {
        for (var trace = local; trace < trace_count; trace += WORKGROUP_SIZE) {
            let slot = scratch[trace_solution_offset() + trace].values;
            partial_modal += trace_coefficient(i, trace) * slot.x * slot.y;
        }
    }
    let new_modal = reduce_boundary_scalar(local, partial_modal);
    // The largest trace flux, which is the scale a Newton step is judged
    // against: a node near zero has no scale of its own, and the sweep's f32
    // floor is set by the whole trace, not by that node.
    var trace_scale = 0.0;
    if local == 0u {
        uniform_flag = u32(nonlinear_trace());
    }
    if workgroupUniformLoad(&uniform_flag) != 0u {
        var partial_scale = 0.0;
        for (var trace = local; trace < trace_count; trace += WORKGROUP_SIZE) {
            partial_scale = max(
                partial_scale, abs(scratch[trace_solution_offset() + trace].values.x));
        }
        trace_scale = reduce_boundary_max(local, partial_scale);
    }
    if !participating || local != 0u { return; }
    if i < trace_count {
        let trace = i;
        let trace_word = boundary[control.table_offsets.z + trace].data;
        let node = trace_word.x;
        let inverse_mass = scratch[trace_solution_offset() + trace].values.y;
        let damping = bitcast<f32>(trace_word.z);
        let old = select(
            accepted_q(node), candidate_q(node), second || has_loss_stages());
        let next = scratch[trace_solution_offset() + trace].values.x;
        let source_time = control.clock_f32.y
            + select(0.0, control.clock_f32.x, second);
        let source = source_rate(node, source_time);
        let held_force = force(node, second);
        var midpoint = 0.5 * (old + next) * inverse_mass;
        if nonlinear_trace() {
            // The last Newton step must have settled: a solve still moving
            // at its cap is a detected failure, not an accepted state.
            let iterate = scratch[nonlinear_trace_offset() + trace].values.x;
            if abs(next - iterate) > 1.0e-5 * max(trace_scale, 1.0e-30) {
                reject(STATUS_INVERSE_CONVERGENCE);
            }
            midpoint = trace_gradient(node, old, next, boundary_instant(second)).x;
        }
        let source_work = duration * midpoint * source;
        let force_work = duration * midpoint * held_force;
        let boundary_loss = duration * damping * midpoint * midpoint;
        let energy_change = 0.5 * (next * next - old * old) * inverse_mass;
        set_candidate_q(node, next);
        scratch[node].values.x += source_work;
        scratch[node].values.w += boundary_loss;
        if has_prescribed_trace() {
            scratch[node].values.y += energy_change - source_work + force_work + boundary_loss;
        }
        if !finite_scalar(next) { reject(STATUS_NON_FINITE); }
        if second { inject_at(node); }
    }

    if i < mode_count {
        let mode = i;
        let mode_header = boundary[mode_word(mode, 0u)].data;
        var auxiliary_change = 0.0;
        var midpoint_memory = 0.0;
        if mode_header.z != 0u {
            let auxiliary = mode_header.y;
            let old_z = vec3<f32>(
                select(accepted_auxiliary(auxiliary), candidate_auxiliary(auxiliary), second || has_loss_stages()),
                select(accepted_auxiliary(auxiliary + 1u), candidate_auxiliary(auxiliary + 1u), second || has_loss_stages()),
                select(accepted_auxiliary(auxiliary + 2u), candidate_auxiliary(auxiliary + 2u), second || has_loss_stages()));
            var new_z = scratch[mode_temporary_offset() + mode].values.xyz;
            let solved_column = vec3<f32>(
                boundary_float(mode_word(mode, 10u), 0u),
                boundary_float(mode_word(mode, 10u), 1u),
                boundary_float(mode_word(mode, 10u), 2u));
            new_z = vec3<f32>(
                fma(-solved_column.x, new_modal, new_z.x),
                fma(-solved_column.y, new_modal, new_z.y),
                fma(-solved_column.z, new_modal, new_z.z));
            set_candidate_auxiliary(auxiliary, new_z.x);
            set_candidate_auxiliary(auxiliary + 1u, new_z.y);
            set_candidate_auxiliary(auxiliary + 2u, new_z.z);
            auxiliary_change = 0.5 * (dot(new_z, new_z) - dot(old_z, old_z));
            let loss_coefficient = vec3<f32>(
                boundary_float(mode_word(mode, 11u), 0u),
                boundary_float(mode_word(mode, 11u), 1u),
                boundary_float(mode_word(mode, 11u), 2u));
            midpoint_memory = dot(loss_coefficient, 0.5 * (old_z + new_z));
            if !all(new_z >= vec3<f32>(-MAX_FINITE))
                || !all(new_z <= vec3<f32>(MAX_FINITE)) {
                reject(STATUS_NON_FINITE);
            }
            if second {
                inject_at(auxiliary_offset() + auxiliary);
                inject_at(auxiliary_offset() + auxiliary + 1u);
                inject_at(auxiliary_offset() + auxiliary + 2u);
            }
        }
        let old_modal = scratch[mode_modal_offset() + mode].values.x;
        let midpoint_field = 0.5 * (old_modal + new_modal);
        let outgoing_loss = duration
            * (midpoint_field + midpoint_memory) * (midpoint_field + midpoint_memory);
        scratch[mode_accounting_offset() + mode].values.x += auxiliary_change;
        scratch[mode_accounting_offset() + mode].values.w += outgoing_loss;
    }
}

// One Newton linearization of a nonlinear wall's trace: from the iterate
// `Q_k` (the old flux on the first pass, the last solve's after), the
// discrete gradient `ū_k`, its slope `g_k`, the old field `2c_k` the prepare
// and reduce passes read in place of `Q_old/m`, and the step just taken,
// which the finalize checks.
fn boundary_linearize(trace: u32, second: bool, first_iteration: bool) {
    if stopped() || trace >= control.counts_b.y { return; }
    let word_index = control.table_offsets.z + trace;
    let node = boundary[word_index].data.x;
    let old = select(accepted_q(node), candidate_q(node), second || has_loss_stages());
    let region = nonlinear_trace_offset() + trace;
    var iterate = old;
    var step = 0.0;
    if !first_iteration {
        iterate = scratch[trace_solution_offset() + trace].values.x;
        step = abs(iterate - scratch[region].values.x);
    }
    let gradient = trace_gradient(node, old, iterate, boundary_instant(second));
    boundary[word_index].data.y = bitcast<u32>(2.0 * (gradient.x - gradient.y * iterate));
    scratch[region].values = vec4<f32>(iterate, gradient.x, 2.0 * gradient.y, step);
}

@compute @workgroup_size(128)
fn boundary_linearize_begin_first(@builtin(global_invocation_id) id: vec3<u32>) {
    boundary_linearize(id.x, false, true);
}

@compute @workgroup_size(128)
fn boundary_linearize_first(@builtin(global_invocation_id) id: vec3<u32>) {
    boundary_linearize(id.x, false, false);
}

@compute @workgroup_size(128)
fn boundary_linearize_begin_second(@builtin(global_invocation_id) id: vec3<u32>) {
    boundary_linearize(id.x, true, true);
}

@compute @workgroup_size(128)
fn boundary_linearize_second(@builtin(global_invocation_id) id: vec3<u32>) {
    boundary_linearize(id.x, true, false);
}

@compute @workgroup_size(128)
fn boundary_prepare_first(
    @builtin(workgroup_id) group: vec3<u32>,
    @builtin(local_invocation_id) local: vec3<u32>,
) {
    boundary_prepare(group.x, local.x, false);
}

@compute @workgroup_size(128)
fn boundary_reduce_first(
    @builtin(workgroup_id) group: vec3<u32>,
    @builtin(local_invocation_id) local: vec3<u32>,
) {
    boundary_reduce(group.x, local.x, false);
}

@compute @workgroup_size(128)
fn boundary_sweep_modal_pass(
    @builtin(workgroup_id) group: vec3<u32>,
    @builtin(local_invocation_id) local: vec3<u32>,
) {
    boundary_sweep_modal(group.x, local.x);
}

@compute @workgroup_size(128)
fn boundary_sweep_trace_pass(
    @builtin(workgroup_id) group: vec3<u32>,
    @builtin(local_invocation_id) local: vec3<u32>,
) {
    boundary_sweep_trace(group.x, local.x);
}

@compute @workgroup_size(128)
fn boundary_direct_trace_pass(
    @builtin(workgroup_id) group: vec3<u32>,
    @builtin(local_invocation_id) local: vec3<u32>,
) {
    boundary_direct_trace(group.x, local.x);
}

@compute @workgroup_size(128)
fn boundary_finalize_first(
    @builtin(workgroup_id) group: vec3<u32>,
    @builtin(local_invocation_id) local: vec3<u32>,
) {
    boundary_finalize(group.x, local.x, false);
}

@compute @workgroup_size(128)
fn boundary_prepare_second(
    @builtin(workgroup_id) group: vec3<u32>,
    @builtin(local_invocation_id) local: vec3<u32>,
) {
    boundary_prepare(group.x, local.x, true);
}

@compute @workgroup_size(128)
fn boundary_reduce_second(
    @builtin(workgroup_id) group: vec3<u32>,
    @builtin(local_invocation_id) local: vec3<u32>,
) {
    boundary_reduce(group.x, local.x, true);
}

@compute @workgroup_size(128)
fn boundary_finalize_second(
    @builtin(workgroup_id) group: vec3<u32>,
    @builtin(local_invocation_id) local: vec3<u32>,
) {
    boundary_finalize(group.x, local.x, true);
}

// The primary field a driven drift reads at its midpoint. A prescribed node's
// flux is pinned at the step endpoints, where the kicks and the work quadrature
// stage, so its field is read from its signal at the drift's own instant;
// dividing the endpoint flux by the midpoint mass would be first order.
fn temporal_drift_field(node: u32, middle_time: f32) -> f32 {
    if nodes[node].boundary.z != 0u {
        return harmonic_value(nodes[node].prescribed, middle_time);
    }
    if field_laws() {
        return scratch[node_field_offset() + node].values.x;
    }
    return candidate_q(node) * temporal_inverse_primary_mass(node, middle_time);
}

@compute @workgroup_size(128)
fn nonlinear_node_fields(@builtin(global_invocation_id) id: vec3<u32>) {
    let node = id.x;
    if stopped() || node >= control.counts_a.x { return; }
    let middle_time = control.clock_f32.y + 0.5 * control.clock_f32.x;
    scratch[node_field_offset() + node].values.x =
        temporal_primary_field(node, candidate_q(node), middle_time);
}

fn cache_sample_secant(sample: u32, second: bool) {
    if stopped() || sample >= control.counts_a.y { return; }
    let local_time = control.clock_f32.y + select(0.0, control.clock_f32.x, second);
    let flux = select(accepted_b(sample), candidate_b(sample), second || has_loss_stages());
    scratch[sample_secant_offset() + sample].values.x =
        temporal_complementary_secant(sample, flux, local_time);
}

@compute @workgroup_size(128)
fn nonlinear_sample_secants_first(@builtin(global_invocation_id) id: vec3<u32>) {
    cache_sample_secant(id.x, false);
}

@compute @workgroup_size(128)
fn nonlinear_sample_secants_second(@builtin(global_invocation_id) id: vec3<u32>) {
    cache_sample_secant(id.x, true);
}

@compute @workgroup_size(128)
fn drift(@builtin(global_invocation_id) id: vec3<u32>) {
    let i = id.x;
    if stopped() { return; }
    // `ṙ = u` drifts with `b`, on the same midpoint field and interval.
    if i < control.counts_a.x && restoring() {
        let index = integrated_index(i);
        let middle_time = control.clock_f32.y + 0.5 * control.clock_f32.x;
        let next = accepted_auxiliary(index)
            + control.clock_f32.x * temporal_drift_field(i, middle_time);
        set_candidate_auxiliary(index, next);
        if !finite_scalar(next) { reject(STATUS_NON_FINITE); }
        inject_at(auxiliary_offset() + index);
    }
    if i < control.counts_a.x && use_force_cache() {
        let next_force = accepted_force(i)
            + control.clock_f32.x
                * stiffness_force(i, control.clock_f32.y + 0.5 * control.clock_f32.x);
        set_candidate_force(i, next_force);
        if !finite_scalar(next_force) { reject(STATUS_NON_FINITE); }
    }
    if i < control.counts_a.y {
        if !has_loss_stages() {
            scratch[complementary_offset() + i].values = vec4<f32>(0.0);
        }
        let sample = samples[i];
        var curl = vec2<f32>(0.0);
        if temporal_enabled() {
            let middle_time = control.clock_f32.y + 0.5 * control.clock_f32.x;
            let reference = temporal_drift_field(sample.nodes_a.x, middle_time);
            for (var local = 1u; local < 7u; local += 1u) {
                let node = sample_node(sample, local);
                let field = temporal_drift_field(node, middle_time);
                curl += sample_curl(sample, local) * (field - reference);
            }
        } else {
            let reference = candidate_q(sample.nodes_a.x)
                * nodes[sample.nodes_a.x].mass_loss.y;
            for (var local = 1u; local < 7u; local += 1u) {
                let node = sample_node(sample, local);
                let field = candidate_q(node) * nodes[node].mass_loss.y;
                curl += sample_curl(sample, local) * (field - reference);
            }
        }
        let old = select(accepted_b(i), candidate_b(i), has_loss_stages());
        let next = old + control.evolution.y * control.clock_f32.x * curl;
        set_candidate_b(i, next);
        // Gate O: a self-oscillating sample's viscous stress `τ η C u` on the
        // same midpoint gradient, for the second kick to gather. The loss
        // stage zeroed the lanes, so a sample without it gathers nothing.
        let short_wave = short_wave_offset();
        if short_wave != 0u {
            let viscosity = table_float(short_wave + i / 4u, i % 4u);
            if viscosity != 0.0 {
                let stress = viscosity * control.evolution.y * curl;
                scratch[complementary_offset() + i].values.z = stress.x;
                scratch[complementary_offset() + i].values.w = stress.y;
            }
        }
        if !all(next >= vec2<f32>(-MAX_FINITE))
            || !all(next <= vec2<f32>(MAX_FINITE)) {
            reject(STATUS_NON_FINITE);
        }
        inject_at(complementary_offset() + i);
    }
    if i < control.counts_b.x {
        let record = tables[control.table_offsets.x + i].data;
        let left = record.x;
        let right = record.y;
        let auxiliary_word = record.z;
        // The gap drifts on the same field and over the same interval as the
        // bulk does, so a driven one reads the same midpoint field: dividing
        // by the authored mass under a pump misses by the modulation depth.
        var difference = candidate_q(left) * nodes[left].mass_loss.y
            - candidate_q(right) * nodes[right].mass_loss.y;
        if temporal_enabled() {
            let middle_time = control.clock_f32.y + 0.5 * control.clock_f32.x;
            difference = temporal_drift_field(left, middle_time)
                - temporal_drift_field(right, middle_time);
        }
        let auxiliary = auxiliary_word - auxiliary_offset();
        let next = accepted_auxiliary(auxiliary) + control.clock_f32.x * difference;
        set_candidate_auxiliary(auxiliary, next);
        if !finite_scalar(next) { reject(STATUS_NON_FINITE); }
        inject_at(auxiliary_word);
    }
}

@compute @workgroup_size(128)
fn finish_loss_validate(@builtin(global_invocation_id) id: vec3<u32>) {
    let i = id.x;
    if stopped() { return; }
    if i >= control.counts_a.w { return; }
    let node_count = control.counts_a.x;
    let sample_count = control.counts_a.y;
    if i < node_count {
        let before = candidate_q(i);
        var next = before - before * node_loss_fraction(i, true);
        if node_is_active(i) { next = active_loss_map(i, before, true); }
        let stage_time = control.clock_f32.y + control.clock_f32.x;
        scratch[i].values.z += stage_primary_energy(i, before, stage_time)
            - stage_primary_energy(i, next, stage_time);
        if nodes[i].boundary.z != 0u && !temporal_enabled() {
            let owned = nodes[i].mass_loss.x
                * harmonic_value(nodes[i].prescribed, control.clock_f32.y + control.clock_f32.x);
            scratch[i].values.y += 0.5 * (owned * owned - next * next) * nodes[i].mass_loss.y;
            next = owned;
        }
        set_candidate_q(i, next);
        if !finite_scalar(next) { reject(STATUS_NON_FINITE); }
    } else if i < node_count + sample_count {
        let sample_index = i - node_count;
        let before = candidate_b(sample_index);
        let next = before - before * sample_loss_fraction(sample_index, true);
        let stage_time = control.clock_f32.y + control.clock_f32.x;
        scratch[i].values.x += stage_complementary_energy(sample_index, before, stage_time)
            - stage_complementary_energy(sample_index, next, stage_time);
        set_candidate_b(sample_index, next);
        if !all(next >= vec2<f32>(-MAX_FINITE))
            || !all(next <= vec2<f32>(MAX_FINITE)) {
            reject(STATUS_NON_FINITE);
        }
    } else if !finite_scalar(state[i].values.y) {
        reject(STATUS_NON_FINITE);
    }
    let injection = atomicLoad(&status.injection);
    if injection != 0u && i == atomicLoad(&status.injection_index) {
        reject(injection);
    }
}

fn reduce_accounting_half(local: u32, value: vec4<f32>) -> vec4<f32> {
    mode_values[local] = value.xy;
    reduced_values[local] = value.z;
    solved_values[local] = value.w;
    workgroupBarrier();
    var width = WORKGROUP_SIZE / 2u;
    loop {
        if width == 0u { break; }
        if local < width {
            mode_values[local] += mode_values[local + width];
            reduced_values[local] += reduced_values[local + width];
            solved_values[local] += solved_values[local + width];
        }
        workgroupBarrier();
        width /= 2u;
    }
    return vec4<f32>(mode_values[0], reduced_values[0], solved_values[0]);
}

@compute @workgroup_size(128)
fn stage_accounting(@builtin(global_invocation_id) id: vec3<u32>) {
    let i = id.x;
    if stopped() || i >= accounting_item_count() { return; }
    var contribution = vec4<f32>(0.0);
    if i < control.counts_a.x {
        contribution = scratch[i].values;
    } else if i < control.counts_a.x + control.counts_a.y {
        let sample = i - control.counts_a.x;
        contribution.x = scratch[complementary_offset() + sample].values.x;
    } else {
        let mode = i - control.counts_a.x - control.counts_a.y;
        contribution = scratch[mode_accounting_offset() + mode].values;
    }
    let accepted = accounting_bank_offset(accepted_slot()) + i;
    let candidate = accounting_bank_offset(accepted_slot() ^ 1u) + i;
    scratch[candidate].values = scratch[accepted].values + contribution;
}

@compute @workgroup_size(128)
fn reduce_accounting(@builtin(local_invocation_id) id: vec3<u32>) {
    let local = id.x;
    var accounting_a = vec4<f32>(0.0);
    for (var node = local; node < control.counts_a.x; node += WORKGROUP_SIZE) {
        let item = scratch[accounting_bank_offset(accepted_slot()) + node].values;
        accounting_a.x += item.x;
        if nodes[node].boundary.x == NO_INDEX || has_prescribed_trace() {
            accounting_a.y += item.y;
        }
        // An active node's loss stage is gain of either sign, charged below.
        if !node_is_active(node) { accounting_a.z += item.z; }
    }
    for (var sample = local; sample < control.counts_a.y; sample += WORKGROUP_SIZE) {
        let item = control.counts_a.x + sample;
        accounting_a.w += scratch[accounting_bank_offset(accepted_slot()) + item].values.x;
    }
    for (var mode = local; mode < control.counts_b.z; mode += WORKGROUP_SIZE) {
        if has_prescribed_trace() {
            let item_index = control.counts_a.x + control.counts_a.y + mode;
            let item = scratch[accounting_bank_offset(accepted_slot()) + item_index].values;
            accounting_a.y += item.x + item.w;
        }
    }
    let total_a = reduce_accounting_half(local, accounting_a);
    if local == 0u {
        control.accepted_accounting_a = control.candidate_accounting_a + total_a;
    }
    workgroupBarrier();

    // The step fills only `b.x`; `b.y` carries the active gain through the
    // reduction and lands in `c.x`.
    var accounting_b = vec4<f32>(0.0);
    for (var node = local; node < control.counts_a.x; node += WORKGROUP_SIZE) {
        let item = scratch[accounting_bank_offset(accepted_slot()) + node].values;
        accounting_b.x += item.w;
        if node_is_active(node) { accounting_b.y -= item.z; }
    }
    for (var mode = local; mode < control.counts_b.z; mode += WORKGROUP_SIZE) {
        let item_index = control.counts_a.x + control.counts_a.y + mode;
        accounting_b.x += scratch[accounting_bank_offset(accepted_slot()) + item_index].values.w;
    }
    let total_b = reduce_accounting_half(local, accounting_b);
    if local == 0u {
        control.accepted_accounting_b = control.candidate_accounting_b
            + vec4<f32>(total_b.x, 0.0, 0.0, 0.0);
        control.accepted_accounting_c = control.candidate_accounting_c
            + vec4<f32>(total_b.y, 0.0, 0.0, 0.0);
    }
}

@compute @workgroup_size(128)
fn commit_step(@builtin(global_invocation_id) id: vec3<u32>) {
    let i = id.x;
    if i != 0u { return; }
    let failure = atomicLoad(&status.candidate);
    if failure != 0u {
        atomicMax(&status.latch, failure);
        return;
    }
    control.event.z = accepted_slot() ^ 1u;
    control.clock_u32.z += 1u;
    control.clock_u32.w += 1u;
    // The epoch-local time is the step count times the step, formed afresh
    // each step. Adding the step to an f32 running time instead rounds at the
    // spacing of `t` every step and keeps the error: 8e-6 s behind by step
    // 400, and 0.12 s behind across a full epoch, which every source, drive
    // and Switch then read.
    control.clock_f32.y = f32(control.clock_u32.z) * control.clock_f32.x;
    control.clock_f32.z = control.clock_f32.y;
    control.event.x = control.event.y;
    publish_snapshot_metadata();
}
