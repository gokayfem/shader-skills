// Open ocean at golden hour under a physically based sky.
// Shows: a sum of dispersive exp-sine waves with analytic slope and curvature (normals and foam with no extra samples),
// a heightfield tracer clipped to the wave slab, distance filtering that hands the slope of vanished waves to GGX
// roughness (the glitter path widens with distance), simplex flow-noise ripples, Fresnel sky reflection, a sun glint,
// light glowing through thin crests, curvature foam broken up by billow noise, aerial perspective, AgX.
// Units are meters; the atmosphere functions take kilometers.
// @render time=12.5

#define DEBUG_MODE 0   // 0 final | 1 height | 2 normals | 3 foam mask | 4 trace steps | 5 fresnel | 6 light terms

// Library code below is bundled from shader-skills (MIT): https://github.com/gokayfem/shader-skills
// ---- shader-skills library: common ----
const float PI  = 3.14159265358979;
const float TAU = 6.28318530717959;
// Squared length.
float dot2(vec2 v) { return dot(v, v); }
float dot2(vec3 v) { return dot(v, v); }
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
// Random vec2 in [0,1)^2 for an integer-valued 2D input.
vec2 hash22(vec2 p) {
    ivec2 q = ivec2(floor(p));
    return uintToUnit(pcg3d(uvec3(q.x, q.y, 0x2545u))).xy;
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
// 2D simplex noise with analytic gradient. Returns vec3(value, d/dx, d/dy). Sums three radial kernels (0.5 - r^2)^4 on a triangular lattice instead of blending four square-cell corners, so it has fewer axis-aligned artifacts than gradientNoise. rotation (radians) spins every lattice gradient: animating it gives flow noise that churns in place instead of sliding (Perlin and Neyret 2001). Scaled so |value| <= 1 exactly.
vec3 simplexNoiseD(vec2 p, float rotation) {
    const float SKEW = 0.36602540378;     // (sqrt(3) - 1) / 2: square lattice -> triangle lattice
    const float UNSKEW = 0.21132486541;   // (3 - sqrt(3)) / 6
    vec2 cell = floor(p + (p.x + p.y) * SKEW);
    vec2 d0 = p - cell + (cell.x + cell.y) * UNSKEW;
    vec2 o = (d0.x > d0.y) ? vec2(1.0, 0.0) : vec2(0.0, 1.0);   // which of the cell's two triangles holds p
    vec2 corners[3] = vec2[3](cell, cell + o, cell + 1.0);
    vec2 offsets[3] = vec2[3](d0, d0 - o + UNSKEW, d0 - 1.0 + 2.0 * UNSKEW);
    mat2 spin = rot2(rotation);
    vec3 sum = vec3(0.0);
    for (int k = 0; k < 3; k++) {
        vec2 d = offsets[k];
        float t = max(0.5 - dot(d, d), 0.0);
        vec2 g = spin * latticeGradient(corners[k]);
        float gd = dot(g, d);
        float t3 = t * t * t;
        // value t^4 (g.d); gradient t^4 g - 8 t^3 (g.d) d
        sum += vec3(t3 * t * gd, t3 * t * g - 8.0 * t3 * gd * d);
    }
    return 99.2 * sum;   // 1 / max over the simplex of sum t^4 |d|
}
vec3 simplexNoiseD(vec2 p) { return simplexNoiseD(p, 0.0); }
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
// Billowy multifractal: |noise| per octave makes puffy rounded lobes split by sharp creases (the inverse of ridgedFbm). Each octave is weighted by the previous one, so detail gathers on the lobes while the creases stay clean.
float billowFbm(vec2 p, int octaves) {
    const mat2 octaveRotation = mat2(0.8, 0.6, -0.6, 0.8);
    float sum = 0.0, amp = 0.5, weight = 1.0, norm = 0.0;
    for (int i = 0; i < FBM_MAX_OCTAVES; i++) {
        if (i >= octaves) break;
        float n = abs(gradientNoise(p) * 1.4);
        n = n * (2.0 - n);   // round the lobe tops, keep the creases sharp
        sum += amp * n * weight;
        norm += amp;
        weight = clamp(n * 1.5, 0.0, 1.0);
        amp *= 0.5;
        p = 2.01 * (octaveRotation * p) + vec2(17.13, -3.71);
    }
    return sum / norm;
}
float billowFbm(vec3 p, int octaves) {
    const mat3 octaveRotation = mat3(0.00, 0.80, 0.60,
                                     -0.80, 0.36, -0.48,
                                     -0.60, -0.48, 0.64);
    float sum = 0.0, amp = 0.5, weight = 1.0, norm = 0.0;
    for (int i = 0; i < FBM_MAX_OCTAVES; i++) {
        if (i >= octaves) break;
        float n = min(abs(gradientNoise(p)) * 1.4, 1.0);
        n = n * (2.0 - n);
        sum += amp * n * weight;
        norm += amp;
        weight = clamp(n * 1.5, 0.0, 1.0);
        amp *= 0.5;
        p = 2.01 * (octaveRotation * p) + vec3(17.13, -3.71, 5.29);
    }
    return sum / norm;
}
// Exact distance to Voronoi cell borders, found in two passes: nearest point, then nearest bisector. Returns vec3(borderDistance, vectorToNearestPoint). Unlike F2-F1, the border distance is a true distance, so lines have constant width.
vec3 voronoiBorder(vec2 p, float jitter) {
    vec2 ip = floor(p), fp = fract(p);
    vec2 mg = vec2(0.0), mr = vec2(0.0);
    float md = 8.0;
    for (int j = -1; j <= 1; j++)
    for (int i = -1; i <= 1; i++) {
        vec2 g = vec2(float(i), float(j));
        vec2 r = g + jitter * hash22(ip + g) - fp;
        float d = dot(r, r);
        if (d < md) { md = d; mr = r; mg = g; }
    }
    md = 8.0;
    for (int j = -2; j <= 2; j++)
    for (int i = -2; i <= 2; i++) {
        vec2 g = mg + vec2(float(i), float(j));
        vec2 r = g + jitter * hash22(ip + g) - fp;
        if (dot2(mr - r) > 1e-5) {
            md = min(md, dot(0.5 * (mr + r), normalize(r - mr)));
        }
    }
    return vec3(md, mr);
}

// ---- shader-skills library: raymarch ----
#define ZERO (min(iFrame, 0))

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

// ---- shader-skills library: color ----
// Exact sRGB encode (OETF): linear -> display [0,1]. Apply exactly once, after tonemapping. pow(c, 1/2.2) is visibly wrong near black (0.001 encodes to 0.013 here vs 0.043 with 2.2).
vec3 linearToSrgb(vec3 c) {
    c = max(c, 0.0);
    return mix(c * 12.92, 1.055 * pow(c, vec3(1.0 / 2.4)) - 0.055, step(0.0031308, c));
}
// Relative luminance of linear Rec.709 / sRGB color.
float luminance(vec3 c) { return dot(c, vec3(0.2126, 0.7152, 0.0722)); }

// ---- shader-skills library: tonemap ----
// AgX with the "punchy" look (more contrast and saturation) for stylized art.
vec3 tonemapAgXPunchy(vec3 c) {
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
    c = pow(max(c, 0.0), vec3(1.35));
    float luma = dot(c, vec3(0.2126, 0.7152, 0.0722));
    c = luma + 1.4 * (c - luma);
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
const vec3  SUN_DIR = normalize(vec3(0.2, 0.14, -1.0));    // low sun ahead of the camera, a little to the right
const float SUN_INTENSITY = 20.0;
const float WIND_ANGLE = 1.9;          // direction the waves travel, radians from +X toward +Z (camera looks down -Z)
const float SWELL_LENGTH = 34.0;       // longest wavelength, meters
const float SWELL_HEIGHT = 0.8;        // amplitude of the longest wave, meters
const float CHOP = 0.7;                // amplitude ratio between successive waves: higher = choppier
const float CREST_SHARPNESS = 2.4;     // 1 rolling swell .. 3 peaky crests
const float RIPPLE = 0.008;            // capillary ripple amplitude, meters
const float FOAM_AMOUNT = 1.0;
const vec3  WATER_DEEP = vec3(0.004, 0.02, 0.04);      // linear albedo of the deep body
const vec3  WATER_GLOW = vec3(0.05, 0.26, 0.25);       // color of sunlight scattered through thin crests
const float HAZE = 6.0;                // aerial perspective strength (multiplies sea-level air extinction)
const float CAMERA_HEIGHT = 3.0;       // meters above mean sea level
const float EXPOSURE = 0.9;
const float FOCAL = 2.0;

const float GRAVITY = 9.81;
const float WAVELENGTH_RATIO = 0.8;    // wavelength ratio between successive waves
const int   TRACE_WAVES = 12;          // waves in the traced surface
const int   SHADE_WAVES = 28;          // waves in the shading normal (adds fine chop)
const int   FOAM_WAVES = 6;            // waves whose crests make foam
const float TRACE_FAR = 400.0;         // beyond this, intersect the mean plane and rely on filtering

// Wave i of the sum: unit direction, wavenumber, amplitude and phase. Wavelengths fall geometrically from the swell;
// long waves run with the wind, short chop spreads wider around it.
void wave(int i, out vec2 dir, out float k, out float amp, out float phase) {
    float fi = float(i);
    vec3 h = hash32(vec2(fi, 17.0));
    float a = WIND_ANGLE + mix(0.35, 1.3, min(fi / 12.0, 1.0)) * (2.0 * h.x - 1.0);
    dir = vec2(cos(a), sin(a));
    k = TAU / (SWELL_LENGTH * pow(WAVELENGTH_RATIO, fi) * mix(0.85, 1.15, h.y));
    amp = SWELL_HEIGHT * pow(CHOP, fi);
    phase = TAU * h.z;
}

// Deep-water dispersion: a wave of wavenumber k moves at angular frequency sqrt(g k), so long swell outruns chop.
float wavePhase(vec2 dir, float k, float phase, vec2 xz) {
    return k * dot(dir, xz) - sqrt(GRAVITY * k) * iTime + phase;
}

// How much of a wave survives filtering: 1 while its wavelength spans 6+ pixel footprints, 0 below 2.
// A pixel covers an ellipse on the water: fp.zw = its size across and along the view, fp.xy = the unit view
// direction projected on the water. At grazing angles it is far longer along the view, which is where waves alias.
float waveKeep(vec2 dir, float k, vec4 fp) {
    float footprint = length(vec2(dot(dir, vec2(-fp.y, fp.x)) * fp.z, dot(dir, fp.xy) * fp.w));
    return smoothstep(2.0, 6.0, TAU / (k * footprint));
}

// Exp-sine profile: sharp crests, broad troughs. exp(s (sin - 1)) - 0.3 keeps the mean level near 0.
// Waves too small for the footprint are faded out, so distant geometry does not alias into streaks.
float oceanHeight(vec2 xz, int count, vec4 fp) {
    float h = 0.0;
    for (int i = ZERO; i < SHADE_WAVES; i++) {
        if (i >= count) break;
        vec2 dir; float k, amp, phase;
        wave(i, dir, k, amp, phase);
        h += waveKeep(dir, k, fp) * amp * (exp(CREST_SHARPNESS * (sin(wavePhase(dir, k, phase, xz)) - 1.0)) - 0.3);
    }
    return h;
}

// Steepest slope one wave of amplitude a and wavenumber k can reach: a k max|s cos(t) e^(s (sin(t) - 1))|.
float profileMaxSlope() {
    float s = CREST_SHARPNESS;
    float sn = (sqrt(1.0 + 4.0 * s * s) - 1.0) / (2.0 * s);
    return s * sqrt(1.0 - sn * sn) * exp(s * (sn - 1.0));
}

// Bounds of the traced surface: vec3(lowest height, highest height, steepest slope).
vec3 oceanBounds() {
    float ampSum = 0.0, slopeSum = 0.0;
    for (int i = ZERO; i < TRACE_WAVES; i++) {
        vec2 dir; float k, amp, phase;
        wave(i, dir, k, amp, phase);
        ampSum += amp;
        slopeSum += amp * k;
    }
    return vec3(-0.3 * ampSum, 0.7 * ampSum, slopeSum * profileMaxSlope());
}

// Ray vs the traced wave surface. The surface lies inside the slab bounds.x <= y <= bounds.y, so clip the ray to it,
// then step by the height gap over the fastest rate the gap can close (the ray's descent plus the steepest slope times
// its horizontal speed). The slope bound is relaxed by 0.6 for speed; a secant step refines the crossing.
// Returns vec2(t, steps); t < 0 when the ray reaches TRACE_FAR first.
vec2 traceOcean(vec3 ro, vec3 rd, vec3 bounds, vec4 pixelSpread) {
    if (rd.y >= 0.0) return vec2(-1.0, 0.0);
    float t = max((ro.y - bounds.y) / -rd.y, 0.0);
    float tEnd = min((ro.y - bounds.x) / -rd.y, TRACE_FAR);
    float closing = -rd.y + 0.6 * bounds.z * length(rd.xz);
    float tPrev = t, gapPrev = 1e10;
    for (int i = ZERO; i < 200; i++) {
        vec3 p = ro + rd * t;
        float gap = p.y - oceanHeight(p.xz, TRACE_WAVES, pixelSpread * vec4(1.0, 1.0, t, t));
        if (gap < 0.0) return vec2(mix(tPrev, t, gapPrev / (gapPrev - gap)), float(i));
        if (t >= tEnd) break;
        tPrev = t;
        gapPrev = gap;
        t = min(t + max(gap / closing, 0.004 * t), tEnd);
    }
    return vec2(-1.0, 200.0);
}

// Shading detail at xz: height, slope and Laplacian of all SHADE_WAVES, each wave faded out as its wavelength nears
// the pixel footprint. Returns vec4(height, dh/dx, dh/dz, laplacian). lostSlope2 collects the slope variance of the
// faded waves (it becomes roughness); crest is the convexity of the big waves, 1 where all their crests meet (foam).
vec4 oceanDetail(vec2 xz, vec4 fp, out float lostSlope2, out float crest) {
    const float slopeRms = 0.3 + 0.06 * CREST_SHARPNESS;   // RMS of the profile's slope per unit a k
    vec4 s = vec4(0.0);
    lostSlope2 = 0.0;
    crest = 0.0;
    float crestMax = 0.0;
    for (int i = ZERO; i < SHADE_WAVES; i++) {
        vec2 dir; float k, amp, phase;
        wave(i, dir, k, amp, phase);
        float keep = waveKeep(dir, k, fp);
        float theta = wavePhase(dir, k, phase, xz);
        float sn = sin(theta), cs = cos(theta);
        float e = exp(CREST_SHARPNESS * (sn - 1.0));
        float d1 = CREST_SHARPNESS * cs * e;                                      // dW/dtheta
        float d2 = CREST_SHARPNESS * (CREST_SHARPNESS * cs * cs - sn) * e;        // d2W/dtheta2
        s += keep * amp * vec4(e - 0.3, k * d1 * dir, k * k * d2);
        lostSlope2 += (1.0 - keep) * (amp * k * slopeRms) * (amp * k * slopeRms);
        if (i < FOAM_WAVES) {
            crest -= keep * amp * k * k * d2;
            crestMax += amp * k * k * CREST_SHARPNESS;   // a crest's curvature: -W'' = s at the top
        }
    }
    crest /= crestMax;
    return s;
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
    float pixelAngle = 2.0 / (iResolution.y * FOCAL);
    float viewKm = CAMERA_HEIGHT * 0.001;

#if DEBUG_MODE == 1
    return vec3(0.5 + 0.5 * oceanHeight(uv * 30.0, SHADE_WAVES, vec4(1.0, 0.0, 1e-4, 1e-4)) / SWELL_HEIGHT);
#endif

    vec3 ro = vec3(0.0, CAMERA_HEIGHT, 0.0);
    mat3 cam = cameraMatrix(ro, ro + vec3(0.0, -0.06, -1.0), 0.0);
    vec3 rd = cameraRay(cam, uv, FOCAL);

    vec3 sunColor = SUN_INTENSITY * 0.12 * sunTransmittance(viewKm, SUN_DIR);
    if (rd.y >= 0.0) {
        vec3 sky = atmosphereSky(rd, SUN_DIR, viewKm, SUN_INTENSITY);
        return sky + sunColor * 40.0 * sunDisk(rd, SUN_DIR, 0.012, pixelAngle);
    }

    // pixel footprint on the water per meter of distance: pixelAngle across the view, stretched by 1/sin(grazing
    // angle) along it
    vec4 pixelSpread = vec4(normalize(rd.xz), pixelAngle, pixelAngle / max(-rd.y, 0.01));
    vec3 bounds = oceanBounds();
    vec2 hit = traceOcean(ro, rd, bounds, pixelSpread);
#if DEBUG_MODE == 4
    return debugHeat(hit.y / 200.0);
#endif
    // past TRACE_FAR the waves are below pixel size anyway: use the mean plane and let filtering do the rest
    float t = (hit.x > 0.0) ? hit.x : ro.y / -rd.y;
    vec3 p = ro + rd * t;

    vec4 fp = pixelSpread * vec4(1.0, 1.0, t, t);
    float lostSlope2, crest;
    vec4 wv = oceanDetail(p.xz, fp, lostSlope2, crest);
    vec2 slope = wv.yz;

    // capillary ripples: two octaves of simplex flow noise whose gradients spin, so they churn instead of sliding
    float footprint = sqrt(fp.z * fp.w);   // ripples are isotropic: use the ellipse's mean size
    float keepRipple = smoothstep(2.0, 6.0, 0.8 / footprint);
    float keepFine = smoothstep(2.0, 6.0, 0.3 / footprint);
    if (keepRipple > 0.0) {
        vec3 r1 = simplexNoiseD(p.xz * 1.3 + vec2(0.3, 0.9) * iTime, 1.3 * iTime);
        slope += RIPPLE * keepRipple * 1.3 * r1.yz;
        if (keepFine > 0.0) {
            vec3 r2 = simplexNoiseD(p.xz * 3.7 - vec2(0.6, 0.2) * iTime, -2.1 * iTime);
            slope += RIPPLE * keepRipple * 0.5 * 3.7 * keepFine * r2.yz;
        }
    }
    lostSlope2 += RIPPLE * RIPPLE * 4.0 * (1.0 - keepRipple);
    vec3 n = normalize(vec3(-slope.x, 1.0, -slope.y));

    // foam: the convexity of the big waves' crests sets how much of a billow-noise lace is covered, so patches
    // grow from crisp filaments on mild crests to solid caps on the sharpest ones
    float coverage = 0.7 * smoothstep(0.08, 0.3, crest);
    float foam = 0.0;
    if (coverage > 0.0) {
        float lace = billowFbm(p.xz * vec2(1.2, 2.4) + vec2(0.0, 0.5 * iTime), 6);
        foam = FOAM_AMOUNT * coverage * smoothstep(1.0 - coverage, 1.15 - coverage, lace);
        float bubbleTone = 0.75;
        if (foam > 0.0 && keepFine > 0.0) {
            float border = voronoiBorder(p.xz * vec2(2.5, 4.0), 1.0).x;
            float bubbles = 1.0 - smoothstep(0.0, 0.22, border);
            bubbleTone = mix(bubbleTone, mix(0.4, 1.0, bubbles), keepFine);
        }
        foam *= bubbleTone;
    }

    vec3 v = -rd;
    float NoV = max(dot(n, v), 1e-3);
    vec3 F = fresnelSchlick(NoV, vec3(0.02));
    vec3 refl = reflect(rd, n);
    refl.y = abs(refl.y);   // a reflection into the water would hit the next wave: fold it back to the sky
    vec3 skyRefl = atmosphereSky(refl, SUN_DIR, viewKm, SUN_INTENSITY);
    vec3 skyAmbient = atmosphereSky(vec3(0.0, 1.0, 0.0), SUN_DIR, viewKm, SUN_INTENSITY);

    // sun glint: GGX whose roughness absorbs the slope variance of the waves filtered away at this distance
    float alpha = sqrt(0.02 * 0.02 + 2.0 * lostSlope2);
    vec3 glint = specularGGX(n, v, SUN_DIR, sqrt(alpha), vec3(0.02)) * sunColor;
    vec3 reflection = F * skyRefl;

    // body: refracted light scattered back out, plus sunlight glowing through thin crests seen against the sun
    float backlit = pow(clamp(dot(rd, SUN_DIR), 0.0, 1.0), 4.0);
    float thin = smoothstep(0.45, 0.9, (wv.x - bounds.x) / (bounds.y - bounds.x));   // crest tops only
    vec3 body = (1.0 - F) * (WATER_DEEP * (skyAmbient + sunColor * max(dot(n, SUN_DIR), 0.0))
                             + WATER_GLOW * sunColor * backlit * thin);

    // foam is a thin white scatterer: diffuse sun and sky light, plus sunlight shining through it from behind
    vec3 foamColor = vec3(0.9) * (sunColor * (max(dot(n, SUN_DIR), 0.0) + 0.5 * backlit) + 3.0 * skyAmbient);

#if DEBUG_MODE == 2
    return debugNormal(n);
#elif DEBUG_MODE == 3
    return vec3(foam, clamp(crest, 0.0, 1.0), lace);
#elif DEBUG_MODE == 5
    return sqrt(F);
#elif DEBUG_MODE == 6
    return vec3(luminance(reflection), luminance(glint) * 0.1, luminance(body) * 10.0);
#endif

    vec3 col = mix(reflection + glint + body, foamColor, foam);
    // aerial perspective, with the haze color taken from the sky just above the horizon so the two meet without a seam
    vec3 fogColor = atmosphereSky(normalize(vec3(rd.x, 0.0, rd.z)), SUN_DIR, viewKm, SUN_INTENSITY);
    vec3 airT = exp(-t * 0.001 * HAZE * atmosphereExtinction(atmosphereDensity(0.0)));
    return col * airT + fogColor * (1.0 - airT);
}

void mainImage(out vec4 fragColor, in vec2 fragCoord) {
    vec3 col = render(fragCoord);
#if DEBUG_MODE == 0
    col = linearToSrgb(tonemapAgXPunchy(col * EXPOSURE)) + ditherTPDF(fragCoord, iFrame);
#endif
    fragColor = vec4(col, 1.0);
}
