// Eroded mountains at golden hour under a physically based sky.
// Shows: derivative-damped fBm terrain, heightfield marching as a scaled bound, slope/altitude material masks,
// soft terrain shadows, single-scattering atmosphere (Rayleigh/Mie/ozone), aerial perspective, sun disk, AgX.
// Units are kilometers so the atmosphere constants apply directly.
// @render time=0.0

#define DEBUG_MODE 0   // 0 final | 1 height map | 2 normals | 3 material masks | 4 steps | 5 shadow | 6 light terms

// Library code below is bundled from shader-skills (MIT): https://github.com/gokayfem/shader-skills
// ---- shader-skills library: common ----
const float PI  = 3.14159265358979;
const float TAU = 6.28318530717959;

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
// Random vec2 in [0,1)^2 for an integer-valued 2D input.
vec2 hash22(vec2 p) {
    ivec2 q = ivec2(floor(p));
    return uintToUnit(pcg3d(uvec3(q.x, q.y, 0x2545u))).xy;
}
// Random vec3 in [0,1)^3 for an integer-valued 3D input.
vec3 hash33(vec3 p) { return uintToUnit(pcg3d(uvec3(ivec3(floor(p))))); }

// ---- shader-skills library: noise ----
// Quintic fade 6t^5 - 15t^4 + 10t^3: C2-continuous interpolation weights for lattice noise.
vec2 quinticFade(vec2 f) { return f * f * f * (f * (f * 6.0 - 15.0) + 10.0); }
vec3 quinticFade(vec3 f) { return f * f * f * (f * (f * 6.0 - 15.0) + 10.0); }
// Derivative of quinticFade: 30 t^2 (t - 1)^2.
vec2 quinticFadeDeriv(vec2 f) { return 30.0 * f * f * (f * (f - 2.0) + 1.0); }
vec3 quinticFadeDeriv(vec3 f) { return 30.0 * f * f * (f * (f - 2.0) + 1.0); }
// Unit gradient for a 2D lattice point.
vec2 latticeGradient(vec2 cell) {
    float a = TAU * hash12(cell);
    return vec2(cos(a), sin(a));
}
// 2D gradient (Perlin-style) noise with analytic derivatives. Returns vec3(value, d/dx, d/dy). Gradient noise has less grid-aligned blockiness than value noise. Prefer it for fBm.
vec3 gradientNoiseD(vec2 p) {
    vec2 i = floor(p), f = fract(p);
    vec2 u = quinticFade(f), du = quinticFadeDeriv(f);
    vec2 ga = latticeGradient(i);
    vec2 gb = latticeGradient(i + vec2(1.0, 0.0));
    vec2 gc = latticeGradient(i + vec2(0.0, 1.0));
    vec2 gd = latticeGradient(i + vec2(1.0, 1.0));
    float va = dot(ga, f);
    float vb = dot(gb, f - vec2(1.0, 0.0));
    float vc = dot(gc, f - vec2(0.0, 1.0));
    float vd = dot(gd, f - vec2(1.0, 1.0));
    float k = va - vb - vc + vd;
    float value = va + u.x * (vb - va) + u.y * (vc - va) + u.x * u.y * k;
    vec2 deriv = ga + u.x * (gb - ga) + u.y * (gc - ga) + u.x * u.y * (ga - gb - gc + gd)
               + du * (vec2(vb - va, vc - va) + u.yx * k);
    return vec3(value, deriv);
}
// 2D gradient noise.
float gradientNoise(vec2 p) { return gradientNoiseD(p).x; }
// Dot product of a hashed 3D gradient with the offset to its lattice corner.
float latticeGradDot(vec3 cell, vec3 corner, vec3 f) {
    vec3 g = hash33(cell + corner) * 2.0 - 1.0;
    return dot(g, f - corner);
}
// 3D gradient noise.
float gradientNoise(vec3 p) {
    vec3 i = floor(p), f = fract(p);
    vec3 u = quinticFade(f);
    return mix(mix(mix(latticeGradDot(i, vec3(0, 0, 0), f), latticeGradDot(i, vec3(1, 0, 0), f), u.x),
                   mix(latticeGradDot(i, vec3(0, 1, 0), f), latticeGradDot(i, vec3(1, 1, 0), f), u.x), u.y),
               mix(mix(latticeGradDot(i, vec3(0, 0, 1), f), latticeGradDot(i, vec3(1, 0, 1), f), u.x),
                   mix(latticeGradDot(i, vec3(0, 1, 1), f), latticeGradDot(i, vec3(1, 1, 1), f), u.x), u.y), u.z);
}
// 3D gradient noise with analytic derivatives. Returns vec4(value, d/dx, d/dy, d/dz); value equals gradientNoise(p). The trilinear blend is expanded as a polynomial in the fade weights, so the gradient is the same blend of the corner gradients plus the chain-rule term of the fade.
vec4 gradientNoiseD(vec3 p) {
    vec3 i = floor(p), f = fract(p);
    vec3 u = quinticFade(f), du = quinticFadeDeriv(f);
    vec3 g000 = hash33(i) * 2.0 - 1.0;
    vec3 g100 = hash33(i + vec3(1, 0, 0)) * 2.0 - 1.0;
    vec3 g010 = hash33(i + vec3(0, 1, 0)) * 2.0 - 1.0;
    vec3 g110 = hash33(i + vec3(1, 1, 0)) * 2.0 - 1.0;
    vec3 g001 = hash33(i + vec3(0, 0, 1)) * 2.0 - 1.0;
    vec3 g101 = hash33(i + vec3(1, 0, 1)) * 2.0 - 1.0;
    vec3 g011 = hash33(i + vec3(0, 1, 1)) * 2.0 - 1.0;
    vec3 g111 = hash33(i + vec3(1, 1, 1)) * 2.0 - 1.0;
    float v000 = dot(g000, f);
    float v100 = dot(g100, f - vec3(1, 0, 0));
    float v010 = dot(g010, f - vec3(0, 1, 0));
    float v110 = dot(g110, f - vec3(1, 1, 0));
    float v001 = dot(g001, f - vec3(0, 0, 1));
    float v101 = dot(g101, f - vec3(1, 0, 1));
    float v011 = dot(g011, f - vec3(0, 1, 1));
    float v111 = dot(g111, f - vec3(1, 1, 1));
    // value = v000 + kx u.x + ky u.y + kz u.z + kxy u.x u.y + kyz u.y u.z + kzx u.z u.x + kxyz u.x u.y u.z
    float kx = v100 - v000, ky = v010 - v000, kz = v001 - v000;
    float kxy = v000 - v100 - v010 + v110;
    float kyz = v000 - v010 - v001 + v011;
    float kzx = v000 - v100 - v001 + v101;
    float kxyz = -v000 + v100 + v010 - v110 + v001 - v101 - v011 + v111;
    float value = v000 + kx * u.x + ky * u.y + kz * u.z
                + kxy * u.x * u.y + kyz * u.y * u.z + kzx * u.z * u.x + kxyz * u.x * u.y * u.z;
    vec3 grad = g000 + (g100 - g000) * u.x + (g010 - g000) * u.y + (g001 - g000) * u.z
              + (g000 - g100 - g010 + g110) * u.x * u.y
              + (g000 - g010 - g001 + g011) * u.y * u.z
              + (g000 - g100 - g001 + g101) * u.z * u.x
              + (-g000 + g100 + g010 - g110 + g001 - g101 - g011 + g111) * u.x * u.y * u.z
              + du * vec3(kx + kxy * u.y + kzx * u.z + kxyz * u.y * u.z,
                          ky + kxy * u.x + kyz * u.z + kxyz * u.x * u.z,
                          kz + kyz * u.y + kzx * u.x + kxyz * u.x * u.y);
    return vec4(value, grad);
}
const int FBM_MAX_OCTAVES = 12;
// Screen-filtered 2D fBm with the same spectrum as fbm(p, octaves). Pass the pixel width in input lattice cells. Each octave fades between 4 and 2 pixels per cell. Unresolved bands contribute their mean (zero).
float fbmLod(vec2 p, int octaves, float footprint) {
    int count = clamp(octaves, 0, FBM_MAX_OCTAVES);
    if (count == 0) return 0.0;
    float width = max(footprint, 0.0);
    float sum = 0.0, amplitude = 1.0;
    const mat2 rotation = mat2(0.8, 0.6, -0.6, 0.8);
    for (int i = 0; i < FBM_MAX_OCTAVES; i++) {
        if (i >= count || width >= 0.5) break;
        float keep = 1.0 - smoothstep(0.25, 0.5, width);
        sum += amplitude * keep * gradientNoise(p);
        p = 2.01 * (rotation * p) + vec2(17.13, -3.71);
        amplitude *= 0.5;
        width *= 2.01;
    }
    // Normalize by the requested spectrum so removing detail never boosts the remaining bands.
    return 1.4 * sum / (2.0 * (1.0 - exp2(-float(count))));
}
// Terrain fBm where each octave is damped by the accumulated slope, so detail settles in valleys and steep slopes stay smooth. This gives an eroded look without simulation.
float erodedFbm(vec2 p, int octaves) {
    const mat2 octaveRotation = mat2(0.8, 0.6, -0.6, 0.8);
    float h = 0.0, amp = 0.5;
    vec2 slope = vec2(0.0);
    for (int i = 0; i < FBM_MAX_OCTAVES; i++) {
        if (i >= octaves) break;
        vec3 n = gradientNoiseD(p);
        slope += n.yz;
        h += amp * n.x / (1.0 + dot(slope, slope));
        amp *= 0.5;
        p = 2.0 * (octaveRotation * p);
    }
    return 2.0 * h;
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

// ---- shader-skills library: intersect ----
// Ray vs sphere centered at c with radius r. Returns vec2(tNear, tFar), or vec2(-1) on a miss. Uses the precision-friendly form that measures from the closest point on the ray.
vec2 iSphere(vec3 ro, vec3 rd, vec3 c, float r) {
    vec3 oc = ro - c;
    float b = dot(oc, rd);
    vec3 qc = oc - b * rd;
    float h = r * r - dot(qc, qc);
    if (h < 0.0) return vec2(-1.0);
    h = sqrt(h);
    return vec2(-b - h, -b + h);
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

// ---- shader-skills library: color ----
// Exact sRGB encode (OETF): linear -> display [0,1]. Apply exactly once, after tonemapping. pow(c, 1/2.2) is visibly wrong near black (0.001 encodes to 0.013 here vs 0.043 with 2.2).
vec3 linearToSrgb(vec3 c) {
    c = max(c, 0.0);
    return mix(c * 12.92, 1.055 * pow(c, vec3(1.0 / 2.4)) - 0.055, step(0.0031308, c));
}
// Relative luminance of linear Rec.709 / sRGB color.
float luminance(vec3 c) { return dot(c, vec3(0.2126, 0.7152, 0.0722)); }

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
// Rayleigh phase function.
float phaseRayleigh(float mu) { return 3.0 / (16.0 * PI) * (1.0 + mu * mu); }
// Cornette-Shanks phase (a better-normalized Mie approximation). g = 0.8 for atmospheric haze.
float phaseCornetteShanks(float mu, float g) {
    float g2 = g * g;
    return 3.0 / (8.0 * PI) * (1.0 - g2) * (1.0 + mu * mu)
         / ((2.0 + g2) * pow(max(1.0 + g2 - 2.0 * g * mu, 1e-6), 1.5));
}
// One energy-conserving step of volume integration. sigmaT is extinction per unit length (RGB for tinted media), inScatter is in-scattered radiance at this point (including sigmaS, phase and light), ds is step length. Accumulates into radiance and transmittance. Stable for dense media, unlike radiance += T * S * ds.
void volumeStep(inout vec3 radiance, inout vec3 transmittance, vec3 sigmaT, vec3 inScatter, float ds) {
    vec3 stepT = exp(-sigmaT * ds);
    vec3 integral = (inScatter - inScatter * stepT) / max(sigmaT, vec3(1e-7));
    radiance += transmittance * integral;
    transmittance *= stepT;
}

// ---- shader-skills library: atmosphere ----
// Sun disk with antialiased edge and limb darkening (Neckel 1996). angularRadius in radians (the real sun is 0.00476; art often uses 3-10x). pixelAngle is the angular size of a pixel (about 2 / (iResolution.y * focal)). Returns a 0..1 shape tinted by limb darkening; multiply by a bright sun color (10-50).
vec3 sunDisk(vec3 rd, vec3 sunDir, float angularRadius, float pixelAngle) {
    float ang = acos(clamp(dot(rd, sunDir), -1.0, 1.0));
    float edge = 1.0 - smoothstep(angularRadius - pixelAngle, angularRadius + pixelAngle, ang);
    float r = clamp(ang / angularRadius, 0.0, 1.0);
    float muLimb = sqrt(max(1.0 - r * r, 0.0));
    vec3 limb = pow(vec3(muLimb), vec3(0.397, 0.503, 0.652));
    return edge * limb;
}
const float ATMOSPHERE_GROUND_KM = 6360.0;
const float ATMOSPHERE_TOP_KM = 6460.0;
// Densities (rayleigh, mie, ozone) at altitude h km above the ground.
vec3 atmosphereDensity(float h) {
    return vec3(exp(-h / 8.0), exp(-h / 1.2), max(0.0, 1.0 - abs(h - 25.0) / 15.0));
}
// Extinction per km for the given densities.
vec3 atmosphereExtinction(vec3 density) {
    const vec3 rayleigh = vec3(5.802, 13.558, 33.1) * 1e-3;
    const float mie = (3.996 + 4.40) * 1e-3;
    const vec3 ozone = vec3(0.650, 1.881, 0.085) * 1e-3;
    return rayleigh * density.x + mie * density.y + ozone * density.z;
}
// Physically based sky radiance along rd for a viewer viewHeightKm above the ground, with single scattering (16 view steps x 8 light steps). sunIntensity about 20. Sunsets come out somewhat dark because multiple scattering is ignored; add a small ambient term if needed.
vec3 atmosphereSky(vec3 rd, vec3 sunDir, float viewHeightKm, float sunIntensity) {
    const vec3 rayleighScatter = vec3(5.802, 13.558, 33.1) * 1e-3;
    const float mieScatter = 3.996e-3;
    vec3 ro = vec3(0.0, ATMOSPHERE_GROUND_KM + max(viewHeightKm, 0.001), 0.0);
    vec2 top = iSphere(ro, rd, vec3(0.0), ATMOSPHERE_TOP_KM);
    if (top.y <= 0.0) return vec3(0.0);
    vec2 ground = iSphere(ro, rd, vec3(0.0), ATMOSPHERE_GROUND_KM);
    float tStart = max(top.x, 0.0);
    float tEnd = (ground.x > 0.0) ? ground.x : top.y;
    float mu = dot(rd, sunDir);
    float pR = phaseRayleigh(mu);
    float pM = phaseCornetteShanks(mu, 0.8);
    const int VIEW_STEPS = 16;
    const int LIGHT_STEPS = 8;
    float dt = (tEnd - tStart) / float(VIEW_STEPS);
    vec3 radiance = vec3(0.0), transmittance = vec3(1.0);
    for (int i = 0; i < VIEW_STEPS; i++) {
        vec3 p = ro + rd * (tStart + (float(i) + 0.5) * dt);
        vec3 dens = atmosphereDensity(length(p) - ATMOSPHERE_GROUND_KM);
        vec3 sunT = vec3(0.0);
        if (iSphere(p, sunDir, vec3(0.0), ATMOSPHERE_GROUND_KM).x < 0.0) {
            float tl = iSphere(p, sunDir, vec3(0.0), ATMOSPHERE_TOP_KM).y;
            float dl = tl / float(LIGHT_STEPS);
            vec3 depth = vec3(0.0);
            for (int j = 0; j < LIGHT_STEPS; j++) {
                vec3 q = p + sunDir * ((float(j) + 0.5) * dl);
                depth += atmosphereExtinction(atmosphereDensity(length(q) - ATMOSPHERE_GROUND_KM)) * dl;
            }
            sunT = exp(-depth);
        }
        vec3 scatter = sunT * (rayleighScatter * dens.x * pR + mieScatter * dens.y * pM) * sunIntensity;
        volumeStep(radiance, transmittance, atmosphereExtinction(dens), scatter, dt);
    }
    return radiance;
}

// ---- shader-skills library: post ----
// Triangular-PDF dither of +-1 LSB for 8-bit output. Add AFTER linearToSrgb, right before writing fragColor.
vec3 ditherTPDF(vec2 fragCoord, int frame) {
    vec2 r = hash22(vec2(floor(fragCoord) + 7919.0 * float(frame % 1024)));
    return vec3((r.x + r.y - 1.0) / 255.0);
}

// ---- shader-skills library: debug ----
// Normal as RGB.
vec3 debugNormal(vec3 n) { return 0.5 + 0.5 * n; }
// Heat ramp for a value in [0,1]: black, blue, magenta, orange, yellow, white.
vec3 debugHeat(float t) {
    t = clamp(t, 0.0, 1.0);
    return clamp(vec3(1.5 * t, 1.6 * t * t * t - 0.1, 0.9 * sin(PI * t) + 0.2 * t * t), 0.0, 1.0);
}

// ---- shader ----
// ---- art controls ----
const vec3  SUN_DIR = normalize(vec3(0.85, 0.2, -0.45));
const float SUN_INTENSITY = 20.0;
const float TERRAIN_SCALE = 0.32;    // horizontal frequency: smaller = broader mountains
const float TERRAIN_HEIGHT = 2.0;    // vertical scale in km
const float SNOW_LINE = 1.05;        // km
const float EXPOSURE = 1.4;
const float FOCAL = 1.8;

float terrainHeight(vec2 xz) { return TERRAIN_HEIGHT * erodedFbm(xz * TERRAIN_SCALE, 9); }

vec2 map(vec3 p) {
    // p.y - h(xz) is not a distance on slopes. Scaling by 0.45 keeps the march safe for slopes up to about 63 degrees.
    return vec2(0.45 * (p.y - terrainHeight(p.xz)), 1.0);
}

// Transmittance from a point at heightKm toward dir, through the atmosphere (colors the sunlight).
vec3 sunTransmittance(float heightKm, vec3 dir) {
    vec3 ro = vec3(0.0, ATMOSPHERE_GROUND_KM + heightKm, 0.0);
    if (iSphere(ro, dir, vec3(0.0), ATMOSPHERE_GROUND_KM).x > 0.0) return vec3(0.0);
    float tl = iSphere(ro, dir, vec3(0.0), ATMOSPHERE_TOP_KM).y;
    vec3 depth = vec3(0.0);
    for (int i = 0; i < 12; i++) {
        vec3 q = ro + dir * ((float(i) + 0.5) * tl / 12.0);
        depth += atmosphereExtinction(atmosphereDensity(length(q) - ATMOSPHERE_GROUND_KM)) * tl / 12.0;
    }
    return exp(-depth);
}

vec3 render(vec2 fragCoord) {
    vec2 uv = (2.0 * fragCoord - iResolution.xy) / iResolution.y;

#if DEBUG_MODE == 1
    return vec3(terrainHeight(uv * 8.0) / TERRAIN_HEIGHT * 0.5 + 0.3);
#endif

    vec3 ro = vec3(-3.0 + 0.2 * iTime, 0.0, 4.0 - 0.6 * iTime);
    ro.y = terrainHeight(ro.xz) + 0.9;
    vec3 ta = ro + vec3(0.25, -0.16, -1.0);
    mat3 cam = cameraMatrix(ro, ta, 0.0);
    vec3 rd = cameraRay(cam, uv, FOCAL);

    vec3 sunColor = SUN_INTENSITY * 0.12 * sunTransmittance(ro.y, SUN_DIR);
    vec3 hit = raymarch(ro, rd, 0.01, 40.0, 1.0);
#if DEBUG_MODE == 4
    return debugHeat(hit.z / float(RAYMARCH_MAX_STEPS));
#endif
    if (hit.x < 0.0) {
        return atmosphereSky(rd, SUN_DIR, ro.y, SUN_INTENSITY) + sunColor * 40.0 * sunDisk(rd, SUN_DIR, 0.012, 2.0 / (iResolution.y * FOCAL));
    }

    float t = hit.x;
    vec3 p = ro + rd * t;
    vec3 n = calcNormal(p, 0.001 * t);

    // material masks: rock on steep slopes, grass on flats, snow on high upward-facing ground
    // Project a pixel cone onto the local surface, stretching it at grazing incidence.
    float footprint = (2.0 * t / (iResolution.y * FOCAL)) / max(abs(dot(n, rd)), 0.05);
    float flatness = smoothstep(0.72, 0.88, n.y);
    float snow = smoothstep(SNOW_LINE - 0.15, SNOW_LINE + 0.1, p.y + 0.25 * fbmLod(p.xz * 6.0, 3, footprint * 6.0))
               * smoothstep(0.55, 0.8, n.y);
    float grain = 0.5 + 0.5 * fbmLod(p.xz * 40.0, 4, footprint * 40.0);
    vec3 rock = mix(vec3(0.10, 0.085, 0.07), vec3(0.20, 0.17, 0.14), grain);
    vec3 grass = mix(vec3(0.06, 0.08, 0.03), vec3(0.11, 0.12, 0.05), grain);
    vec3 albedo = mix(rock, grass, flatness * (1.0 - smoothstep(0.6, 1.0, p.y)));
    albedo = mix(albedo, vec3(0.85, 0.88, 0.92), snow);

    float sha = softShadow(p + n * 0.01, SUN_DIR, 0.02, 20.0, 16.0);
    float NoL = clamp(dot(n, SUN_DIR), 0.0, 1.0);
    vec3 skyAmbient = atmosphereSky(vec3(0.0, 1.0, 0.0), SUN_DIR, p.y, SUN_INTENSITY);
    vec3 key = albedo * sunColor * NoL * sha;
    vec3 fill = albedo * skyAmbient * (0.5 + 0.5 * n.y) * 2.0;
    vec3 bounce = albedo * sunColor * 0.08 * clamp(dot(n, normalize(vec3(-SUN_DIR.x, 0.0, -SUN_DIR.z))), 0.0, 1.0);

#if DEBUG_MODE == 2
    return debugNormal(n);
#elif DEBUG_MODE == 3
    return vec3(1.0 - flatness, flatness, snow);
#elif DEBUG_MODE == 5
    return vec3(sha);
#elif DEBUG_MODE == 6
    return vec3(luminance(key), luminance(fill), luminance(bounce)) * 2.0;
#endif

    vec3 col = key + fill + bounce;
    // aerial perspective: attenuate by the air between camera and hit and fill with sky color. Use the sky at the
    // horizon-clamped direction: below the planet's horizon the sky integral jumps, which would print a seam.
    vec3 fogColor = atmosphereSky(normalize(vec3(rd.x, max(rd.y, 0.02), rd.z)), SUN_DIR, ro.y, SUN_INTENSITY);
    vec3 airT = exp(-t * atmosphereExtinction(atmosphereDensity(0.5 * (ro.y + p.y))) * 4.0);
    return col * airT + fogColor * (1.0 - airT);
}

void mainImage(out vec4 fragColor, in vec2 fragCoord) {
    vec3 col = render(fragCoord);
#if DEBUG_MODE == 0
    col = linearToSrgb(tonemapAgX(col * EXPOSURE)) + ditherTPDF(fragCoord, iFrame);
#endif
    fragColor = vec4(col, 1.0);
}
