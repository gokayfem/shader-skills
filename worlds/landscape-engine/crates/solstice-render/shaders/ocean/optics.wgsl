// How water looks, whatever its surface is doing: what it reflects, and what is seen through it.
// For the passes that shade water over the lit scene, which bind that scene as `lit`, its depth
// as `opaque_depth`, and what `gi/screen_trace.wgsl` reads.

// Water's reflectance looking straight down: from its index of refraction, 1.33.
const WATER_F0: f32 = 0.02;
// How far reflections are traced across the screen, and how thick a surface they can hit.
const REFLECTION_DISTANCE: f32 = 300.0;
const REFLECTION_STEPS: u32 = 64u;
const REFLECTION_THICKNESS: f32 = 0.5;

// What the water at `world`, facing `normal`, reflects along `direction`: the scene where the
// screen shows it, then the scene's voxels, then the sky. As the reflections pass traces, but
// a hit's colour is this frame's, as the water is shaded after the scene is lit.
fn reflection(world: vec3f, normal: vec3f, direction: vec3f) -> vec3f {
    let sky = sky_radiance(toward_horizon(direction));
    // A ray reflected down into the water reflects nothing but the sky above the waves.
    if direction.y <= 0.0 {
        return sky;
    }
    let origin = world + normal * 0.05;
    let screen = trace_screen(origin, direction, REFLECTION_DISTANCE, REFLECTION_STEPS, REFLECTION_THICKNESS);
    var seen = vec3f(0.0);
    if screen.hit {
        seen = textureSampleLevel(lit, clamp_sampler, screen.uv, 0.0).rgb;
    }
    if screen.weight >= 1.0 {
        return seen;
    }
    // Off the screen: the volume of voxels around the camera, then the sky beyond it. Water
    // beyond the volume's reach sees only the sky, and need not trace it.
    let reach = volume_edge_fade(world);
    if reach <= 0.0 {
        return mix(sky, seen, screen.weight);
    }
    let level = min(volume_level_at(world, 0u), VOLUME_LEVELS - 1u);
    let voxel = volume_params.levels[level].w;
    let hit = trace_volume_with(origin + normal * voxel, direction, voxel * 0.5, 320.0, 48u, 0.55);
    let hit_point = origin + direction * hit.t;
    let far = mix(sky, select(sky, hit.radiance, hit.hit), volume_edge_fade(hit_point) * reach);
    return mix(far, seen, screen.weight);
}

// What is seen through the water at `world`: the lit scene behind it, bent by refraction where
// the view ray enters, and dimmed and tinted by the water the light crosses, down to the bed and
// back up to the eye, with the water's own glow, `body`, filling in as the path deepens. Over
// sand a few centimetres down the water is almost clear; a metre turns it turquoise, red going
// first; deeper, and over the open sea, it is the body's colour alone.
fn through_water(
    center: vec2f,
    world: vec3f,
    view: vec3f,
    normal: vec3f,
    surface_depth: f32,
    scene_depth: f32,
    body: vec3f,
    extinction: vec3f,
) -> vec3f {
    // Nothing behind this pixel but the sky: open sea, as deep as can be.
    if scene_depth <= 0.0 {
        return body;
    }
    let straight = reconstruct_world(frame, center, scene_depth);
    // Follow the bent ray down as deep as the bed straight behind lies, and find what it
    // reaches on screen. At grazing angles the view ray runs a long way through shallow water;
    // the bent one, steeper, does not.
    let bent = refract(-view, normal, 1.0 / 1.33);
    let deep = max(world.y - straight.y, 0.0);
    var pixel = center;
    let reached = frame.view_proj * vec4f(world + bent * (deep / max(-bent.y, 0.1)), 1.0);
    if reached.w > 0.0 {
        let ndc = reached.xy / reached.w;
        let shift = vec2f(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5) * frame.viewport - center;
        // Off the screen there is nothing to see, and clamping would smear its edge into
        // streaks: near the edge, the shift takes at most half the room left, so pixels
        // nearer the edge still land nearer it, and none land on it.
        let room = select(center, frame.viewport - center, shift > vec2f(0.0)) - vec2f(0.5);
        let fits = clamp(0.5 * max(room, vec2f(0.0)) / max(abs(shift), vec2f(1.0e-3)), vec2f(0.0), vec2f(1.0));
        pixel = center + shift * min(fits.x, fits.y);
    }
    let last = vec2i(frame.viewport) - vec2i(1);
    var bed_depth = textureLoad(opaque_depth, clamp(vec2i(pixel), vec2i(0), last), 0).r;
    // What is found there must lie beyond the water's surface. Anything nearer is in front of
    // the water, and the sky is beyond the scene; either way, look straight through instead.
    if bed_depth <= 0.0 || bed_depth >= surface_depth {
        pixel = center;
        bed_depth = scene_depth;
    }
    let bed = reconstruct_world(frame, pixel, bed_depth);
    let path = distance(bed, world);
    // The light reaching the bed came down through the water first, slanting as the sun's
    // light bends into it.
    let sun_in = refract(-frame.sun_direction, vec3f(0.0, 1.0, 0.0), 1.0 / 1.33);
    let down = max(world.y - bed.y, 0.0) / max(-sun_in.y, 0.2);
    let seen = textureSampleLevel(lit, clamp_sampler, pixel * frame.inv_viewport, 0.0).rgb;
    return seen * exp(-extinction * (path + down)) + body * (vec3f(1.0) - exp(-extinction * path));
}
