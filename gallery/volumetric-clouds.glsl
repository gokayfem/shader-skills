// Cumulus cloud layer lit by a low sun, raymarched as a participating medium.
// Shows: coverage-remapped 3D fBm density with a height profile, energy-conserving integration (volumeStep),
// dual-lobe phase for silver linings, Beer-Powder self-shadowing from a short light march, cheap density for
// lighting, jittered start (no banding), sky and sun, AgX.
// @render time=4.0

#define DEBUG_MODE 0   // 0 final | 4 march cost | 5 density / transmittance | 6 light vs ambient

// Library code below is bundled from shader-skills (MIT): https://github.com/gokayfem/shader-skills
// ---- shader-skills library: common ----
const float PI  = 3.14159265358979;
const float TAU = 6.28318530717959;
// Clamp to [0,1].
float saturate(float x) { return clamp(x, 0.0, 1.0); }
vec3 saturate(vec3 x) { return clamp(x, 0.0, 1.0); }

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
// Fractional Brownian motion over 2D gradient noise. Each octave is rotated to hide lattice alignment. persistence 0.5 is the classic look (H = 1). Lower is smoother, higher is rougher.
float fbm(vec2 p, int octaves, float lacunarity, float persistence) {
    const mat2 octaveRotation = mat2(0.8, 0.6, -0.6, 0.8);
    float sum = 0.0, amp = 1.0, norm = 0.0;
    for (int i = 0; i < FBM_MAX_OCTAVES; i++) {
        if (i >= octaves) break;
        sum += amp * gradientNoise(p);
        norm += amp;
        amp *= persistence;
        p = lacunarity * (octaveRotation * p) + vec2(17.13, -3.71);
    }
    return 1.4 * sum / norm;
}
// 2D fBm with the usual defaults (lacunarity 2.01 so octave peaks never re-align, persistence 0.5).
float fbm(vec2 p, int octaves) { return fbm(p, octaves, 2.01, 0.5); }
// fBm over 3D gradient noise.
float fbm(vec3 p, int octaves, float lacunarity, float persistence) {
    const mat3 octaveRotation = mat3(0.00, 0.80, 0.60,
                                     -0.80, 0.36, -0.48,
                                     -0.60, -0.48, 0.64);
    float sum = 0.0, amp = 1.0, norm = 0.0;
    for (int i = 0; i < FBM_MAX_OCTAVES; i++) {
        if (i >= octaves) break;
        sum += amp * gradientNoise(p);
        norm += amp;
        amp *= persistence;
        p = lacunarity * (octaveRotation * p) + vec3(17.13, -3.71, 5.29);
    }
    return sum / norm;
}
// Interleaved gradient noise: cheap per-pixel noise whose 3x3 neighborhoods are nearly stratified. Not a hash. pixel must be integer pixel coordinates (floor(fragCoord)). The frame overload animates it for accumulation and TAA.
float interleavedGradientNoise(vec2 pixel) {
    return fract(52.9829189 * fract(dot(pixel, vec2(0.06711056, 0.00583715))));
}
float interleavedGradientNoise(vec2 pixel, int frame) {
    return interleavedGradientNoise(pixel + 5.588238 * float(frame % 64));
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
// Henyey-Greenstein phase function. g in (-1, 1): > 0 forward (silver linings), < 0 backward.
float phaseHG(float mu, float g) {
    float g2 = g * g;
    return (1.0 - g2) / (4.0 * PI * pow(max(1.0 + g2 - 2.0 * g * mu, 1e-6), 1.5));
}
// Two-lobe Henyey-Greenstein: a forward lobe gF and a backward lobe gB, mixed by w (weight of the forward lobe). Keeps the silver lining while clouds lit from behind the camera still show detail.
float phaseDualHG(float mu, float gF, float gB, float w) {
    return mix(phaseHG(mu, gB), phaseHG(mu, gF), w);
}
// Beer-Powder lighting term for clouds: 2 e^-d (1 - e^-2d), where d is optical depth toward the sun. Darkens cloud edges that face the light like powdered sugar. Not energy-conserving; use it on the direct term only.
float beerPowder(float d) { return 2.0 * exp(-d) * (1.0 - exp(-2.0 * d)); }
// One energy-conserving step of volume integration. sigmaT is extinction per unit length (RGB for tinted media), inScatter is in-scattered radiance at this point (including sigmaS, phase and light), ds is step length. Accumulates into radiance and transmittance. Stable for dense media, unlike radiance += T * S * ds.
void volumeStep(inout vec3 radiance, inout vec3 transmittance, vec3 sigmaT, vec3 inScatter, float ds) {
    vec3 stepT = exp(-sigmaT * ds);
    vec3 integral = (inScatter - inScatter * stepT) / max(sigmaT, vec3(1e-7));
    radiance += transmittance * integral;
    transmittance *= stepT;
}

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
// Sun disk with antialiased edge and limb darkening (Neckel 1996). angularRadius in radians (the real sun is 0.00476; art often uses 3-10x). pixelAngle is the angular size of a pixel (about 2 / (iResolution.y * focal)). Returns a 0..1 shape tinted by limb darkening; multiply by a bright sun color (10-50).
vec3 sunDisk(vec3 rd, vec3 sunDir, float angularRadius, float pixelAngle) {
    float ang = acos(clamp(dot(rd, sunDir), -1.0, 1.0));
    float edge = 1.0 - smoothstep(angularRadius - pixelAngle, angularRadius + pixelAngle, ang);
    float r = clamp(ang / angularRadius, 0.0, 1.0);
    float muLimb = sqrt(max(1.0 - r * r, 0.0));
    vec3 limb = pow(vec3(muLimb), vec3(0.397, 0.503, 0.652));
    return edge * limb;
}

// ---- shader-skills library: post ----
// Triangular-PDF dither of +-1 LSB for 8-bit output. Add AFTER linearToSrgb, right before writing fragColor.
vec3 ditherTPDF(vec2 fragCoord, int frame) {
    vec2 r = hash22(vec2(floor(fragCoord) + 7919.0 * float(frame % 1024)));
    return vec3((r.x + r.y - 1.0) / 255.0);
}

// ---- shader-skills library: debug ----
// Heat ramp for a value in [0,1]: black, blue, magenta, orange, yellow, white.
vec3 debugHeat(float t) {
    t = clamp(t, 0.0, 1.0);
    return clamp(vec3(1.5 * t, 1.6 * t * t * t - 0.1, 0.9 * sin(PI * t) + 0.2 * t * t), 0.0, 1.0);
}

// ---- shader ----
// ---- art controls ----
const vec3  SUN_DIR = normalize(vec3(0.6, 0.25, -0.75));
const vec3  SUN_COLOR = vec3(6.0, 5.2, 4.2);
const float CLOUD_BOTTOM = 1.5;
const float CLOUD_TOP = 3.2;
const float COVERAGE = 0.42;       // 0 = clear, 1 = overcast
const float DENSITY = 9.0;         // extinction scale: higher = darker, more opaque cores
const float WIND = 0.06;
const float EXPOSURE = 1.0;

// Density in [0, DENSITY]. Use fewer octaves for the light-march samples.
float cloudDensity(vec3 p, int octaves) {
    if (COVERAGE <= 0.0) return 0.0;
    float h = (p.y - CLOUD_BOTTOM) / (CLOUD_TOP - CLOUD_BOTTOM);
    if (h < 0.0 || h > 1.0) return 0.0;
    float profile = smoothstep(0.0, 0.12, h) * (1.0 - smoothstep(0.45, 1.0, h));   // flat-ish bottoms, rounded tops
    vec3 q = p * 0.55 + vec3(iTime * WIND, 0.0, 0.0);
    float n = 0.5 + 0.5 * fbm(q, octaves, 2.02, 0.5);
    float d = saturate((n * profile - (1.0 - COVERAGE)) / COVERAGE);
    return d * DENSITY;
}

vec3 render(vec2 fragCoord) {
    vec2 uv = (2.0 * fragCoord - iResolution.xy) / iResolution.y;
    vec3 ro = vec3(0.0, 0.3, 0.0);
    mat3 cam = cameraMatrix(ro, ro + vec3(0.35, 0.32, -1.0), 0.0);
    vec3 rd = cameraRay(cam, uv, 1.6);

    vec3 sky = simpleSky(rd, SUN_DIR) * 1.2;
    vec3 col = sky + SUN_COLOR * sunDisk(rd, SUN_DIR, 0.03, 2.0 / (iResolution.y * 1.6));
    if (rd.y <= 0.01) return col;

    // clip the march to the cloud slab
    float t0 = (CLOUD_BOTTOM - ro.y) / rd.y;
    float t1 = min((CLOUD_TOP - ro.y) / rd.y, 30.0);
    if (t1 <= t0) return col;
    const int STEPS = 48;
    float dt = (t1 - t0) / float(STEPS);
    float t = t0 + dt * interleavedGradientNoise(floor(fragCoord), iFrame);   // jitter hides slicing

    float mu = dot(rd, SUN_DIR);
    float phase = 4.0 * PI * phaseDualHG(mu, 0.75, -0.25, 0.75);   // artist units: isotropic scattering = 1
    vec3 radiance = vec3(0.0), transmittance = vec3(1.0);
    float lightAcc = 0.0, ambientAcc = 0.0;
    int steps = 0;
    for (int i = 0; i < STEPS; i++) {
        vec3 p = ro + rd * t;
        float sigma = cloudDensity(p, 5);
        if (sigma > 0.001) {
            // short light march toward the sun with the cheap (3-octave) density
            float depth = 0.0;
            for (int j = 0; j < 5; j++) depth += cloudDensity(p + SUN_DIR * (0.1 + 0.25 * float(j)), 3) * 0.25;
            float h = (p.y - CLOUD_BOTTOM) / (CLOUD_TOP - CLOUD_BOTTOM);
            vec3 sunLight = SUN_COLOR * phase * mix(exp(-depth), beerPowder(depth), 0.5 - 0.5 * mu);
            vec3 ambient = vec3(0.22, 0.28, 0.38) * (0.3 + 0.7 * h);
            volumeStep(radiance, transmittance, vec3(sigma), sigma * (sunLight + ambient), dt);
            lightAcc += luminance(sunLight) * transmittance.x;
            ambientAcc += luminance(ambient) * transmittance.x;
        }
        steps++;
        if (transmittance.x < 0.01) break;
        t += dt;
    }

#if DEBUG_MODE == 4
    return debugHeat(float(steps) / float(STEPS));
#elif DEBUG_MODE == 5
    return vec3(1.0 - transmittance.x, cloudDensity(ro + rd * (t0 + 0.5 * (t1 - t0)), 5) / DENSITY, 0.0);
#elif DEBUG_MODE == 6
    return vec3(lightAcc, ambientAcc, 0.0) * 0.1;
#endif

    // distant clouds fade into the sky
    float fade = exp(-0.04 * max(t0, 0.0));
    return mix(col, col * transmittance + radiance, fade);
}

void mainImage(out vec4 fragColor, in vec2 fragCoord) {
    vec3 col = render(fragCoord);
#if DEBUG_MODE == 0
    col = linearToSrgb(tonemapAgX(col * EXPOSURE)) + ditherTPDF(fragCoord, iFrame);
#endif
    fragColor = vec4(col, 1.0);
}
