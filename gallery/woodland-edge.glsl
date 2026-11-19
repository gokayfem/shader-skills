// A plan view of open meadow, oak groves and alder along a stream.
// @render time=1.0
// @variant DEBUG_MODE=3
// @variant DEBUG_MODE=5
#define DEBUG_MODE 0

// Library code below is bundled from shader-skills (MIT): https://github.com/gokayfem/shader-skills
// ---- shader-skills library: common ----
const float TAU = 6.28318530717959;
// 2D rotation matrix. `rot2(a) * v` rotates v counter-clockwise by a radians. (cos, sin) is renormalized because GLSL ES leaves trig precision undefined: on SwiftShader and some mobile GPUs a raw cos/sin pair is far enough from unit length that rotations stop preserving distances.
mat2 rot2(float a) {
    vec2 cs = normalize(vec2(cos(a), sin(a)));
    return mat2(cs.x, cs.y, -cs.y, cs.x);
}

// ---- shader-skills library: hash ----
// PCG-style 1D integer hash.
uint pcgHash(uint v) {
    uint state = v * 747796405u + 2891336453u;
    uint word = ((state >> ((state >> 28u) + 4u)) ^ state) * 277803737u;
    return (word >> 22u) ^ word;
}
// 3D integer hash, three decorrelated outputs.
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
// Map a uint to [0,1) using the top 24 bits, so the result can never round up to 1.0.
float uintToUnit(uint h) { return float(h >> 8u) * (1.0 / 16777216.0); }
vec2 uintToUnit(uvec2 h) { return vec2(h >> 8u) * (1.0 / 16777216.0); }
vec3 uintToUnit(uvec3 h) { return vec3(h >> 8u) * (1.0 / 16777216.0); }
vec4 uintToUnit(uvec4 h) { return vec4(h >> 8u) * (1.0 / 16777216.0); }
// Random float in [0,1) for an integer-valued 2D input.
float hash12(vec2 p) {
    uvec2 q = uvec2(ivec2(floor(p)));
    return uintToUnit(pcgHash(q.x + pcgHash(q.y)));
}
// Random float in [0,1) for an integer-valued 3D input.
float hash13(vec3 p) {
    uvec3 q = uvec3(ivec3(floor(p)));
    return uintToUnit(pcgHash(q.x + pcgHash(q.y + pcgHash(q.z))));
}
// Random vec3 in [0,1)^3 for an integer-valued 3D input.
vec3 hash33(vec3 p) { return uintToUnit(pcg3d(uvec3(ivec3(floor(p))))); }
// Random vec3 in [0,1)^3 for an integer-valued 2D input.
vec3 hash32(vec2 p) {
    ivec2 q = ivec2(floor(p));
    return uintToUnit(pcg3d(uvec3(q.x, q.y, 0x9E37u)));
}

// ---- shader-skills library: noise ----
// Quintic fade 6t^5 - 15t^4 + 10t^3: C2-continuous interpolation weights for lattice noise.
vec2 quinticFade(vec2 f) { return f * f * f * (f * (f * 6.0 - 15.0) + 10.0); }
vec3 quinticFade(vec3 f) { return f * f * f * (f * (f * 6.0 - 15.0) + 10.0); }
// Derivative of quinticFade: 30 t^2 (t - 1)^2.
vec2 quinticFadeDeriv(vec2 f) { return 30.0 * f * f * (f * (f - 2.0) + 1.0); }
vec3 quinticFadeDeriv(vec3 f) { return 30.0 * f * f * (f * (f - 2.0) + 1.0); }
// 2D value noise with analytic derivatives. Returns vec3(value, d/dx, d/dy).
vec3 valueNoiseD(vec2 p) {
    vec2 i = floor(p), f = fract(p);
    vec2 u = quinticFade(f), du = quinticFadeDeriv(f);
    float a = hash12(i);
    float b = hash12(i + vec2(1.0, 0.0));
    float c = hash12(i + vec2(0.0, 1.0));
    float d = hash12(i + vec2(1.0, 1.0));
    float k1 = b - a, k2 = c - a, k3 = a - b - c + d;
    return vec3(a + k1 * u.x + k2 * u.y + k3 * u.x * u.y,
                du * vec2(k1 + k3 * u.y, k2 + k3 * u.x));
}
// 2D value noise.
float valueNoise(vec2 p) { return valueNoiseD(p).x; }
// 3D value noise.
float valueNoise(vec3 p) {
    vec3 i = floor(p), f = fract(p);
    vec3 u = quinticFade(f);
    return mix(mix(mix(hash13(i + vec3(0, 0, 0)), hash13(i + vec3(1, 0, 0)), u.x),
                   mix(hash13(i + vec3(0, 1, 0)), hash13(i + vec3(1, 1, 0)), u.x), u.y),
               mix(mix(hash13(i + vec3(0, 0, 1)), hash13(i + vec3(1, 0, 1)), u.x),
                   mix(hash13(i + vec3(0, 1, 1)), hash13(i + vec3(1, 1, 1)), u.x), u.y), u.z);
}

// ---- shader-skills library: color ----
// Exact sRGB encode (OETF): linear -> display [0,1]. Apply exactly once, after tonemapping. pow(c, 1/2.2) is visibly wrong near black (0.001 encodes to 0.013 here vs 0.043 with 2.2).
vec3 linearToSrgb(vec3 c) {
    c = max(c, 0.0);
    return mix(c * 12.92, 1.055 * pow(c, vec3(1.0 / 2.4)) - 0.055, step(0.0031308, c));
}

// ---- shader ----
const float CLEARING = 0.7;
const float BANK_WIDTH = 1.5;

// Lateral distance from the stream centre, in world units.
float bankDistance(vec2 p) {
    float centre = 1.1 * sin(0.52 * p.y) + 0.32 * sin(1.47 * p.y + 0.5);
    return abs(p.x - centre);
}

float dampGround(vec2 p) {
    return 1.0 - smoothstep(0.32, 0.32 + BANK_WIDTH, bankDistance(p));
}

// Accumulate nearby crowns at the shading point and at the ground plant's root.
// Each crown stays within 1.6 units of its 2-unit founder cell.
void stand(vec2 p, vec2 root, float px, out float rootCover, out float shade,
           out vec3 crownColor, out float crownAlpha) {
    vec2 cell = floor(p * 0.5);
    rootCover = 0.0;
    shade = 0.0;
    crownColor = vec3(0.0);
    crownAlpha = 0.0;
    float top = -1.0;
    for (int y = -1; y <= 1; y++) {
        for (int x = -1; x <= 1; x++) {
            vec2 id = cell + vec2(float(x), float(y));
            vec3 seed = hash32(id);
            vec2 founder = 2.0 * (id + 0.2 + 0.6 * seed.xy);
            float wet = dampGround(founder);
            float alder = step(seed.z, wet);
            float suitability = max(wet, smoothstep(0.25, 0.75, valueNoise(founder * 0.26)));
            if (hash12(id + 91.0) > suitability) continue;
            for (int j = 0; j < 3; j++) {
                vec3 r = hash33(vec3(id, float(j)));
                vec2 centre = founder;
                float radius = mix(0.44, 0.76, r.z);
                if (j > 0) {
                    float angle = TAU * r.x;
                    centre += mix(0.40, 0.94, r.y) * vec2(cos(angle), sin(angle));
                    radius *= 0.64;
                }
                if (bankDistance(centre) < 0.34 || distance(centre, vec2(-2.5, 0.0)) < CLEARING) continue;
                vec2 q = (p - centre) / radius;
                float d = length(q);
                rootCover = max(rootCover, 1.0 - smoothstep(0.70, 1.42, distance(root, centre) / radius));
                shade = max(shade, 1.0 - smoothstep(0.77, 1.12, length(q - vec2(0.27, -0.24))));
                if (d > 1.18) continue;
                float lobes = valueNoise(3.5 * q + 16.0 * r.xy);
                float edge = d - (0.82 + 0.32 * lobes);
                float alpha = 1.0 - smoothstep(-px / radius, px / radius, edge);
                crownAlpha = max(crownAlpha, alpha);
                float height = radius * (1.25 - d * d + 0.24 * lobes);
                if (alpha > 0.0 && height > top) {
                    top = height;
                    float twigs = valueNoise(q * 8.0 + 24.0 * r.yz);
                    vec3 leaves = mix(vec3(0.16, 0.25, 0.061), vec3(0.072, 0.22, 0.13), alder);
                    float light = clamp(0.68 + dot(q, vec2(-0.34, 0.36)), 0.23, 1.0);
                    crownColor = leaves * light * (0.40 + lobes + 0.42 * twigs) * (0.84 + 0.3 * r.z);
                }
            }
        }
    }
}

void mainImage(out vec4 fragColor, in vec2 fragCoord) {
    float px = 8.0 / iResolution.y;
    vec2 p = (fragCoord - 0.5 * iResolution.xy) * px;
    vec2 id = floor(p * 3.0);
    vec3 seed = hash32(id);
    vec2 root = (id + 0.28 + 0.44 * seed.xy) / 3.0;
    float cover, shade, alpha;
    vec3 crown;
    stand(p, root, px, cover, shade, crown, alpha);

    float wet = dampGround(root);
    float edge = 4.0 * cover * (1.0 - cover);
    float fern = cover * wet;
    float shrub = edge * (1.0 - 0.5 * wet);
    float flowers = (1.0 - cover) * (1.0 - wet);
    float grain = valueNoise(p * 3.0) + 0.45 * valueNoise(p * 13.0);
    vec3 ground = mix(vec3(0.34, 0.32, 0.115), vec3(0.13, 0.21, 0.086), dampGround(p));
    ground = mix(ground, vec3(0.13, 0.085, 0.042), shade * 0.85) * (0.67 + 0.46 * grain);

    vec2 blade = rot2(seed.y * TAU) * (p - root);
    float fernShape = min(length(blade * vec2(0.7, 2.0)), length(blade * vec2(2.0, 0.7)));
    vec3 understory = vec3(0.0);
    float plant = 0.0;
    if (bankDistance(root) > 0.34) {
        if (seed.z < fern) {
            plant = 1.0 - smoothstep(0.040 - px, 0.040 + px, fernShape);
            understory = vec3(0.18, 0.36, 0.09);
        } else if (seed.z < fern + shrub) {
            plant = 1.0 - smoothstep(0.072 - px, 0.072 + px, length(blade));
            understory = vec3(0.085, 0.17, 0.047);
        } else if (seed.z > 1.0 - 0.36 * flowers) {
            plant = 1.0 - smoothstep(0.028 - px, 0.028 + px, length(blade));
            understory = vec3(0.82, 0.66, 0.26);
        }
    }
    float water = 1.0 - smoothstep(0.23, 0.31, bankDistance(p));
    vec3 stream = vec3(0.043, 0.19, 0.20) * (0.8 + 0.3 * valueNoise(vec2(p.x * 19.0, p.y * 3.0 - iTime)));
    vec3 col = mix(mix(ground, understory, plant), stream, water);
    col = mix(col * (1.0 - 0.35 * shade), crown, alpha);
#if DEBUG_MODE == 3
    col = vec3(fern, shrub, flowers) * (1.0 - water);
#elif DEBUG_MODE == 5
    col = vec3(cover, wet, alpha);
#endif
    fragColor = vec4(linearToSrgb(col), 1.0);
}
