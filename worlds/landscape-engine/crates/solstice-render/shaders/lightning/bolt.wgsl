// Draws a bolt of lightning: each segment of its channel as a ribbon facing the camera, a hot
// white core in a soft glow, added to the scene's light (lightning.rs). Hidden by whatever
// stands in front, and dimmed by the rain-laden air between.

#include "common/math.wgsl"
#include "common/frame.wgsl"

// Twin of `BoltUniform` in lightning.rs.
struct BoltParams {
    // The channel's radiance at full brightness, and how bright it is now, 0 to 1.
    color: vec4f,
    // How far the air lets light through: extinction per metre; then segments in use.
    air: vec4f,
}

struct Segment {
    // One end, and the channel's width there, in metres.
    a: vec4f,
    // The other end, and how bright this part of it is, 0 to 1.
    b: vec4f,
}

// Twin of `MAX_SEGMENTS` in lightning.rs.
const MAX_SEGMENTS: u32 = 384u;
// A bolt is never drawn thinner than this, in pixels: the glow around its channel shows from
// any distance.
const LEAST_WIDTH: f32 = 2.5;

@group(0) @binding(0) var<uniform> frame: Frame;
@group(0) @binding(1) var<uniform> bolt: BoltParams;
@group(0) @binding(2) var<storage, read> segments: array<Segment, MAX_SEGMENTS>;
@group(0) @binding(3) var scene_depth: texture_2d<f32>;

struct Varyings {
    @builtin(position) position: vec4f,
    // Across the ribbon, -1 to 1.
    @location(0) across: f32,
    @location(1) color: vec3f,
}

@vertex
fn vs_main(@builtin(vertex_index) vertex: u32, @builtin(instance_index) instance: u32) -> Varyings {
    var out: Varyings;
    out.position = vec4f(0.0);
    if instance >= u32(bolt.air.y) {
        return out;
    }
    let s = segments[instance];
    let a_clip = frame.view_proj * vec4f(s.a.xyz, 1.0);
    let b_clip = frame.view_proj * vec4f(s.b.xyz, 1.0);
    if a_clip.w < frame.near || b_clip.w < frame.near {
        return out;
    }
    let a_px = a_clip.xy / a_clip.w * 0.5 * frame.viewport;
    let b_px = b_clip.xy / b_clip.w * 0.5 * frame.viewport;
    var along = b_px - a_px;
    if dot(along, along) < 1.0e-6 {
        along = vec2f(0.0, 1.0e-3);
    }
    let side = normalize(vec2f(-along.y, along.x));
    // How wide the channel really is on screen at its nearer end, and how wide it is drawn.
    let right = vec3f(frame.view[0][0], frame.view[1][0], frame.view[2][0]);
    let beside = frame.view_proj * vec4f(s.a.xyz + right * s.a.w, 1.0);
    let true_px = length(beside.xy / beside.w * 0.5 * frame.viewport - a_px);
    let drawn_px = max(true_px * 3.0, LEAST_WIDTH);
    let corners = array<vec2f, 6>(
        vec2f(-1.0, 0.0), vec2f(1.0, 0.0), vec2f(-1.0, 1.0),
        vec2f(-1.0, 1.0), vec2f(1.0, 0.0), vec2f(1.0, 1.0),
    );
    let c = corners[vertex];
    let clip = select(a_clip, b_clip, c.y > 0.5);
    let end = select(a_px, b_px, c.y > 0.5);
    let px = end + side * c.x * drawn_px * 0.5;
    out.position = vec4f(px / (0.5 * frame.viewport) * clip.w, clip.z, clip.w);
    out.across = c.x;
    let middle = (s.a.xyz + s.b.xyz) * 0.5;
    let through = exp(-distance(middle, frame.camera_position) * bolt.air.x);
    // A channel thinner than its drawing spreads the same light over it.
    let spread = clamp(max(true_px, 0.3) * 3.0 / drawn_px, 0.05, 1.0);
    out.color = bolt.color.rgb * bolt.color.w * s.b.w * through * spread;
    return out;
}

@fragment
fn fs_main(in: Varyings) -> @location(0) vec4f {
    // Behind the scene: hidden. Reversed depth, so nearer is greater.
    let scene = textureLoad(scene_depth, vec2u(in.position.xy), 0).r;
    if scene > in.position.z {
        discard;
    }
    let x = in.across;
    // A white-hot core in a soft glow.
    let core = exp(-x * x * 18.0);
    let glow = exp(-x * x * 3.0) * 0.35;
    return vec4f(in.color * (core + glow), 0.0);
}
