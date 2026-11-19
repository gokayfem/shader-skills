// A stamped brass panel. The repeat supplies fine breakup and relief; a separate
// object-local mask allows patina to collect inside engraved paths.
// @render time=1.2
// @variant DEBUG_MODE=2
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
// Random vec3 in [0,1)^3 for an integer-valued 2D input.
vec3 hash32(vec2 p) {
    ivec2 q = ivec2(floor(p));
    return uintToUnit(pcg3d(uvec3(q.x, q.y, 0x9E37u)));
}

// ---- shader-skills library: pattern ----
// Equilateral triangle containing p (unit edge). Each column holds integer lattice ID in xy, barycentric weight in z.
mat3 triangleTileWeights(vec2 p) {
    float row = p.y * (2.0 / sqrt(3.0));
    vec2 lattice = vec2(p.x - 0.5 * row, row);
    vec2 base = floor(lattice);
    vec2 f = fract(lattice);
    vec2 tip = vec2(0.0);
    vec3 weights = vec3(1.0 - f.x - f.y, f.x, f.y);
    if (f.x + f.y > 1.0) {
        tip = vec2(1.0);
        weights = vec3(f.x + f.y - 1.0, 1.0 - f.y, 1.0 - f.x);
    }
    return mat3(vec3(base + tip, weights.x),
                vec3(base + vec2(1.0, 0.0), weights.y),
                vec3(base + vec2(0.0, 1.0), weights.z));
}

// ---- shader-skills library: color ----
// Exact sRGB encode (OETF): linear -> display [0,1]. Apply exactly once, after tonemapping. pow(c, 1/2.2) is visibly wrong near black (0.001 encodes to 0.013 here vs 0.043 with 2.2).
vec3 linearToSrgb(vec3 c) {
    c = max(c, 0.0);
    return mix(c * 12.92, 1.055 * pow(c, vec3(1.0 / 2.4)) - 0.055, step(0.0031308, c));
}

// ---- shader ----
const float SHUFFLE = 1.0;

// Periodic brushed grain. Integer frequencies make both axes wrap.
float rolledGrain(vec2 uv) {
    vec2 phase = TAU * uv;
    return 0.5 + 0.22 * sin(phase.y + 2.6 * sin(phase.x))
               + 0.18 * sin(3.0 * phase.y + 1.8 * cos(phase.x))
               + 0.10 * cos(phase.x + 5.0 * phase.y);
}

float sheetGrain(vec2 p) {
    mat3 tiles = triangleTileWeights(p);
    float grain = 0.0;
    float total = 0.0;
    for (int i = 0; i < 3; i++) {
        vec3 vertex = tiles[i];
        vec3 seed = hash32(vertex.xy);
        float weight = vertex.z * vertex.z;
        vec2 uv = rot2(TAU * seed.x) * p + seed.yz;
        grain += weight * rolledGrain(uv);
        total += weight;
    }
    return mix(rolledGrain(p), grain / total, SHUFFLE);
}

void mainImage(out vec4 fragColor, in vec2 fragCoord) {
    vec2 p = (2.0 * fragCoord - iResolution.xy) / iResolution.y;
    vec2 cell = fract(p * 1.35 + 0.5) - 0.5;
    float r = length(cell);
    float ring = min(abs(r - 0.33), abs(r - 0.385));
    float diamond = abs(abs(cell.x) + abs(cell.y) - 0.42) * 0.7071;
    float groove = min(ring, diamond);
    float width = max(fwidth(groove), 0.0001);
    float recess = 1.0 - smoothstep(0.004, 0.019 + width, groove);
    float grain = sheetGrain(p * 4.3);
    float deposit = smoothstep(0.20, 0.72, grain);
    float patina = recess * deposit;
    float height = 0.01 * grain - 0.022 * recess;
    float pixel = 2.0 / iResolution.y;
    vec3 normal = normalize(vec3(-dFdx(height), -dFdy(height), pixel));
    vec3 light = normalize(vec3(cos(iTime * 0.45), sin(iTime * 0.45), 1.1));
    vec3 brass = mix(vec3(0.29, 0.13, 0.035), vec3(0.58, 0.34, 0.11), grain);
    vec3 base = mix(brass, vec3(0.025, 0.17, 0.13), patina);
    float spec = pow(max(dot(normal, normalize(light + vec3(0.0, 0.0, 1.0))), 0.0), 48.0);
    vec3 lit = base * (0.28 + 0.72 * max(dot(normal, light), 0.0))
             + brass * spec * (1.0 - patina);
#if DEBUG_MODE == 2
    fragColor = vec4(0.5 + 0.5 * normal, 1.0);
#elif DEBUG_MODE == 5
    fragColor = vec4(deposit, recess, patina, 1.0);
#else
    fragColor = vec4(linearToSrgb(lit), 1.0);
#endif
}
