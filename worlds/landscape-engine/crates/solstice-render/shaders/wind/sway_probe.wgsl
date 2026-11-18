// Runs the bending of common/sway_vertex.wgsl on points given by a test, so the test can check
// what a blade does in the wind without drawing one. Used only by wind.rs's tests.

#include "common/scene.wgsl"

@group(0) @binding(0) var wind_field: texture_2d<f32>;
@group(0) @binding(1) var<storage, read> instances: array<Instance>;

#include "common/sway_vertex.wgsl"

struct Probe {
    // A point of the instance's mesh, and where it is seen from.
    local: vec3f,
    _a: f32,
    eye: vec3f,
    _b: f32,
}

struct ProbeResult {
    // Where the wind carries the point now and carried it a frame ago, from where it stands at
    // rest, as resolve works them out; then where the rasterizers place it, the same way.
    now: vec4f,
    before: vec4f,
    placed: vec4f,
    // Up, turned as the blade is turned there.
    up: vec4f,
}

@group(0) @binding(2) var<storage, read> probes: array<Probe>;
@group(0) @binding(3) var<storage, read_write> results: array<ProbeResult>;

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id: vec3u) {
    if id.x >= arrayLength(&probes) {
        return;
    }
    let probe = probes[id.x];
    let instance = instances[0];
    let world = transform_point(instance, probe.local);
    let w = wind_params();
    let s = vertex_sway(w, instance, probe.local, world, probe.eye);
    results[id.x] = ProbeResult(
        vec4f(s.now, sway_height(instance, probe.local)),
        vec4f(s.before, 0.0),
        vec4f(swayed_position(instance, probe.local, world, probe.eye) - world, 0.0),
        vec4f(bend_direction(s.bend, vec3f(0.0, 1.0, 0.0)), 0.0),
    );
}
