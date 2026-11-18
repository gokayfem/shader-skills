// Hierarchical screen traces, as in Lumen: a ray marches through the closest-depth pyramid
// (gi/depth_pyramid.wgsl) in screen space, where reversed-Z depth varies linearly along it.
// Where the ray passes in front of a whole tile it skips the tile and climbs to a coarser
// mip; where it meets the tile's nearest surface it descends. A ray reaching a surface at
// mip 0 has hit. The traversal follows AMD FidelityFX SSSR.
//
// Long rays cost a few dozen texel reads instead of hundreds, and never step over thin
// geometry the way a fixed-step march does.
//
// The including shader declares, in group 0:
//   depth_pyramid: texture_2d<f32>   the closest-depth pyramid
//   normal_tex: texture_2d<f32>      G-buffer normals, to reject back faces
// gi/screen_history.wgsl finds a hit's colour in last frame's image.

const SCREEN_EDGE_FADE: f32 = 0.08;

struct ScreenHit {
    hit: bool,
    // Distance along the world-space ray.
    t: f32,
    uv: vec2f,
    // Fades to zero towards the screen edges, where the screen's data runs out.
    weight: f32,
}

fn screen_miss() -> ScreenHit {
    return ScreenHit(false, 0.0, vec2f(0.0), 0.0);
}

// To uv and reversed-Z depth, without jitter.
fn project_to_screen(world: vec3f) -> vec3f {
    let clip = frame.unjittered_view_proj * vec4f(world, 1.0);
    let ndc = clip.xyz / clip.w;
    return vec3f(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5, ndc.z);
}

fn view_distance(world: vec3f) -> f32 {
    return (frame.unjittered_view_proj * vec4f(world, 1.0)).w;
}

// Moves the ray to the edge of its texel at the given mip resolution, or to the tile's depth
// plane if that comes first. Returns whether the ray passed in front of the whole tile.
fn advance_ray(
    origin: vec3f,
    inv_direction: vec3f,
    direction: vec3f,
    mip_position: vec2f,
    inv_resolution: vec2f,
    floor_offset: vec2f,
    uv_offset: vec2f,
    surface: f32,
    position: ptr<function, vec3f>,
    t: ptr<function, f32>,
) -> bool {
    let planes = vec3f((floor(mip_position) + floor_offset) * inv_resolution + uv_offset, surface);
    var crossing = (planes - origin) * inv_direction;
    // Reversed Z: only a ray moving away from the camera can meet the depth plane from in front.
    crossing.z = select(3.0e38, crossing.z, direction.z < 0.0);
    let t_min = min(min(crossing.x, crossing.y), crossing.z);
    let in_front = surface < (*position).z;
    if in_front {
        *t = t_min;
        *position = origin + direction * t_min;
    }
    return in_front && t_min != crossing.z;
}

// Traces from `origin` along unit `direction` for up to `max_distance` metres. `thickness`
// is how far, in metres, a surface is assumed to extend behind what the camera sees.
fn trace_screen(origin: vec3f, direction: vec3f, max_distance: f32, max_steps: u32, thickness: f32) -> ScreenHit {
    // Stop short of the near plane, where projection breaks down.
    var distance = max_distance;
    let w0 = view_distance(origin);
    let dw = view_distance(origin + direction) - w0;
    if dw < 0.0 {
        distance = min(distance, (w0 - frame.near * 2.0) / -dw);
    }
    if distance <= 0.0 {
        return screen_miss();
    }
    let start = project_to_screen(origin);
    let end = project_to_screen(origin + direction * distance);
    let ray = end - start;
    if dot(ray.xy, ray.xy) < 1.0e-12 {
        return screen_miss();
    }
    let inv_ray = select(vec3f(3.0e38), 1.0 / ray, abs(ray) > vec3f(1.0e-12));

    let top_mip = i32(textureNumLevels(depth_pyramid)) - 1;
    let pixel_offset = 0.005 * frame.inv_viewport;
    let uv_offset = select(pixel_offset, -pixel_offset, ray.xy < vec2f(0.0));
    let floor_offset = select(vec2f(1.0), vec2f(0.0), ray.xy < vec2f(0.0));

    // Step off the starting pixel first, so the ray does not hit the surface it leaves.
    var position = start;
    var t = 0.0;
    {
        let planes = (floor(start.xy * frame.viewport) + floor_offset) * frame.inv_viewport + uv_offset;
        let crossing = (planes - start.xy) * inv_ray.xy;
        t = min(crossing.x, crossing.y);
        position = start + ray * t;
    }

    var mip = 0;
    var steps = 0u;
    while mip >= 0 && steps < max_steps {
        if t > 1.0 || any(position.xy < vec2f(0.0)) || any(position.xy > vec2f(1.0)) {
            return screen_miss();
        }
        let resolution = frame.viewport * exp2(-f32(mip));
        let mip_position = position.xy * resolution;
        let texel = min(vec2i(mip_position), vec2i(textureDimensions(depth_pyramid, mip)) - vec2i(1));
        let surface = textureLoad(depth_pyramid, texel, mip).r;
        let skipped = advance_ray(start, inv_ray, ray, mip_position, 1.0 / resolution, floor_offset, uv_offset, surface, &position, &t);
        mip = select(mip - 1, min(mip + 1, top_mip), skipped);
        steps++;
    }
    if mip >= 0 || t > 1.0 {
        return screen_miss();
    }

    // The ray is level with or behind the surface here. It hits only if that surface is
    // plausibly this thick, and only its front: a ray leaving a surface seen at a grazing
    // angle would otherwise "hit" the same surface, since a pixel there spans metres of depth.
    let pixel = min(vec2i(position.xy * frame.viewport), vec2i(frame.viewport) - vec2i(1));
    let scene_depth = textureLoad(depth_pyramid, pixel, 0).r;
    if scene_depth <= 0.0 || position.z <= 0.0 {
        return screen_miss();
    }
    let scene_distance = frame.near / scene_depth;
    let ray_distance = frame.near / position.z;
    if ray_distance - scene_distance > thickness + scene_distance * 0.02 {
        return screen_miss();
    }
    let surface_normal = oct_decode(textureLoad(normal_tex, pixel, 0).xy);
    if dot(surface_normal, direction) >= 0.0 {
        return screen_miss();
    }
    let edge = min(min(position.x, 1.0 - position.x), min(position.y, 1.0 - position.y));
    return ScreenHit(true, t * distance, position.xy, smoothstep(0.0, SCREEN_EDGE_FADE, edge));
}

