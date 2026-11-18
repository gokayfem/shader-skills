// Studio still life: a smooth-blended sculpture on a plinth, with a gold ring.
// Shows: SDF modeling (smooth union whose blend weight also blends color), PBR key light with soft shadows,
// sky fill + bounce with AO, sky reflections, ray-footprint filtering, fog, AgX, supersampling, DEBUG_MODE.
// Paste-ready version: python3 scripts/glsl_tools.py bundle templates/raymarch-sdf.glsl -o scene.glsl
// @render time=1.2

#define DEBUG_MODE 0   // 0 final | 1 distance slice | 2 normals | 3 material ids | 4 steps | 5 shadow/AO/blend | 6 light terms
#define AA 1           // samples per pixel (4-8 for stills)

// Library code below is bundled from shader-skills (MIT): https://github.com/gokayfem/shader-skills
// ---- shader-skills library: common ----
const float PI  = 3.14159265358979;
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
// Random vec2 in [0,1)^2 for an integer-valued 2D input.
vec2 hash22(vec2 p) {
    ivec2 q = ivec2(floor(p));
    return uintToUnit(pcg3d(uvec3(q.x, q.y, 0x2545u))).xy;
}

// ---- shader-skills library: pattern ----
// Triangle wave, the integral of a square wave. Building block for box-filtered patterns.
vec2 triangleWave(vec2 x) { return 1.0 - 2.0 * abs(fract(x * 0.5) - 0.5); }
// Box-filtered checkerboard with unit squares: 0 or 1 up close, fading to 0.5 where squares are smaller than a pixel. Compute the footprint before any branching: w = fwidth(p) (or max(abs(dFdx(p)), abs(dFdy(p)))).
float filteredChecker(vec2 p, vec2 w) {
    w = max(w, 1e-4);
    vec2 i = (triangleWave(p + 0.5 * w) - triangleWave(p - 0.5 * w)) / w;
    return 0.5 - 0.5 * i.x * i.y;
}

// ---- shader-skills library: sdf3d ----
// Sphere of radius r.
float sdSphere(vec3 p, float r) { return length(p) - r; }
// Box with rounded edges. The outer size stays b; r is the edge radius (r <= min(b)).
float sdRoundBox(vec3 p, vec3 b, float r) {
    vec3 q = abs(p) - b + r;
    return length(max(q, 0.0)) + min(max(q.x, max(q.y, q.z)), 0.0) - r;
}
// Torus in the XZ plane. t.x = major radius, t.y = tube radius.
float sdTorus(vec3 p, vec2 t) {
    vec2 q = vec2(length(p.xz) - t.x, p.y);
    return length(q) - t.y;
}

// ---- shader-skills library: operators ----
// Smooth union (polynomial, C1). Blends where |a - b| < k, with a max bulge of k/4.
float smin(float a, float b, float k) {
    float h = clamp(0.5 + 0.5 * (b - a) / k, 0.0, 1.0);
    return mix(b, a, h) - k * h * (1.0 - h);
}
// Smooth intersection, the mirror of smin.
float smax(float a, float b, float k) { return -smin(-a, -b, k); }
// Smooth subtraction: a with b carved out, with filleted edges.
float smoothSubtract(float a, float b, float k) { return smax(a, -b, k); }
// Smooth union that also returns the blend weight. Returns vec2(distance, weight), where weight is 1 when the result is fully a and 0 when fully b. Use the weight to mix material colors across the blend.
vec2 sminBlend(float a, float b, float k) {
    float h = clamp(0.5 + 0.5 * (b - a) / k, 0.0, 1.0);
    return vec2(mix(b, a, h) - k * h * (1.0 - h), h);
}

// ---- shader-skills library: raymarch ----
#define ZERO (min(iFrame, 0))
vec2 map(vec3 p);
const int RAYMARCH_MAX_STEPS = 256;
// Sphere-trace from ro along unit rd between tmin and tmax. Returns vec3(t, materialId, stepCount). t is -1 on a miss. The hit threshold grows with t (about one pixel's footprint), so distant surfaces don't burn steps. stepScale is 1 for exact or bound fields. Use < 1 (e.g. 0.6) when map() has Lipschitz > 1 (displacement, warps, twist).
vec3 raymarch(vec3 ro, vec3 rd, float tmin, float tmax, float stepScale) {
    float t = tmin;
    for (int i = ZERO; i < RAYMARCH_MAX_STEPS; i++) {
        vec2 h = map(ro + rd * t);
        if (abs(h.x) < max(0.0002 * t, 1e-5)) return vec3(t, h.y, float(i));
        t += h.x * stepScale;
        if (t > tmax) break;
    }
    return vec3(-1.0, -1.0, float(RAYMARCH_MAX_STEPS));
}
// Surface normal from 4 tetrahedral samples (cheaper than 6 central differences, and still centered). eps should scale with distance, e.g. 0.0005 * t: that filters distant detail and avoids sparkling.
vec3 calcNormal(vec3 p, float eps) {
    vec3 n = vec3(0.0);
    for (int i = ZERO; i < 4; i++) {
        vec3 e = 0.5773 * (2.0 * vec3((((i + 3) >> 1) & 1), ((i >> 1) & 1), (i & 1)) - 1.0);
        n += e * map(p + e * eps).x;
    }
    return normalize(n);
}
// Soft shadow toward a light along unit rd. k controls sharpness (k is about 1 / light angular size: 8 soft, 32 hard). Uses the "closest approach between samples" estimate, which removes banding from sharp-cornered occluders. Returns 0 (full shadow) .. 1 (lit).
float softShadow(vec3 ro, vec3 rd, float tmin, float tmax, float k) {
    float res = 1.0;
    float t = tmin;
    float ph = 1e10;
    for (int i = ZERO; i < 64; i++) {
        float h = map(ro + rd * t).x;
        if (h < 0.0001) return 0.0;
        float y = h * h / (2.0 * ph);
        float d = sqrt(max(h * h - y * y, 0.0));
        res = min(res, k * d / max(t - y, 1e-4));
        ph = h;
        t += clamp(h, 0.005, 0.5);
        if (res < 0.001 || t > tmax) break;
    }
    res = clamp(res, 0.0, 1.0);
    return res * res * (3.0 - 2.0 * res);
}
// Ambient occlusion from 5 distance samples along the normal. Compares the expected distance with the actual one. scale is the sampling reach in scene units (about 0.12 for a unit-sized object). Returns 0 (occluded) .. 1 (open).
float calcAO(vec3 p, vec3 n, float scale) {
    float occ = 0.0;
    float weight = 1.0;
    for (int i = ZERO; i < 5; i++) {
        float h = 0.01 + scale * float(i) / 4.0;
        float d = map(p + h * n).x;
        occ += (h - d) * weight;
        weight *= 0.95;
    }
    return clamp(1.0 - 3.0 * occ / max(scale / 0.12, 1e-3), 0.0, 1.0);
}

// ---- shader-skills library: camera ----
// Camera-to-world matrix looking from ro toward ta, with roll in radians. Columns are (right, up, forward).
mat3 cameraMatrix(vec3 ro, vec3 ta, float roll) {
    vec3 w = normalize(ta - ro);
    vec3 up = vec3(sin(roll), cos(roll), 0.0);
    vec3 u = normalize(cross(w, up));
    vec3 v = cross(u, w);
    return mat3(u, v, w);
}
// Primary ray direction for screen point uv. focal is about 1.5-2.5 for natural framing (see fovToFocal).
vec3 cameraRay(mat3 cam, vec2 uv, float focal) {
    return normalize(cam * vec3(uv, focal));
}

// ---- shader-skills library: shading ----
// Schlick Fresnel. u is VoH for lights and NoV for environment reflections and rims. f0 = 0.04 for dielectrics (plastic, skin, stone, water ~0.02); for metals f0 = base color.
vec3 fresnelSchlick(float u, vec3 f0) {
    float m = clamp(1.0 - u, 0.0, 1.0);
    float m2 = m * m;
    return f0 + (1.0 - f0) * (m2 * m2 * m);
}
// GGX / Trowbridge-Reitz normal distribution. alpha = roughness^2.
float distributionGGX(float NoH, float alpha) {
    float a2 = alpha * alpha;
    float d = NoH * NoH * (a2 - 1.0) + 1.0;
    return a2 / (PI * d * d);
}
// Height-correlated Smith visibility. Already contains the 1 / (4 NoL NoV) term.
float visibilitySmithGGX(float NoV, float NoL, float alpha) {
    float a2 = alpha * alpha;
    float gv = NoL * sqrt(NoV * NoV * (1.0 - a2) + a2);
    float gl = NoV * sqrt(NoL * NoL * (1.0 - a2) + a2);
    return 0.5 / max(gv + gl, 1e-6);
}
// Microfacet specular for one light: pi * D * V * F * NoL (artist units). roughness is perceptual, in [0,1].
vec3 specularGGX(vec3 n, vec3 v, vec3 l, float roughness, vec3 f0) {
    vec3 h = normalize(v + l);
    float NoL = clamp(dot(n, l), 0.0, 1.0);
    float NoV = max(dot(n, v), 1e-4);
    float NoH = clamp(dot(n, h), 0.0, 1.0);
    float VoH = clamp(dot(v, h), 0.0, 1.0);
    float alpha = max(roughness, 0.045);
    alpha *= alpha;
    return PI * distributionGGX(NoH, alpha) * visibilitySmithGGX(NoV, NoL, alpha) * fresnelSchlick(VoH, f0) * NoL;
}
// Complete metallic-roughness shading for one light (diffuse + specular, NoL included, artist units). Multiply by light color and shadow. Keep albedo around 0.05-0.7 (linear); brighten lights, not albedo.
vec3 shadePBR(vec3 n, vec3 v, vec3 l, vec3 albedo, float roughness, float metallic) {
    vec3 f0 = mix(vec3(0.04), albedo, metallic);
    float NoL = clamp(dot(n, l), 0.0, 1.0);
    vec3 F = fresnelSchlick(max(dot(n, v), 0.0), f0);
    vec3 diffuse = albedo * (1.0 - metallic) * (1.0 - F) * NoL;
    return diffuse + specularGGX(n, v, l, roughness, f0);
}
// Split-sum environment specular without a lookup texture (Karis 2014, after Lazarov). Multiply by the sky or environment color in the reflection direction.
vec3 envBRDFApprox(vec3 f0, float NoV, float roughness) {
    vec4 r = roughness * vec4(-1.0, -0.0275, -0.572, 0.022) + vec4(1.0, 0.0425, 1.04, -0.04);
    float a = min(r.x * r.x, exp2(-9.28 * NoV)) * r.x + r.y;
    vec2 ab = vec2(-1.04, 1.04) * a + r.zw;
    return f0 * ab.x + ab.y;
}
// Hemisphere light: sky color from above blended to ground color from below.
vec3 hemisphereLight(vec3 n, vec3 skyColor, vec3 groundColor) {
    return mix(groundColor, skyColor, 0.5 + 0.5 * n.y);
}

// ---- shader-skills library: color ----
// Exact sRGB decode (EOTF): encoded [0,1] -> linear.
vec3 srgbToLinear(vec3 c) {
    c = max(c, 0.0);
    return mix(c / 12.92, pow((c + 0.055) / 1.055, vec3(2.4)), step(0.04045, c));
}
// Exact sRGB encode (OETF): linear -> display [0,1]. Apply exactly once, after tonemapping. pow(c, 1/2.2) is visibly wrong near black (0.001 encodes to 0.013 here vs 0.043 with 2.2).
vec3 linearToSrgb(vec3 c) {
    c = max(c, 0.0);
    return mix(c * 12.92, 1.055 * pow(c, vec3(1.0 / 2.4)) - 0.055, step(0.0031308, c));
}
// Relative luminance of linear Rec.709 / sRGB color.
float luminance(vec3 c) { return dot(c, vec3(0.2126, 0.7152, 0.0722)); }
// Linear sRGB -> Oklab (L in [0,1], a/b around [-0.4, 0.4]). Matrices from Bjorn Ottosson (public domain / MIT).
vec3 linearSrgbToOklab(vec3 c) {
    vec3 lms = c * mat3(0.4122214708, 0.5363325363, 0.0514459929,
                        0.2119034982, 0.6806995451, 0.1073969566,
                        0.0883024619, 0.2817188376, 0.6299787005);
    lms = sign(lms) * pow(abs(lms), vec3(1.0 / 3.0));
    return lms * mat3(0.2104542553, 0.7936177850, -0.0040720468,
                      1.9779984951, -2.4285922050, 0.4505937099,
                      0.0259040371, 0.7827717662, -0.8086757660);
}
// Oklab -> linear sRGB. The result can fall outside [0,1] for saturated colors, so clamp after tonemapping.
vec3 oklabToLinearSrgb(vec3 lab) {
    vec3 lms = lab * mat3(1.0, 0.3963377774, 0.2158037573,
                          1.0, -0.1055613458, -0.0638541728,
                          1.0, -0.0894841775, -1.2914855480);
    lms = lms * lms * lms;
    return lms * mat3(4.0767416621, -3.3077115913, 0.2309699292,
                      -1.2684380046, 2.6097574011, -0.3413193965,
                      -0.0041960863, -0.7034186147, 1.7076147010);
}
// Mix two linear colors in Oklab. Avoids the muddy mid-tones of RGB lerps and the hue detours of HSV.
vec3 mixOklab(vec3 a, vec3 b, float t) {
    return oklabToLinearSrgb(mix(linearSrgbToOklab(a), linearSrgbToOklab(b), t));
}

// ---- shader-skills library: tonemap ----
// AgX: log encoding plus a sigmoid inside an inset/outset gamut. Mid-grey 0.18 maps to about 0.5 display. Minimal implementation after Troy Sobotka's AgX and Benjamin Wrensch's polynomial fit (both MIT).
vec3 tonemapAgX(vec3 c) {
    const mat3 inset = mat3(0.842479062253094, 0.0423282422610123, 0.0423756549057051,
                            0.0784335999999992, 0.878468636469772, 0.0784336,
                            0.0792237451477643, 0.0791661274605434, 0.879142973793104);
    const mat3 outset = mat3(1.19687900512017, -0.0528968517574562, -0.0529716355144438,
                             -0.0980208811401368, 1.15190312990417, -0.0980434501171241,
                             -0.0990297440797205, -0.0989611768448433, 1.15107367264116);
    const float minEv = -12.47393, maxEv = 4.026069;
    c = inset * max(c, 0.0);
    c = clamp((log2(max(c, 1e-10)) - minEv) / (maxEv - minEv), 0.0, 1.0);
    vec3 x2 = c * c, x4 = x2 * x2;
    c = 15.5 * x4 * x2 - 40.14 * x4 * c + 31.96 * x4 - 6.868 * x2 * c + 0.4298 * x2 + 0.1191 * c - 0.00232;
    c = outset * c;
    return pow(max(c, 0.0), vec3(2.2));
}

// ---- shader-skills library: volume ----
// Distance fog amount (0 = clear, 1 = fully fogged) for distance t.
float distanceFog(float t, float density) { return 1.0 - exp(-t * density); }

// ---- shader-skills library: atmosphere ----
// Cheap analytic sky: horizon-to-zenith gradient, darker ground below the horizon, sun glow lobes. Returns linear radiance (roughly 0-2; the glow can exceed 1 near the sun).
vec3 simpleSky(vec3 rd, vec3 sunDir) {
    float y = max(rd.y, 0.0);
    vec3 zenith = vec3(0.12, 0.28, 0.62);
    vec3 horizon = vec3(0.62, 0.74, 0.88);
    vec3 col = mix(horizon, zenith, pow(y, 0.5));
    col = mix(col, vec3(0.28, 0.26, 0.24), 1.0 - smoothstep(-0.08, 0.0, rd.y));
    float mu = max(dot(rd, sunDir), 0.0);
    col += vec3(1.0, 0.75, 0.45) * (0.25 * pow(mu, 8.0) + 0.6 * pow(mu, 64.0) + 2.0 * pow(mu, 512.0));
    return col;
}

// ---- shader-skills library: post ----
// Subpixel offset in [-0.5, 0.5)^2 for sample i of a supersampling loop (R2 low-discrepancy sequence). Better coverage than a regular grid for any sample count.
vec2 aaOffset(int i) {
    return fract(0.5 + float(i) * vec2(0.7548776662, 0.5698402910)) - 0.5;
}
// Triangular-PDF dither of +-1 LSB for 8-bit output. Add AFTER linearToSrgb, right before writing fragColor.
vec3 ditherTPDF(vec2 fragCoord, int frame) {
    vec2 r = hash22(vec2(floor(fragCoord) + 7919.0 * float(frame % 1024)));
    return vec3((r.x + r.y - 1.0) / 255.0);
}

// ---- shader-skills library: debug ----
// Distance field as contour bands: warm outside, cool inside, white zero line. Bands are 0.1 units apart.
vec3 debugDistance(float d) {
    vec3 col = (d > 0.0) ? vec3(0.9, 0.6, 0.3) : vec3(0.4, 0.7, 0.85);
    col *= 1.0 - exp(-4.0 * abs(d));
    col *= 0.8 + 0.2 * cos(TAU * d / 0.1);
    return mix(col, vec3(1.0), 1.0 - smoothstep(0.0, 0.01, abs(d)));
}
// Normal as RGB.
vec3 debugNormal(vec3 n) { return 0.5 + 0.5 * n; }
// Distinct color per integer id.
vec3 debugIdColor(float id) {
    return 0.55 + 0.45 * cos(TAU * (0.618034 * id + vec3(0.0, 0.33, 0.67)));
}
// Heat ramp for a value in [0,1]: black, blue, magenta, orange, yellow, white.
vec3 debugHeat(float t) {
    t = clamp(t, 0.0, 1.0);
    return clamp(vec3(1.5 * t, 1.6 * t * t * t - 0.1, 0.9 * sin(PI * t) + 0.2 * t * t), 0.0, 1.0);
}

// ---- shader ----
// ---- art controls ----
const vec3  SUN_DIR = normalize(vec3(0.7, 0.6, -0.4));
const vec3  SUN_COLOR = vec3(3.2, 2.8, 2.3);
const vec3  SKY_FILL = vec3(0.35, 0.45, 0.6);
const vec3  GROUND_FILL = vec3(0.18, 0.15, 0.12);
const float BLEND = 0.25;        // smooth-union width: larger = softer, fleshier join
const float EXPOSURE = 1.0;
const float FOCAL = 2.2;         // larger = more telephoto

// Material ids: integer part = material, fractional part = blend weight between the two glazes.
const float MAT_FLOOR = 1.0;
const float MAT_PLINTH = 2.0;
const float MAT_GLAZE = 3.0;
const float MAT_GOLD = 4.0;

vec2 map(vec3 p) {
    vec2 res = vec2(p.y, MAT_FLOOR);

    float plinth = sdRoundBox(p - vec3(0.0, 0.3, 0.0), vec3(0.9, 0.3, 0.9), 0.04);
    if (plinth < res.x) res = vec2(plinth, MAT_PLINTH);

    // Sculpture: a rounded block and a ball melted together. sminBlend's weight drives the glaze color.
    vec3 q = p - vec3(0.0, 1.0, 0.0);
    q.xz = rot2(0.6) * q.xz;
    float block = sdRoundBox(q, vec3(0.32, 0.38, 0.32), 0.08);
    float ball = sdSphere(q - vec3(0.16, 0.44, 0.05), 0.3);
    vec2 blob = sminBlend(block, ball, BLEND);
    blob.x = smoothSubtract(blob.x, sdTorus(q - vec3(0.0, -0.08, 0.0), vec2(0.4, 0.05)), 0.04);
    if (blob.x < res.x) res = vec2(blob.x, MAT_GLAZE + 0.99 * blob.y);

    // Gold ring standing on the plinth.
    vec3 r = p - vec3(-0.45, 0.8, -0.5);
    r.yz = rot2(1.25) * r.yz;
    float ring = sdTorus(r, vec2(0.17, 0.035));
    if (ring < res.x) res = vec2(ring, MAT_GOLD);
    return res;
}

// footprint = world-space size of one pixel at the hit, used to filter the floor pattern without derivatives.
void material(float id, vec3 p, float footprint, out vec3 albedo, out float rough, out float metal) {
    float m = floor(id);
    rough = 0.5;
    metal = 0.0;
    if (m == MAT_FLOOR) {
        float checker = filteredChecker(p.xz * 2.0, vec2(footprint * 2.0));
        albedo = mix(vec3(0.30, 0.28, 0.26), vec3(0.42, 0.40, 0.37), checker);
        rough = 0.7;
    } else if (m == MAT_PLINTH) {
        albedo = vec3(0.55, 0.53, 0.5);
        rough = 0.85;
    } else if (m == MAT_GLAZE) {
        float w = fract(id) / 0.99;
        albedo = mixOklab(srgbToLinear(vec3(0.95, 0.45, 0.25)), srgbToLinear(vec3(0.15, 0.55, 0.6)), w);
        rough = 0.25;
    } else {
        albedo = vec3(1.0, 0.78, 0.34);
        rough = 0.2;
        metal = 1.0;
    }
}

vec3 render(vec2 fragCoord) {
    vec2 uv = (2.0 * fragCoord - iResolution.xy) / iResolution.y;
    float an = 0.35 * iTime;
    vec3 ro = vec3(3.4 * sin(an), 1.9, -3.4 * cos(an));
    vec3 ta = vec3(0.0, 0.85, 0.0);
    mat3 cam = cameraMatrix(ro, ta, 0.0);
    vec3 rd = cameraRay(cam, uv, FOCAL);

#if DEBUG_MODE == 1
    return debugDistance(map(vec3(uv.x * 1.6, uv.y * 1.6 + 0.9, 0.0)).x);   // slice at z = 0
#endif

    vec3 sky = simpleSky(rd, SUN_DIR);
    vec3 hit = raymarch(ro, rd, 0.1, 30.0, 1.0);
#if DEBUG_MODE == 4
    return debugHeat(hit.z / float(RAYMARCH_MAX_STEPS));
#endif
    if (hit.x < 0.0) return sky;

    float t = hit.x;
    vec3 p = ro + rd * t;
    vec3 n = calcNormal(p, 0.0005 * t);
    vec3 v = -rd;
    vec3 albedo;
    float rough, metal;
    material(hit.y, p, t * 2.0 / (iResolution.y * FOCAL), albedo, rough, metal);

    float sha = softShadow(p + n * 0.002, SUN_DIR, 0.01, 8.0, 12.0);
    float ao = calcAO(p, n, 0.15);
    vec3 refl = reflect(rd, n);
    vec3 f0 = mix(vec3(0.04), albedo, metal);

    vec3 key = shadePBR(n, v, SUN_DIR, albedo, rough, metal) * SUN_COLOR * sha;
    vec3 fill = albedo * (1.0 - metal) * hemisphereLight(n, SKY_FILL, GROUND_FILL) * ao;
    vec3 spec = envBRDFApprox(f0, max(dot(n, v), 0.0), rough) * simpleSky(refl, SUN_DIR)
              * ao * smoothstep(-0.1, 0.3, refl.y);
    vec3 bounceDir = normalize(vec3(-SUN_DIR.x, 0.0, -SUN_DIR.z));
    vec3 bounce = albedo * (1.0 - metal) * vec3(0.25, 0.18, 0.12) * clamp(dot(n, bounceDir), 0.0, 1.0) * ao;

#if DEBUG_MODE == 2
    return debugNormal(n);
#elif DEBUG_MODE == 3
    return debugIdColor(floor(hit.y));
#elif DEBUG_MODE == 5
    return vec3(sha, ao, fract(hit.y) / 0.99);
#elif DEBUG_MODE == 6
    return vec3(luminance(key), luminance(fill + bounce), luminance(spec));
#endif

    vec3 col = key + fill + spec + bounce;
    return mix(col, sky, distanceFog(t, 0.02));
}

void mainImage(out vec4 fragColor, in vec2 fragCoord) {
    vec3 acc = vec3(0.0);
    for (int i = 0; i < AA; i++) {
        vec3 c = render(fragCoord + (AA > 1 ? aaOffset(i) : vec2(0.0)));
#if DEBUG_MODE == 0
        c = tonemapAgX(c * EXPOSURE);   // tonemap per sample so bright edges antialias
#endif
        acc += c;
    }
    vec3 col = acc / float(AA);
#if DEBUG_MODE == 0
    col = linearToSrgb(col) + ditherTPDF(fragCoord, iFrame);
#endif
    fragColor = vec4(col, 1.0);
}
