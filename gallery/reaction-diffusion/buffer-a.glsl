

// Library code below is bundled from shader-skills (MIT): https://github.com/gokayfem/shader-skills
// ---- shader-skills library: hash ----
uvec3 pcg3d(uvec3 v) {
    v = v * 1664525u + 1013904223u;
    v.x += v.y * v.z;
    v.y += v.z * v.x;
    v.z += v.x * v.y;
    v ^= v >> 16u;
    v.x += v.y * v.z;
    v.y += v.z * v.x;
    v.z += v.x * v.y;
    return v;
}
float uintToUnit(uint h) { return float(h >> 8u) * (1.0 / 16777216.0); }
vec2 uintToUnit(uvec2 h) { return vec2(h >> 8u) * (1.0 / 16777216.0); }
vec3 uintToUnit(uvec3 h) { return vec3(h >> 8u) * (1.0 / 16777216.0); }
vec4 uintToUnit(uvec4 h) { return vec4(h >> 8u) * (1.0 / 16777216.0); }
vec3 hash32(vec2 p) {
    ivec2 q = ivec2(floor(p));
    return uintToUnit(pcg3d(uvec3(q.x, q.y, 0x9E37u)));
}

// ---- shader ----
// Gray-Scott reaction-diffusion. Buffer A channel 0 = Buffer A, floating-point, nearest.
// The display pass is independent of the solver. Click and drag to seed the B field.
const vec2 DIFFUSION = vec2(0.16, 0.08);
const float FEED = 0.035;
const float KILL = 0.061;

vec2 fieldAt(ivec2 pixel, ivec2 size) {
    // Clamp neighbors to impose zero outward flux.
    return texelFetch(iChannel0, clamp(pixel, ivec2(0), size - 1), 0).rg;
}

void mainImage(out vec4 fragColor, in vec2 fragCoord) {
    ivec2 p = ivec2(fragCoord), size = ivec2(iResolution.xy);
    vec2 uv = fragCoord / iResolution.xy;
    vec2 chemicals;
    if (iFrame == 0) {
        vec2 grid = fragCoord / 21.0;
        vec2 id = floor(grid);
        vec3 jitter = hash32(id);
        vec2 center = mix(vec2(0.22), vec2(0.78), jitter.xy);
        float seed = step(length(fract(grid) - center), 0.12 + 0.09*jitter.z);
        chemicals = mix(vec2(1,0),vec2(0.43,0.30+0.17*jitter.x),seed);
    } else {
        chemicals = fieldAt(p,size);
        vec2 lap = fieldAt(p+ivec2(1,0),size) + fieldAt(p-ivec2(1,0),size)
                 + fieldAt(p+ivec2(0,1),size) + fieldAt(p-ivec2(0,1),size) - 4.0*chemicals;
        // A spatial feed gradient gives the left and right sides different pattern regimes.
        float feed = FEED + 0.016 * (uv.x - 0.5);
        float reaction = chemicals.x * chemicals.y * chemicals.y;
        chemicals += DIFFUSION*lap + vec2(feed*(1.0-chemicals.x)-reaction,
                                           reaction-(KILL+feed)*chemicals.y);
    }
    if (iMouse.z > 0.0 && length(fragCoord-iMouse.xy)<5.0) chemicals = vec2(0.42,0.48);
    fragColor=vec4(chemicals,0.0,1.0);
}
