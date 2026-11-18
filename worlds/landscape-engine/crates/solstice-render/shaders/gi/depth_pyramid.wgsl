// Builds the closest-depth pyramid that screen traces march through. Each texel holds the
// *nearest* (largest, in reversed Z) depth of the pixels it covers, so a ray in front of a
// texel's value is in front of everything under it and can skip the whole tile.
//
// Mip 0 is the full-resolution depth, padded with zero depth (nothing) to a whole number of
// the coarsest tiles. Texel i of mip n covers pixels [i * 2^n, (i + 1) * 2^n), so a pixel's
// texel at any mip is just `pixel >> n`.

@group(0) @binding(0) var source: texture_2d<f32>;
@group(0) @binding(1) var output: texture_storage_2d<r32float, write>;

@compute @workgroup_size(8, 8)
fn copy(@builtin(global_invocation_id) id: vec3u) {
    if any(id.xy >= textureDimensions(output)) {
        return;
    }
    var depth = 0.0;
    if all(id.xy < textureDimensions(source)) {
        depth = textureLoad(source, id.xy, 0).r;
    }
    textureStore(output, id.xy, vec4f(depth, 0.0, 0.0, 0.0));
}

@compute @workgroup_size(8, 8)
fn reduce(@builtin(global_invocation_id) id: vec3u) {
    if any(id.xy >= textureDimensions(output)) {
        return;
    }
    // The padding makes every mip exactly half the one above.
    let base = vec2i(id.xy) * 2;
    var nearest = textureLoad(source, base, 0).r;
    nearest = max(nearest, textureLoad(source, base + vec2i(1, 0), 0).r);
    nearest = max(nearest, textureLoad(source, base + vec2i(0, 1), 0).r);
    nearest = max(nearest, textureLoad(source, base + vec2i(1, 1), 0).r);
    textureStore(output, id.xy, vec4f(nearest, 0.0, 0.0, 0.0));
}
