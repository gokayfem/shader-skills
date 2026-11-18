// Draws one emitter's particles as sprites facing the camera, over the lit scene, premultiplied.
// Each instance is a particle pulled from the emitter's alive list; there is no vertex buffer.
//
// A sprite fades out where it nears the surface behind it, over its emitter's soft distance, so
// it never shows a hard line where it crosses the ground or the water, and fades as it reaches
// the camera.
//
// Its light was worked out once for the whole particle by particles/light.wgsl: here it is
// scattered towards the eye by the emitter's phase function, its glow added, and the haze
// between it and the eye laid over it, as over everything else.
//
// Translucent particles mark how much of each pixel they cover in the reactive mask, for the
// temporal filter; solid ones are drawn first, by `fs_solid`, into the scene's depth and motion.

#include "common/frame.wgsl"
#include "particles/common.wgsl"
#include "particles/light_common.wgsl"

const CURVE_ENTRIES: u32 = 32u;
// Each emitter's curves: its colour's entries, then its size's, four to a vector.
const CURVE_STRIDE: u32 = 40u;
// The overdraw view adds this per layer: dark red for one, through yellow, to white.
const OVERDRAW_STEP: vec3f = vec3f(0.25, 0.06, 0.015);
// How strongly the reactive mask marks a particle, for its opacity.
const REACTIVE_GAIN: f32 = 4.0;

struct DrawParams {
    // Where the emitter's alive list starts, among both lists.
    list_offset: u32,
    emitter: u32,
    _pad0: u32,
    _pad1: u32,
}

@group(0) @binding(0) var<uniform> frame: Frame;
@group(1) @binding(0) var<storage, read> particles: array<Particle>;
@group(1) @binding(1) var<storage, read> alive: array<u32>;
@group(1) @binding(2) var<storage, read> emitters: array<Emitter, MAX_EMITTERS>;
@group(1) @binding(3) var<storage, read> curves: array<vec4f>;
@group(1) @binding(4) var<uniform> draw: DrawParams;
@group(1) @binding(5) var<storage, read> received: array<ParticleLight>;
// The scene's depth with the water in it: reversed Z, zero where there is only sky.
@group(2) @binding(0) var scene_depth: texture_2d<f32>;

struct Varyings {
    @builtin(position) clip: vec4f,
    // Where on the sprite, from -1 to 1 across it.
    @location(0) corner: vec2f,
    // The radiance the particle sends to the eye, through the haze, and its opacity; not yet
    // premultiplied.
    @location(1) color: vec4f,
    // Distance along the view, and over how far to fade into the scene and towards the camera.
    @location(2) @interpolate(flat) fades: vec3f,
    // Where this point of the sprite is now, without the camera's jitter, and was last frame,
    // for solid particles' motion.
    @location(3) now: vec4f,
    @location(4) before: vec4f,
}

// What a translucent particle writes: its light, premultiplied, and how much of the pixel it
// covers, for the temporal filter to trust history less there.
struct Translucent {
    @location(0) color: vec4f,
    @location(1) reactive: vec4f,
}

// What a solid particle writes, as any surface does: its light, depth and motion.
struct Solid {
    @location(0) color: vec4f,
    @location(1) depth: vec4f,
    @location(2) motion: vec4f,
}

fn curve_color(e: u32, t: f32) -> vec4f {
    let x = clamp(t, 0.0, 1.0) * f32(CURVE_ENTRIES - 1u);
    let i = u32(floor(x));
    let j = min(i + 1u, CURVE_ENTRIES - 1u);
    return mix(curves[e * CURVE_STRIDE + i], curves[e * CURVE_STRIDE + j], x - f32(i));
}

fn curve_size(e: u32, t: f32) -> f32 {
    let x = clamp(t, 0.0, 1.0) * f32(CURVE_ENTRIES - 1u);
    let i = u32(floor(x));
    let j = min(i + 1u, CURVE_ENTRIES - 1u);
    let first = e * CURVE_STRIDE + CURVE_ENTRIES;
    let a = curves[first + i / 4u][i % 4u];
    let b = curves[first + j / 4u][j % 4u];
    return mix(a, b, x - f32(i));
}

@vertex
fn vs_main(@builtin(vertex_index) vertex: u32, @builtin(instance_index) instance: u32) -> Varyings {
    let slot = alive[draw.list_offset + instance];
    let p = particles[slot];
    var out: Varyings;
    // Killed by a collision this frame: drawn as nothing, and freed next frame.
    if is_dead(p) {
        out.clip = vec4f(0.0, 0.0, -1.0, 1.0);
        return out;
    }
    let e = draw.emitter;
    let emitter = emitters[e];
    let t = p.age / max(p.lifetime, 1.0e-6);
    let size = p.size * curve_size(e, t);

    // Two triangles: (-1,-1) (1,-1) (1,1), (-1,-1) (1,1) (-1,1).
    var corners = array<vec2f, 6>(
        vec2f(-1.0, -1.0), vec2f(1.0, -1.0), vec2f(1.0, 1.0),
        vec2f(-1.0, -1.0), vec2f(1.0, 1.0), vec2f(-1.0, 1.0),
    );
    let corner = corners[vertex];
    let c = cos(p.rotation);
    let s = sin(p.rotation);
    let turned = vec2f(c * corner.x - s * corner.y, s * corner.x + c * corner.y);
    // The camera's right and up, from the rows of the view matrix.
    let right = vec3f(frame.view[0].x, frame.view[1].x, frame.view[2].x);
    let up = vec3f(frame.view[0].y, frame.view[1].y, frame.view[2].y);
    let offset = (right * turned.x + up * turned.y) * size;
    let world = p.position + offset;

    out.clip = frame.view_proj * vec4f(world, 1.0);
    out.corner = corner;
    // Last frame the particle stood at the start of its last step; the sprite turns with the
    // camera, so its corner is taken as it is now.
    out.now = frame.unjittered_view_proj * vec4f(world, 1.0);
    out.before = frame.prev_view_proj * vec4f(p.previous + offset, 1.0);
    let tint = emitter.color * curve_color(e, t);
    let light = unpack_light(received[slot]);
    // Scattered towards the eye: the sun by the phase function, the rest as it came.
    let to_eye = normalize(frame.camera_position - p.position);
    let sun = light.sun * phase(emitter.phase.x, dot(-frame.sun_direction, to_eye)) / PI;
    // The ambient light already carries the scene's GI intensity, from the light pass.
    let scattered = sun + light.ambient + light.lamps;
    var radiance = tint.rgb;
    if (emitter.flags & FLAG_LIT) != 0u {
        radiance *= scattered;
    }
    radiance += light.glow;
    if frame.debug_mode == DEBUG_PARTICLE_LIGHT {
        out.color = vec4f(scattered, tint.a);
    } else if (emitter.flags & FLAG_ADD) != 0u {
        // An additive particle covers nothing, so the haze over what is behind it is already
        // there: it adds only its own light, dimmed by the air.
        out.color = vec4f(radiance * light.transmittance, tint.a);
    } else {
        out.color = vec4f(light.haze + radiance * light.transmittance, tint.a);
    }
    let distance = -(frame.view * vec4f(p.position, 1.0)).z;
    out.fades = vec3f(distance, emitter.soft, max(2.0 * size, 0.2));
    return out;
}

// A soft round blob.
fn blob(corner: vec2f) -> f32 {
    return sq(max(1.0 - dot(corner, corner), 0.0));
}

@fragment
fn fs_main(in: Varyings) -> Translucent {
    var out: Translucent;
    if frame.debug_mode == DEBUG_PARTICLES {
        out.color = vec4f(OVERDRAW_STEP, 0.0);
        out.reactive = vec4f(0.0);
        return out;
    }
    let shape = blob(in.corner);

    let depth = textureLoad(scene_depth, vec2i(in.clip.xy), 0).r;
    var behind = 1.0e9;
    if depth > 0.0 {
        behind = frame.near / depth;
    }
    let distance = in.fades.x;
    var soft = 1.0;
    if in.fades.y > 0.0 {
        soft = clamp((behind - distance) / in.fades.y, 0.0, 1.0);
    }
    let near = clamp((distance - frame.near) / in.fades.z, 0.0, 1.0);

    let alpha = clamp(in.color.a * shape * soft * near, 0.0, 1.0);
    // Additive emitters cover nothing: premultiplied with no opacity, they only add light.
    let emitter = emitters[draw.emitter];
    let covers = select(alpha, 0.0, (emitter.flags & FLAG_ADD) != 0u);
    out.color = vec4f(in.color.rgb * alpha, covers);
    // Where it shows, the history behind it is not what is there now. Faint particles are
    // marked as nearly as strongly as opaque ones: they are what the filter loses most easily,
    // as their neighbourhood looks much like what is behind them.
    out.reactive = vec4f(min(alpha * REACTIVE_GAIN, 1.0));
    return out;
}

// A solid particle is opaque where its shape is more than half covered and absent elsewhere,
// and writes its depth and motion there, so the temporal filter follows it exactly.
@fragment
fn fs_solid(in: Varyings) -> Solid {
    if in.color.a * blob(in.corner) < 0.5 {
        discard;
    }
    var out: Solid;
    out.color = vec4f(in.color.rgb, 1.0);
    if frame.debug_mode == DEBUG_PARTICLES {
        out.color = vec4f(OVERDRAW_STEP, 1.0);
    }
    out.depth = vec4f(in.clip.z, 0.0, 0.0, 0.0);
    // In UV units, from here to where it was last frame, as the scene's motion is.
    let now = in.now.xy / in.now.w;
    let before = in.before.xy / in.before.w;
    out.motion = vec4f((before - now) * vec2f(0.5, -0.5), 0.0, 0.0);
    return out;
}
