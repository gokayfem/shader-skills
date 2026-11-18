// The seabed under the sea, from the map drawn looking straight down at it (ocean/seabed.rs), and
// how the waves shrink as the water over it shoals.
//
// The including shader declares, in group 0:
//   surface: SurfaceParams           with the map's projection and heights
//   seabed_map: texture_depth_2d     the map

// Waves break once they stand taller than about 0.78 of the water's depth (McCowan 1894), and a
// crest rises about half a wave's height above the calm level, so crests and troughs stand at
// most this share of the depth from it.
const CREST_PER_DEPTH: f32 = 0.4;
// Depth of water below which the waves' sideways sway dies away too, in metres.
const SWAY_DEPTH: f32 = 2.0;

// Height of the highest opaque surface under `xz`, at or below the map's top; as deep as the map
// reaches where it holds nothing there, or does not reach.
fn seabed_height(xz: vec2f) -> f32 {
    let top = surface.seabed.x;
    let deepest = top - surface.seabed.y;
    let clip = surface.seabed_view_proj * vec4f(xz.x, top, xz.y, 1.0);
    let texels = vec2f(textureDimensions(seabed_map));
    let at = (clip.xy * vec2f(0.5, -0.5) + vec2f(0.5)) * texels - vec2f(0.5);
    if any(at < vec2f(0.0)) || any(at > texels - vec2f(1.0)) {
        return deepest;
    }
    // Bilinear, by hand: depth textures are not filtered.
    let base = vec2i(floor(at));
    let last = vec2i(texels) - vec2i(1);
    let next = min(base + vec2i(1), last);
    let f = at - floor(at);
    let depth = mix(
        mix(textureLoad(seabed_map, base, 0), textureLoad(seabed_map, vec2i(next.x, base.y), 0), f.x),
        mix(textureLoad(seabed_map, vec2i(base.x, next.y), 0), textureLoad(seabed_map, next, 0), f.x),
        f.y,
    );
    return top - depth * surface.seabed.y;
}

// How deep the calm sea is over `xz`: negative over land.
fn water_depth_at(xz: vec2f) -> f32 {
    return surface.sizes.w - seabed_height(xz);
}

// The most a crest can rise, or a trough sink, in water `depth` metres deep.
fn wave_limit(depth: f32) -> f32 {
    return max(CREST_PER_DEPTH * depth, 1.0e-3);
}

// The waves' displacement `offset` in water `depth` metres deep: their height squeezed smoothly
// within what the depth can hold, and their sway fading in the shallows. In deep water nothing
// changes; over land the water lies flat and still, below the ground.
fn shoal(offset: vec3f, depth: f32) -> vec3f {
    let limit = wave_limit(depth);
    let sway = clamp(depth / SWAY_DEPTH, 0.0, 1.0);
    // tanh is clamped first: some implementations overflow on large arguments.
    return vec3f(offset.x * sway, limit * tanh(clamp(offset.y / limit, -10.0, 10.0)), offset.z * sway);
}

// How `shoal` scales the waves' vertical slopes, where they stand `height` above the calm sea in
// water `depth` deep: the derivative of its squeeze.
fn shoal_slope(height: f32, depth: f32) -> f32 {
    let t = tanh(clamp(height / wave_limit(depth), -10.0, 10.0));
    return 1.0 - t * t;
}
