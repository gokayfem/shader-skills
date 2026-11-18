// First level of the hierarchical depth pyramid used for occlusion culling. Each
// texel holds the *farthest* (smallest, in reversed Z) depth of the area it covers.
// The pyramid is a power of two and the screen need not be, so a texel can straddle
// source pixels. Every touched pixel counts, to stay conservative.

struct Params {
    source_size: vec2u,
    target_size: vec2u,
}

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> vis_depth: array<u32>;
@group(0) @binding(2) var output: texture_storage_2d<r32float, write>;

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) id: vec3u) {
    if id.x >= params.target_size.x || id.y >= params.target_size.y {
        return;
    }
    let source = params.source_size;
    let begin = id.xy * source / params.target_size;
    let end = min(
        ((id.xy + vec2u(1u)) * source + params.target_size - vec2u(1u)) / params.target_size,
        source,
    );
    var farthest = 1.0;
    for (var y = begin.y; y < end.y; y++) {
        for (var x = begin.x; x < end.x; x++) {
            farthest = min(farthest, bitcast<f32>(vis_depth[y * source.x + x]));
        }
    }
    textureStore(output, id.xy, vec4f(farthest, 0.0, 0.0, 0.0));
}
