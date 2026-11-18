// What the clouds are doing this frame. Twin of `CloudUniform` in clouds/mod.rs.

struct CloudParams {
    // How far the weather has drifted with the wind, in metres along X and Z.
    wind_offset: vec2f,
    // How far cloud shapes have boiled upwards as they evolve, in metres of noise.
    evolution: f32,
    coverage: f32,
    // 0 for flat stratus, 0.5 for stratocumulus, 1 for heaped cumulus.
    cloud_type: f32,
    // Bottom and top of the cloud layer above the ground, in metres.
    base: f32,
    top: f32,
    density: f32,
    cirrus: f32,
    rain: f32,
    // Ground covered by the cloud shadow map, in metres along each side, and its corner.
    shadow_extent: f32,
    // 0 when clouds are switched off.
    enabled: f32,
    shadow_origin: vec2f,
    // Unit direction the wind blows towards, in X and Z.
    wind_direction: vec2f,
    // A flash of lightning this frame (lightning.rs): towards it from the camera, and the
    // illuminance it gives the scene, in the sun's units; 0 for none.
    flash: vec4f,
}
