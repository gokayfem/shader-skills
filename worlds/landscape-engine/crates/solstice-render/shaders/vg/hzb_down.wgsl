// Every level of the depth pyramid after the first: the farthest of the 2x2 texels above.

@group(0) @binding(0) var source: texture_2d<f32>;
@group(0) @binding(1) var output: texture_storage_2d<r32float, write>;

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) id: vec3u) {
    let output_size = textureDimensions(output);
    if id.x >= output_size.x || id.y >= output_size.y {
        return;
    }
    let last = vec2i(textureDimensions(source)) - vec2i(1);
    let base = vec2i(id.xy) * 2;
    var farthest = textureLoad(source, min(base, last), 0).r;
    farthest = min(farthest, textureLoad(source, min(base + vec2i(1, 0), last), 0).r);
    farthest = min(farthest, textureLoad(source, min(base + vec2i(0, 1), last), 0).r);
    farthest = min(farthest, textureLoad(source, min(base + vec2i(1, 1), last), 0).r);
    textureStore(output, id.xy, vec4f(farthest, 0.0, 0.0, 0.0));
}
