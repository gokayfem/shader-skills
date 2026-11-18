// Draws the rain: falling drops as streaks, the crowns of droplets they throw up where they
// strike, and drops falling from eaves (docs/weather-effects.md).
//
// Nothing is simulated. Every drop's place follows from a hash and the clock: the air carries
// a lattice of drops downwind with the mean wind, and each falls through it at its terminal
// speed, round and round a column of air around the camera. So rain costs only its drawing,
// and the same drop is in the same place however the camera moves. Three rings of such drops,
// coarser and fainter outward, fill the air out past a hundred metres. Drops end where the rain
// map says the first surface is, so none falls under a roof.
//
// A drop moving through one exposure is seen as a streak (Garg and Nayar 2006): it covers each
// pixel along its path for only the time it takes to cross it, so a streak is faint, and
// brightest against what is dark behind it. It shows the light around it, most of all the sun
// scattered onward through it, from the rain's light volume (rain/light.wgsl).

#include "common/math.wgsl"
#include "common/frame.wgsl"
#include "rain/params.wgsl"

@group(0) @binding(0) var<uniform> rain: RainParams;
@group(0) @binding(1) var<uniform> frame: Frame;
@group(0) @binding(2) var rain_map: texture_2d<f32>;
@group(0) @binding(3) var scene_depth: texture_2d<f32>;
@group(0) @binding(4) var light_ambient: texture_3d<f32>;
@group(0) @binding(5) var light_sun: texture_3d<f32>;
@group(0) @binding(6) var light_sampler: sampler;

// What an instance draws: its kind in the top bits of its index. Twins of the constants in
// rain/mod.rs.
const KIND_SHIFT: u32 = 20u;
const KIND_SPLASH: u32 = 3u;
const KIND_DRIP: u32 = 4u;

// The rings of falling drops: spacing of their lattice in metres, cells along each side, the
// height of the column of air each fills, how much wider and fainter their streaks are drawn.
const RING_SPACING: array<f32, 3> = array<f32, 3>(0.3, 1.0, 3.0);
const RING_CELLS: array<u32, 3> = array<u32, 3>(80u, 80u, 80u);
const RING_HEIGHT: array<f32, 3> = array<f32, 3>(24.0, 60.0, 140.0);
const RING_WIDTH: array<f32, 3> = array<f32, 3>(1.0, 2.5, 6.0);
// An outer ring's lattice is coarser, so each of its streaks stands for more drops: drawn this
// much stronger, the rain keeps its density on screen into the distance instead of thinning
// to nothing past the first ring.
const RING_STRENGTH: array<f32, 3> = array<f32, 3>(1.0, 2.4, 4.5);
// Far off, a streak's path over the exposure is a pixel or two, and a field of dots reads as
// snow. Each far streak stands for several drops one above another, so it is drawn a little
// longer; not much, or far rain would seem to fall faster than near rain.
const RING_STRETCH: array<f32, 3> = array<f32, 3>(1.0, 1.3, 1.8);
// Splashes and drips: their cells' spacing and how many along each side.
const SPLASH_SPACING: f32 = 0.5;
const SPLASH_CELLS: u32 = 28u;
const SPLASH_DROPLETS: u32 = 6u;
const DRIP_SPACING: f32 = 0.25;
const DRIP_CELLS: u32 = 64u;

const GRAVITY: f32 = 9.81;
// A streak is never drawn thinner than this, in pixels; a thinner drop is drawn fainter instead.
const LEAST_WIDTH: f32 = 1.2;
// One drop covers a pixel of its path for only a sliver of the exposure, and is far thinner than
// a pixel; drawn so, a streak would vanish. There are a hundred times fewer streaks than drops,
// so each stands for many: it is drawn about as opaque as this, near, thinning with distance.
const STREAK_OPACITY: f32 = 0.13;
// How much of the fading for being thinner than a pixel is kept: some, so far rain is finer
// and fainter, not all, or it would vanish.
const THINNESS: f32 = 0.35;
// How a drop scatters: onward, strongly, and evenly in all directions.
const FORWARD: f32 = 0.75;
const SUN_SCATTER: f32 = 0.6;
// How close to the camera a drop can come, in metres, before it fades out.
const NEAREST: f32 = 0.6;
// Clear air dims what is far by this per metre; rain thickens it.
const HAZE: f32 = 0.0011;

struct Varyings {
    @builtin(position) position: vec4f,
    // Across the streak, -1 to 1, and along it, 0 at its tail to 1 at its head.
    @location(0) at: vec2f,
    // Premultiplied radiance and opacity.
    @location(1) color: vec4f,
}

// The rain map's texel holding `xz`: the first surface's height, the puddle's depth, the eave's
// height, and 1 if anything is there; or nothing, outside it.
fn rain_map_at(xz: vec2f) -> vec4f {
    if !in_rain_map(rain, xz) {
        return vec4f(NOTHING_BELOW, 0.0, 0.0, 0.0);
    }
    return textureLoad(rain_map, vec2u(rain_map_texel(rain, xz)), 0);
}

// The light a drop at `p` sends towards the camera.
fn drop_light(p: vec3f) -> vec3f {
    let uvw = (p - rain.light.xyz) / (rain.light.w * vec3f(textureDimensions(light_ambient)));
    let ambient = textureSampleLevel(light_ambient, light_sampler, uvw, 0.0).rgb;
    let sun = textureSampleLevel(light_sun, light_sampler, uvw, 0.0).rgb;
    let toward = normalize(p - frame.camera_position);
    let cos_theta = dot(frame.sun_direction, toward);
    let g = FORWARD;
    let phase = (1.0 - g * g) / (4.0 * PI * pow(1.0 + g * g - 2.0 * g * cos_theta, 1.5));
    return ambient + sun * phase * SUN_SCATTER;
}

// A streak from `tail` to `head`, `width` metres wide, `opacity` at most, for one of its six
// corners. Off-screen and degenerate streaks collapse to nothing.
fn streak(corner: u32, tail: vec3f, head: vec3f, width: f32, opacity: f32, light: vec3f) -> Varyings {
    var out: Varyings;
    out.position = vec4f(0.0);
    out.at = vec2f(0.0);
    out.color = vec4f(0.0);
    let head_clip = frame.view_proj * vec4f(head, 1.0);
    let tail_clip = frame.view_proj * vec4f(tail, 1.0);
    if head_clip.w < frame.near || tail_clip.w < frame.near {
        return out;
    }
    let head_px = head_clip.xy / head_clip.w * 0.5 * frame.viewport;
    let tail_px = tail_clip.xy / tail_clip.w * 0.5 * frame.viewport;
    var along = head_px - tail_px;
    if dot(along, along) < 1.0e-6 {
        along = vec2f(0.0, -1.0e-3);
    }
    let direction = normalize(along);
    let side = vec2f(-direction.y, direction.x);
    // How wide the drop really is on screen, and how wide it is drawn.
    let right = vec3f(frame.view[0][0], frame.view[1][0], frame.view[2][0]);
    let beside = frame.view_proj * vec4f(head + right * width, 1.0);
    let true_px = length(beside.xy / beside.w * 0.5 * frame.viewport - head_px);
    let drawn_px = max(true_px, LEAST_WIDTH);
    // Corners: tail left, tail right, head left; head left, tail right, head right. The ends
    // reach out half a width so a short streak is a round drop.
    let corners = array<vec2f, 6>(
        vec2f(-1.0, 0.0), vec2f(1.0, 0.0), vec2f(-1.0, 1.0),
        vec2f(-1.0, 1.0), vec2f(1.0, 0.0), vec2f(1.0, 1.0),
    );
    let c = corners[corner];
    let end = select(tail_px - direction * drawn_px * 0.5, head_px + direction * drawn_px * 0.5, c.y > 0.5);
    let clip = select(tail_clip, head_clip, c.y > 0.5);
    let px = end + side * c.x * drawn_px * 0.5;
    out.position = vec4f(px / (0.5 * frame.viewport) * clip.w, clip.z, clip.w);
    out.at = c;
    let alpha = clamp(opacity * pow(true_px / drawn_px, THINNESS), 0.0, 1.0);
    out.color = vec4f(light * alpha, alpha);
    return out;
}

// How much of what is `distance` away the air lets through.
fn through_air(distance: f32) -> f32 {
    return exp(-distance * HAZE * (1.0 + 2.5 * rain.rain.y));
}

// A falling drop of ring `ring`, number `index` within it.
fn falling(corner: u32, ring: u32, index: u32) -> Varyings {
    var none: Varyings;
    none.position = vec4f(0.0);
    let cells = RING_CELLS[ring];
    let spacing = RING_SPACING[ring];
    let per_layer = cells * cells;
    let layer = index / per_layer;
    let cell_index = index % per_layer;
    // Only as many layers as this rain and the quality ask for; the last in part.
    let layers = rain.drops.w;
    let seed = hash4(cell_index, layer, ring);
    if f32(layer) + seed.w > layers {
        return none;
    }
    // The lattice is fixed in the air, which the wind carries: cells around the camera's place
    // in it.
    let camera = rain.camera.xyz;
    let air_camera = camera.xz - rain.wind.zw;
    let first = floor(air_camera / spacing) - vec2f(f32(cells / 2u));
    let cell = first + vec2f(f32(cell_index % cells), f32(cell_index / cells));
    let cell_seed = hash4(u32(i32(cell.x)), u32(i32(cell.y)), layer * 3u + ring);
    var xz = (cell + cell_seed.xy) * spacing + rain.wind.zw;
    // Inner rings draw near the camera; outer ones only beyond, fading in across the seam.
    let reach = f32(cells) * spacing * 0.5;
    let away = max(abs(xz.x - camera.x), abs(xz.y - camera.z));
    var fade = 1.0 - smoothstep(reach * 0.8, reach, away);
    if ring > 0u {
        let inner = f32(RING_CELLS[ring - 1u]) * RING_SPACING[ring - 1u] * 0.5;
        fade *= smoothstep(inner * 0.8, inner, away);
    }
    if fade <= 0.0 {
        return none;
    }
    let speed = rain.drops.y * (0.85 + 0.3 * cell_seed.z);
    let height = RING_HEIGHT[ring];
    let bottom = camera.y - height * 0.5;
    let fallen = cell_seed.w * height - rain.camera.w * speed - bottom;
    let y = bottom + fallen - floor(fallen / height) * height;
    // Drops drift with the air that carries their lattice, the mean wind; their streaks lean the
    // same way, so each streak points along its drop's own motion.
    let wind = rain.wind.xy;
    let head = vec3f(xz.x, y, xz.y);
    // Ended where it met something, or sheltered under it.
    let below = rain_map_at(head.xz);
    if below.w > 0.5 && y < below.x + 0.02 {
        return none;
    }
    // Ended where it met the sea.
    if y < frame.sea_level {
        return none;
    }
    // The column of air fades out at its top and bottom, so that looking straight up or down
    // no edge to the rain shows where the lattice wraps round.
    fade *= smoothstep(0.0, height * 0.15, y - bottom) * smoothstep(0.0, height * 0.15, bottom + height - y);
    let distance = length(head - camera);
    fade *= smoothstep(NEAREST, NEAREST * 2.5, distance) * through_air(distance);
    // A far drop seen against the sky shows about the sky's own light, and vanishes into it:
    // drawn there, the outer rings would speckle the clouds like snow.
    if ring > 0u {
        let elevation = (head.y - camera.y) / distance;
        fade *= 1.0 - smoothstep(0.05, 0.3, elevation);
    }
    let velocity = vec3f(wind.x, -speed, wind.y);
    let tail = head - velocity * rain.drops.z * RING_STRETCH[ring];
    let diameter = rain.drops.x * (0.7 + 0.6 * seed.x);
    // Bigger drops show more; a long streak, from a long exposure, spreads the same drop thinner.
    let length_m = max(length(velocity) * rain.drops.z, diameter);
    let opacity = STREAK_OPACITY * RING_STRENGTH[ring] * clamp(diameter / 0.0015, 0.5, 1.6)
        * sqrt(clamp(0.12 / length_m, 0.25, 1.0)) * fade;
    return streak(corner, tail, head, diameter * RING_WIDTH[ring], opacity, drop_light(head));
}

// A droplet of a splash where a drop struck the first surface.
fn splashing(corner: u32, index: u32) -> Varyings {
    var none: Varyings;
    none.position = vec4f(0.0);
    let cell_index = index / SPLASH_DROPLETS;
    let droplet = index % SPLASH_DROPLETS;
    let camera = rain.camera.xyz;
    let first = floor(camera.xz / SPLASH_SPACING) - vec2f(f32(SPLASH_CELLS / 2u));
    let cell = first + vec2f(f32(cell_index % SPLASH_CELLS), f32(cell_index / SPLASH_CELLS));
    let cell_seed = hash4(u32(i32(cell.x)), u32(i32(cell.y)), 17u);
    // How often a splash is seen in a cell: more, the harder it rains.
    let period = 0.35 / max(rain.rain.y, 0.05) * (0.7 + 0.6 * cell_seed.x);
    let cycles = rain.camera.w / period + cell_seed.y;
    let cycle = u32(floor(cycles));
    let life = 0.3;
    let age = fract(cycles) * period;
    if age > life || rain.rain.y <= 0.0 {
        return none;
    }
    let at = hash4(u32(i32(cell.x)) + cycle * 7919u, u32(i32(cell.y)), 23u);
    let xz = (cell + at.xy) * SPLASH_SPACING;
    let surface = rain_map_at(xz);
    // On the sea, at its surface; over open sea the cover map has nothing, but the sea is there.
    let has_sea = frame.sea_level > -1.0e29;
    if surface.w < 0.5 && !has_sea {
        return none;
    }
    let ground = select(frame.sea_level, max(surface.x, frame.sea_level), surface.w > 0.5);
    let d = hash4(cycle, droplet, cell_index);
    let angle = (f32(droplet) + d.x) * TAU / f32(SPLASH_DROPLETS);
    let out_speed = 0.4 + 0.8 * d.y;
    let up_speed = 0.8 + 1.0 * d.z;
    let start = vec3f(xz.x, ground + 0.005, xz.y);
    let velocity = vec3f(cos(angle) * out_speed, up_speed - GRAVITY * age, sin(angle) * out_speed);
    let head = start + vec3f(cos(angle) * out_speed * age, up_speed * age - 0.5 * GRAVITY * age * age, sin(angle) * out_speed * age);
    if head.y < ground {
        return none;
    }
    let distance = length(head - camera);
    // Splashes are tiny: seen only close by, and only as the briefest flecks. Drawn longer or
    // further off, their droplets read as a second rain, rising.
    let fade = smoothstep(NEAREST, NEAREST * 2.0, distance)
        * (1.0 - smoothstep(4.0, 7.0, distance)) * (1.0 - age / life);
    let tail = head - velocity * 0.004;
    let diameter = 0.0008 * (0.6 + 0.8 * d.w);
    return streak(corner, tail, head, diameter, 0.3 * fade, drop_light(head));
}

// A drop falling from an eave.
fn dripping(corner: u32, index: u32) -> Varyings {
    var none: Varyings;
    none.position = vec4f(0.0);
    let strength = rain.extra.x;
    if strength <= 0.0 {
        return none;
    }
    let camera = rain.camera.xyz;
    let first = floor(camera.xz / DRIP_SPACING) - vec2f(f32(DRIP_CELLS / 2u));
    let cell = first + vec2f(f32(index % DRIP_CELLS), f32(index / DRIP_CELLS));
    let xz = (cell + vec2f(0.5)) * DRIP_SPACING;
    let below = rain_map_at(xz);
    if below.w < 0.5 || below.z <= below.x {
        return none;
    }
    let seed = hash4(u32(i32(cell.x)), u32(i32(cell.y)), 31u);
    // Not every spot along an edge drips, and those that do, at their own pace.
    if seed.x > 0.35 + 0.5 * strength {
        return none;
    }
    let drop = below.z - below.x;
    let fall_time = sqrt(2.0 * drop / GRAVITY);
    let period = max((0.6 + 1.8 * seed.y) / strength, fall_time + 0.05);
    let age = fract(rain.camera.w / period + seed.z) * period;
    if age > fall_time {
        return none;
    }
    let head = vec3f(xz.x + (seed.w - 0.5) * 0.15, below.z - 0.5 * GRAVITY * age * age, xz.y);
    let velocity = vec3f(rain.wind.x * 0.05, -GRAVITY * age, rain.wind.y * 0.05);
    let distance = length(head - camera);
    let fade = smoothstep(NEAREST, NEAREST * 2.0, distance) * (1.0 - smoothstep(12.0, 16.0, distance));
    let tail = head - velocity * rain.drops.z - vec3f(0.0, 0.004, 0.0);
    let opacity = STREAK_OPACITY * 2.0 * fade;
    return streak(corner, tail, head, 0.004, opacity, drop_light(head));
}

@vertex
fn vs_main(@builtin(vertex_index) vertex: u32, @builtin(instance_index) instance: u32) -> Varyings {
    let kind = instance >> KIND_SHIFT;
    let index = instance & ((1u << KIND_SHIFT) - 1u);
    switch kind {
        case 0u, 1u, 2u: {
            return falling(vertex, kind, index);
        }
        case KIND_SPLASH: {
            return splashing(vertex, index);
        }
        default: {
            return dripping(vertex, index);
        }
    }
}

struct Translucent {
    @location(0) color: vec4f,
    // How much of the pixel the rain covers, for the temporal filter (post/taa.wgsl).
    @location(1) reactive: vec4f,
}

@fragment
fn fs_main(in: Varyings) -> Translucent {
    // Behind the scene: drawn only in front of whatever the pixel shows. Reversed depth, so
    // nearer is greater.
    let scene = textureLoad(scene_depth, vec2u(in.position.xy), 0).r;
    if scene > in.position.z {
        discard;
    }
    let across = exp(-in.at.x * in.at.x * 2.5);
    let color = in.color * across;
    var out: Translucent;
    out.color = color;
    out.reactive = vec4f(min(color.a * 4.0, 1.0));
    return out;
}
