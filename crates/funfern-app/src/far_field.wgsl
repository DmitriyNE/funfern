struct Parameters {
    time_data: vec4<f32>,
    count_data: vec4<u32>,
}

struct State {
    levels: vec4<f32>,
    auxiliary: vec4<f32>,
    reconstruction: vec4<f32>,
}

struct FarFieldStencil {
    nodes_a: vec4<u32>,
    nodes_b: vec4<u32>,
    weights_a: vec4<f32>,
    weights_b: vec4<f32>,
    gradient_x_a: vec4<f32>,
    gradient_x_b: vec4<f32>,
    gradient_y_a: vec4<f32>,
    gradient_y_b: vec4<f32>,
    position_normal: vec4<f32>,
}

struct FarFieldControl {
    // Sample period, ring frames, contour points, directions.
    sampling: vec4<f32>,
    // Exterior speed, contour spacing, delay margin, unused.
    projection: vec4<f32>,
}

// What an unwritten frame holds. A recorded time is never negative, so this is
// a comparison against a real number rather than a NaN test, which the fast
// math these shaders compile under is free to fold away.
const UNRECORDED: f32 = -1.0e30;

struct ProbeSample {
    values: vec4<f32>,
}

@group(0) @binding(0) var<storage, read> parameters: Parameters;
@group(0) @binding(1) var<storage, read> states: array<State>;
@group(0) @binding(2) var<storage, read> stencils: array<FarFieldStencil>;
@group(0) @binding(3) var<storage, read> control: FarFieldControl;
@group(0) @binding(4) var<storage, read_write> contour_history: array<ProbeSample>;
@group(0) @binding(5) var<storage, read_write> directional_history: array<ProbeSample>;

fn weighted(values_a: vec4<f32>, values_b: vec4<f32>, stencil: FarFieldStencil) -> f32 {
    return dot(values_a, stencil.weights_a) + dot(values_b, stencil.weights_b);
}

fn ring_frame(frame: u32, age: u32, frames: u32) -> u32 {
    return (frame + frames - (age % frames)) % frames;
}

// Time of the state the solver has just produced. The transfer carries this
// clock into the buffers that replace it, so it spans the mesh swaps the ring
// spans and needs no offset of its own.
fn state_time() -> f32 {
    return parameters.time_data.z - parameters.time_data.x;
}

// A frame is one bucket of that clock. The first step to reach a bucket records
// it; the rest of the steps in it do nothing.
fn bucket_of(time: f32) -> f32 {
    return floor(time / control.sampling.x);
}

fn bucket_frame(bucket: f32, frames: u32) -> u32 {
    let count = f32(frames);
    return u32(bucket - floor(bucket / count) * count);
}

// A frame already holding a sample of this bucket has been recorded. Asking the
// ring, rather than the step count, is what makes the cadence the dispatcher
// picks irrelevant: it only has to be dense enough to visit every bucket, and
// the clock it estimates need not be the clock recorded here.
fn bucket_recorded(recorded: f32, bucket: f32) -> bool {
    return recorded > UNRECORDED * 0.5 && bucket_of(recorded) == bucket;
}

@compute @workgroup_size(64)
fn sample_contour(@builtin(global_invocation_id) invocation: vec3<u32>) {
    let point = invocation.x;
    let point_count = u32(control.sampling.z);
    if point >= point_count {
        return;
    }
    let time = state_time();
    let frames = u32(control.sampling.y);
    let bucket = bucket_of(time);
    let index = bucket_frame(bucket, frames) * point_count + point;
    if bucket_recorded(contour_history[index].values.w, bucket) {
        return;
    }
    let stencil = stencils[point];
    let a = stencil.nodes_a;
    let b = stencil.nodes_b;
    let displacement_a = vec4<f32>(states[a.x].auxiliary.w, states[a.y].auxiliary.w, states[a.z].auxiliary.w, states[a.w].auxiliary.w);
    let displacement_b = vec4<f32>(states[b.x].auxiliary.w, states[b.y].auxiliary.w, states[b.z].auxiliary.w, 0.0);
    let velocity_a = vec4<f32>(states[a.x].auxiliary.z, states[a.y].auxiliary.z, states[a.z].auxiliary.z, states[a.w].auxiliary.z);
    let velocity_b = vec4<f32>(states[b.x].auxiliary.z, states[b.y].auxiliary.z, states[b.z].auxiliary.z, 0.0);
    let displacement = weighted(displacement_a, displacement_b, stencil);
    let velocity = weighted(velocity_a, velocity_b, stencil);
    let gradient_x = dot(displacement_a, stencil.gradient_x_a) + dot(displacement_b, stencil.gradient_x_b);
    let gradient_y = dot(displacement_a, stencil.gradient_y_a) + dot(displacement_b, stencil.gradient_y_b);
    let normal_gradient = gradient_x * stencil.position_normal.z + gradient_y * stencil.position_normal.w;
    contour_history[index].values = vec4<f32>(displacement, velocity, normal_gradient, time);
}

@compute @workgroup_size(64)
fn project_directions(@builtin(global_invocation_id) invocation: vec3<u32>) {
    let direction = invocation.x;
    let direction_count = u32(control.sampling.w);
    if direction >= direction_count {
        return;
    }
    let now = state_time();
    let frames = u32(control.sampling.y);
    let point_count = u32(control.sampling.z);
    let period = control.sampling.x;
    let bucket = bucket_of(now);
    let current_frame = bucket_frame(bucket, frames);
    let wave_speed = control.projection.x;
    let spacing = control.projection.y;
    let margin = control.projection.z;
    let far_time = now - margin;
    let slot = current_frame * direction_count + direction;
    // Recorded emission times run one delay margin behind the clock.
    if bucket_recorded(directional_history[slot].values.z + margin, bucket) {
        return;
    }
    let angle = 6.283185307179586 * f32(direction) / f32(direction_count);
    let ray = vec2<f32>(cos(angle), sin(angle));
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
        // The buckets are uniform, but the samples inside them are not: a step
        // lands wherever it lands, and a handoff can change the step. So the
        // estimate below only starts the search, and the recorded times decide.
        var index = u32(floor(age));
        var newer = contour_history[ring_frame(current_frame, index, frames) * point_count + point].values;
        var older = contour_history[ring_frame(current_frame, index + 1u, frames) * point_count + point].values;
        for (var step = 0u; step < 4u; step += 1u) {
            if newer.w < 0.0 || older.w < 0.0 {
                break;
            }
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
        // A frame the search could not bracket is either unwritten or a lap-old
        // leftover, both of which mean the ring does not reach back this far.
        if newer.w < 0.0 || older.w < 0.0
            || retarded > newer.w + period || retarded < older.w - period {
            valid = false;
            continue;
        }
        let span = newer.w - older.w;
        var fraction = 0.0;
        if span > 0.0 {
            fraction = clamp((newer.w - retarded) / span, 0.0, 1.0);
        }
        let sample = mix(newer.xyz, older.xyz, fraction);
        amplitude += spacing * (sample.z - dot(normal, ray) * sample.y / wave_speed);
    }
    if valid {
        directional_history[slot].values = vec4<f32>(amplitude, amplitude * amplitude, far_time, 1.0);
    } else {
        // The last component is what the readback reads as "do not plot this".
        directional_history[slot].values = vec4<f32>(0.0, 0.0, far_time, 0.0);
    }
}
