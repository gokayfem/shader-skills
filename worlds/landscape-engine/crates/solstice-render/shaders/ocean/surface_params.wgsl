// What the passes that draw the sea's surface need to know of it. Twin of `SurfaceUniform` in
// crates/solstice-render/src/ocean/surface.rs.
struct SurfaceParams {
    // Width of each cascade's tile in metres; the calm sea's level.
    sizes: vec4f,
    // Texels across each tile; choppiness; the finest patch's width; each level's range, in
    // its patches' widths.
    grid: vec4f,
    // Where blending to the coarser grid starts, as a share of a level's range.
    morph: vec4f,
    // How strongly the water absorbs and scatters red, green and blue light, per metre.
    absorption: vec4f,
    scattering: vec4f,
    // Where the table of unresolved slopes starts and steps, in log10 of wavenumber; the
    // shortest wave the cascades draw; and the smoothest the water ever is, as GGX alpha.
    slope_range: vec4f,
    // The mean square slope of all waves shorter than each wavenumber of the table.
    slopes: array<vec4f, 8>,
    // The seabed map (ocean/seabed.rs): projects the world onto it, with depth 0 at its top; and
    // the height of its top, and how far down it reaches.
    seabed_view_proj: mat4x4f,
    seabed: vec4f,
    // The wind near the water, packed (common/wind.wgsl, `unpack_wind`): its gusts roughen the
    // water as they cross it.
    wind: array<vec4f, 2>,
}
