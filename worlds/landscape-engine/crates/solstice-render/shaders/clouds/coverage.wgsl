// Measures how the weather map's coverage values are spread, once, after the map is made. The
// cloud passes look values up in the result, which spreads them evenly over 0..1: then a
// coverage of 0.6 really does let cloud form over 60% of the sky, whatever the noise's own
// spread. Without this, noise values bunch in the middle and a small change in coverage turns a
// few clouds into an overcast sky.

@group(0) @binding(0) var weather_map: texture_2d<f32>;
@group(0) @binding(1) var<storage, read_write> bins: array<atomic<u32>, 256>;
@group(0) @binding(2) var<storage, read_write> cumulative: array<f32, 256>;

@compute @workgroup_size(8, 8)
fn histogram(@builtin(global_invocation_id) id: vec3u) {
    let size = textureDimensions(weather_map);
    if any(id.xy >= size) {
        return;
    }
    let value = textureLoad(weather_map, id.xy, 0).r;
    atomicAdd(&bins[min(u32(value * 255.0 + 0.5), 255u)], 1u);
}

// The share of the map at or below each value.
@compute @workgroup_size(1)
fn accumulate() {
    let size = textureDimensions(weather_map);
    let total = f32(size.x * size.y);
    var running = 0u;
    for (var i = 0u; i < 256u; i++) {
        running += atomicLoad(&bins[i]);
        cumulative[i] = f32(running) / total;
    }
}
