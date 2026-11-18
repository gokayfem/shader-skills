// The water-line map (ocean/waterline.rs): for each spot near the camera, the highest the water has
// stood there lately and where it stands now, as heights in the world. Twin of
// `WaterlineUniform` in crates/solstice-render/src/ocean/waterline.rs.
//
// The map is fixed in the world: a spot keeps its texel as the camera moves, and texels wrap
// around the map's edges. Its window, the square of texels it covers this frame, follows the
// camera.

struct WaterlineParams {
    // The window's first texel, this frame and last, in texels of the world grid.
    window: vec4f,
    // Metres per texel; texels across; how much of the remembered height above the water's
    // present one is left after this frame's drying; 1 if last frame's map can be kept.
    grid: vec4f,
}

// The texel of the world grid holding `xz`.
fn waterline_world_texel(xz: vec2f) -> vec2f {
    return floor(xz / waterline.grid.x);
}

// Where the world grid's texel `texel` is stored in the map.
fn waterline_index(texel: vec2f) -> vec2i {
    let n = waterline.grid.y;
    return vec2i(texel - floor(texel / n) * n);
}

// Whether the world grid's texel `texel` lies in the window starting at `first`.
fn in_waterline_window(texel: vec2f, first: vec2f) -> bool {
    return all(texel >= first) && all(texel < first + vec2f(waterline.grid.y));
}
