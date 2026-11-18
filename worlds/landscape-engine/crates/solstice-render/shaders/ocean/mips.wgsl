// One level of the waves' mip chains from the level above: each texel the mean of the four it
// covers, for every cascade. Tiles repeat, and their sizes are powers of two, so every texel has
// exactly four beneath it.

@group(0) @binding(0) var displacement_above: texture_2d_array<f32>;
@group(0) @binding(1) var derivatives_above: texture_2d_array<f32>;
@group(0) @binding(2) var displacement: texture_storage_2d_array<rgba16float, write>;
@group(0) @binding(3) var derivatives: texture_storage_2d_array<rgba16float, write>;

fn mean_of_four(source: texture_2d_array<f32>, texel: vec2u, cascade: u32) -> vec4f {
    let corner = texel * 2u;
    return (textureLoad(source, corner, cascade, 0)
        + textureLoad(source, corner + vec2u(1u, 0u), cascade, 0)
        + textureLoad(source, corner + vec2u(0u, 1u), cascade, 0)
        + textureLoad(source, corner + vec2u(1u, 1u), cascade, 0)) * 0.25;
}

@compute @workgroup_size(8, 8, 1)
fn main(@builtin(global_invocation_id) id: vec3u) {
    if any(id.xy >= textureDimensions(displacement)) {
        return;
    }
    textureStore(displacement, id.xy, id.z, mean_of_four(displacement_above, id.xy, id.z));
    textureStore(derivatives, id.xy, id.z, mean_of_four(derivatives_above, id.xy, id.z));
}
