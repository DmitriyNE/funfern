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
struct FarFieldStencil {
    nodes_a: vec4<u32>, nodes_b: vec4<u32>,
    weights_a: vec4<f32>, weights_b: vec4<f32>,
    gradient_x_a: vec4<f32>, gradient_x_b: vec4<f32>,
    gradient_y_a: vec4<f32>, gradient_y_b: vec4<f32>,
    position_normal: vec4<f32>,
}
struct FarFieldControl { sampling: vec4<f32>, projection: vec4<f32> }
struct ProbeSample { values: vec4<f32> }
// An unwritten frame. Not a NaN, which fast math would compare as data.
const UNRECORDED: f32 = -1.0e9;

@group(0) @binding(0) var<storage, read> control: Control;
@group(0) @binding(1) var<storage, read> state: array<StateWord>;
@group(0) @binding(2) var<storage, read> nodes: array<Node>;
@group(0) @binding(3) var<storage, read> stencils: array<FarFieldStencil>;
@group(0) @binding(4) var<storage, read> far_control: FarFieldControl;
@group(0) @binding(5) var<storage, read_write> contour_history: array<ProbeSample>;
@group(0) @binding(6) var<storage, read_write> directional_history: array<ProbeSample>;

fn accepted_q(node: u32) -> f32 {
    return select(state[node].values.x, state[node].values.y, (control.event.z & 1u) != 0u);
}
fn previous_q(node: u32) -> f32 {
    return select(state[node].values.y, state[node].values.x, (control.event.z & 1u) != 0u);
}
fn ring_frame(frame: u32, age: u32, frames: u32) -> u32 {
    return (frame + frames - (age % frames)) % frames;
}
fn state_time() -> f32 {
    return control.clock_origin.x + control.clock_origin.y + control.clock_f32.y;
}
fn bucket_of(time: f32) -> f32 { return floor(time / far_control.sampling.x); }
fn bucket_frame(bucket: f32, frames: u32) -> u32 {
    let count = f32(frames);
    return u32(bucket - floor(bucket / count) * count);
}
fn bucket_recorded(recorded: f32, bucket: f32) -> bool {
    return recorded > UNRECORDED * 0.5 && bucket_of(recorded) == bucket;
}

@compute @workgroup_size(64)
fn sample_contour(@builtin(global_invocation_id) invocation: vec3<u32>) {
    let point = invocation.x;
    let point_count = u32(far_control.sampling.z);
    if point >= point_count { return; }
    let time = state_time();
    let frames = u32(far_control.sampling.y);
    let bucket = bucket_of(time);
    let index = bucket_frame(bucket, frames) * point_count + point;
    if bucket_recorded(contour_history[index].values.w, bucket) { return; }
    let stencil = stencils[point];
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
    let primary = dot(current_a, stencil.weights_a) + dot(current_b, stencil.weights_b);
    let previous = dot(previous_a, stencil.weights_a) + dot(previous_b, stencil.weights_b);
    let rate = (primary - previous) / control.clock_f32.x;
    let gradient = vec2<f32>(
        dot(current_a, stencil.gradient_x_a) + dot(current_b, stencil.gradient_x_b),
        dot(current_a, stencil.gradient_y_a) + dot(current_b, stencil.gradient_y_b));
    let normal_gradient = dot(gradient, stencil.position_normal.zw);
    contour_history[index].values = vec4<f32>(primary, rate, normal_gradient, time);
}

// Portable trigonometry: this block is the same in every shader that needs
// it, which `every_shader_carries_the_same_portable_trigonometry` holds.
// WGSL promises its own sin and cos only to 2^-11 absolute, and SwiftShader's
// are 1.9e-4 off everywhere, which carried a harmonic drive's field 2e-4 off
// the reference. These are 9e-8 off from multiply-adds alone: the argument
// loses its multiple of pi/2 in three parts, the first two short enough that
// their products with it are exact, through fma - Metal's fast math folded
// the plain differences back into one rounded pi/2 - and Cephes'
// single-precision polynomials take the rest on [-pi/4, pi/4].
//
// They give up the GPU's special-function units on purpose. A builtin is one
// or two of their instructions and this about twenty multiply-adds, but the
// step is bound by memory, not arithmetic: on the M1 Max the driven step costs
// 0.3% more for it and the sine-Gordon step, two calls a node a stage, 1.9%.
// Calls that repeat across nodes - a pump's carrier is its material's - are
// still made in place rather than once a stage, since handing the values on
// through memory would add to the traffic the step is bound by. The same
// arithmetic everywhere is the point: Metal and SwiftShader read the same
// errors at the same arguments.
const TRIG_TWO_OVER_PI: f32 = 0.63661975;
const TRIG_HALF_PI_1: f32 = 1.5703125;
const TRIG_HALF_PI_2: f32 = 4.837512969970703e-4;
const TRIG_HALF_PI_3: f32 = 7.549790126404332e-8;

fn portable_sin_cos(x: f32) -> vec2<f32> {
    let k = round(x * TRIG_TWO_OVER_PI);
    let r = fma(-k, TRIG_HALF_PI_3, fma(-k, TRIG_HALF_PI_2, fma(-k, TRIG_HALF_PI_1, x)));
    let z = r * r;
    let s = r + r * z * (-1.6666654611e-1 + z * (8.3321608736e-3 + z * -1.9515295891e-4));
    let c = 1.0 - 0.5 * z
        + z * z * (4.166664568298827e-2 + z * (-1.388731625493765e-3 + z * 2.443315711809948e-5));
    let quadrant = i32(k) & 3;
    let swap = (quadrant & 1) != 0;
    let sine = select(s, c, swap);
    let cosine = select(c, s, swap);
    return vec2<f32>(
        select(sine, -sine, (quadrant & 2) != 0),
        select(cosine, -cosine, ((quadrant + 1) & 2) != 0),
    );
}

fn portable_sin(x: f32) -> f32 { return portable_sin_cos(x).x; }

fn portable_cos(x: f32) -> f32 { return portable_sin_cos(x).y; }

// The angle `value` names, in [-pi, pi]: its multiple of 2 pi taken out as
// above.
fn reduced_phase(value: f32) -> f32 {
    let k = round(value * (0.25 * TRIG_TWO_OVER_PI));
    return fma(-k, 4.0 * TRIG_HALF_PI_3,
        fma(-k, 4.0 * TRIG_HALF_PI_2, fma(-k, 4.0 * TRIG_HALF_PI_1, value)));
}
// End of the portable trigonometry.

@compute @workgroup_size(64)
fn project_directions(@builtin(global_invocation_id) invocation: vec3<u32>) {
    let direction = invocation.x;
    let direction_count = u32(far_control.sampling.w);
    if direction >= direction_count { return; }
    let now = state_time();
    let frames = u32(far_control.sampling.y);
    let point_count = u32(far_control.sampling.z);
    let period = far_control.sampling.x;
    let bucket = bucket_of(now);
    let current_frame = bucket_frame(bucket, frames);
    let wave_speed = far_control.projection.x;
    let spacing = far_control.projection.y;
    let margin = far_control.projection.z;
    let far_time = now - margin;
    let slot = current_frame * direction_count + direction;
    if bucket_recorded(directional_history[slot].values.z + margin, bucket) { return; }
    let angle = 6.283185307179586 * f32(direction) / f32(direction_count);
    let ray = vec2<f32>(portable_cos(angle), portable_sin(angle));
    var amplitude = 0.0;
    var valid = true;
    for (var point = 0u; point < point_count; point += 1u) {
        let stencil = stencils[point];
        let position = stencil.position_normal.xy;
        let normal = stencil.position_normal.zw;
        let delay = margin - dot(ray, position) / wave_speed;
        let retarded = now - delay;
        let age = delay / period;
        if age < 0.0 || age >= f32(frames - 1u) {
            valid = false;
            continue;
        }
        var index = u32(floor(age));
        var newer = contour_history[ring_frame(current_frame, index, frames) * point_count + point].values;
        var older = contour_history[ring_frame(current_frame, index + 1u, frames) * point_count + point].values;
        for (var step = 0u; step < 4u; step += 1u) {
            if newer.w < 0.0 || older.w < 0.0 { break; }
            if retarded > newer.w && index > 0u {
                index -= 1u;
            } else if retarded < older.w && index + 2u < frames {
                index += 1u;
            } else {
                break;
            }
            newer = contour_history[ring_frame(current_frame, index, frames) * point_count + point].values;
            older = contour_history[ring_frame(current_frame, index + 1u, frames) * point_count + point].values;
        }
        if newer.w < 0.0 || older.w < 0.0
            || retarded > newer.w + period || retarded < older.w - period {
            valid = false;
            continue;
        }
        let span = newer.w - older.w;
        var fraction = 0.0;
        if span > 0.0 { fraction = clamp((newer.w - retarded) / span, 0.0, 1.0); }
        let sample = mix(newer.xyz, older.xyz, fraction);
        amplitude += spacing * (sample.z - dot(normal, ray) * sample.y / wave_speed);
    }
    if valid {
        directional_history[slot].values = vec4<f32>(amplitude, amplitude * amplitude, far_time, 1.0);
    } else {
        directional_history[slot].values = vec4<f32>(0.0, 0.0, far_time, 0.0);
    }
}
