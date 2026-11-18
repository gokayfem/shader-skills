//! noise: value, gradient and simplex noise (2D/3D, analytic derivatives), fBm, turbulence and billow, domain warping,
//! curl flow, Voronoi, dithering.
//! Every function here is texture-free and original. The math follows the cited articles.
//! Ranges are approximate unless stated. Noise built on integer hashes stays stable at large coordinates
//! until float precision runs out (|p| around 1e5). Re-center long animations with mod/fract on time.

/// Quintic fade 6t^5 - 15t^4 + 10t^3: C2-continuous interpolation weights for lattice noise.
/// @use custom lattice noises, smooth cell interpolation
vec2 quinticFade(vec2 f) { return f * f * f * (f * (f * 6.0 - 15.0) + 10.0); }
vec3 quinticFade(vec3 f) { return f * f * f * (f * (f * 6.0 - 15.0) + 10.0); }

/// Derivative of quinticFade: 30 t^2 (t - 1)^2.
/// @use analytic noise derivatives
vec2 quinticFadeDeriv(vec2 f) { return 30.0 * f * f * (f * (f - 2.0) + 1.0); }
vec3 quinticFadeDeriv(vec3 f) { return 30.0 * f * f * (f * (f - 2.0) + 1.0); }

/// 2D value noise with analytic derivatives. Returns vec3(value, d/dx, d/dy).
/// @use terrain base layers, cheap clouds, anything needing normals without finite differences
/// @range value in [0,1]
/// @see https://iquilezles.org/articles/morenoise/
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

/// 2D value noise.
/// @use soft blotches, low-frequency color variation
/// @range [0,1]
float valueNoise(vec2 p) { return valueNoiseD(p).x; }

/// 3D value noise.
/// @use volumetric density, animated 2D noise (use z = time)
/// @range [0,1]
float valueNoise(vec3 p) {
    vec3 i = floor(p), f = fract(p);
    vec3 u = quinticFade(f);
    return mix(mix(mix(hash13(i + vec3(0, 0, 0)), hash13(i + vec3(1, 0, 0)), u.x),
                   mix(hash13(i + vec3(0, 1, 0)), hash13(i + vec3(1, 1, 0)), u.x), u.y),
               mix(mix(hash13(i + vec3(0, 0, 1)), hash13(i + vec3(1, 0, 1)), u.x),
                   mix(hash13(i + vec3(0, 1, 1)), hash13(i + vec3(1, 1, 1)), u.x), u.y), u.z);
}

/// Unit gradient for a 2D lattice point.
vec2 latticeGradient(vec2 cell) {
    float a = TAU * hash12(cell);
    return vec2(cos(a), sin(a));
}

/// 2D gradient (Perlin-style) noise with analytic derivatives. Returns vec3(value, d/dx, d/dy).
/// Gradient noise has less grid-aligned blockiness than value noise. Prefer it for fBm.
/// @use fBm, terrain with analytic normals, curl flow, marble and wood distortions
/// @range value in about [-0.7,0.7]
/// @see https://iquilezles.org/articles/gradientnoise/
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

/// 2D gradient noise.
/// @use general-purpose smooth noise
/// @range about [-0.7,0.7]
float gradientNoise(vec2 p) { return gradientNoiseD(p).x; }

/// 2D simplex noise with analytic gradient. Returns vec3(value, d/dx, d/dy).
/// Sums three radial kernels (0.5 - r^2)^4 on a triangular lattice instead of blending four square-cell corners, so it
/// has fewer axis-aligned artifacts than gradientNoise. rotation (radians) spins every lattice gradient: animating it
/// gives flow noise that churns in place instead of sliding (Perlin and Neyret 2001). Scaled so |value| <= 1 exactly.
/// @use flowing water and lava (animate rotation), ripples with analytic normals, isotropic fBm, swirling fields
/// @range value in [-1,1]
/// @see Gustavson, "Simplex noise demystified" (2005); Gustavson and McEwan, JCGT 11(1), 2022 (psrdnoise)
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

/// 2D simplex noise.
/// @use isotropic smooth noise, fBm layers without grid alignment
/// @range [-1,1]
float simplexNoise(vec2 p) { return simplexNoiseD(p, 0.0).x; }

/// Dot product of a hashed 3D gradient with the offset to its lattice corner.
float latticeGradDot(vec3 cell, vec3 corner, vec3 f) {
    vec3 g = hash33(cell + corner) * 2.0 - 1.0;
    return dot(g, f - corner);
}

/// 3D gradient noise.
/// @use clouds, smoke, solid textures (marble, wood), animated 2D noise (z = time)
/// @range about [-1,1]
/// @see https://iquilezles.org/articles/gradientnoise/
float gradientNoise(vec3 p) {
    vec3 i = floor(p), f = fract(p);
    vec3 u = quinticFade(f);
    return mix(mix(mix(latticeGradDot(i, vec3(0, 0, 0), f), latticeGradDot(i, vec3(1, 0, 0), f), u.x),
                   mix(latticeGradDot(i, vec3(0, 1, 0), f), latticeGradDot(i, vec3(1, 1, 0), f), u.x), u.y),
               mix(mix(latticeGradDot(i, vec3(0, 0, 1), f), latticeGradDot(i, vec3(1, 0, 1), f), u.x),
                   mix(latticeGradDot(i, vec3(0, 1, 1), f), latticeGradDot(i, vec3(1, 1, 1), f), u.x), u.y), u.z);
}

/// 3D gradient noise with analytic derivatives. Returns vec4(value, d/dx, d/dy, d/dz); value equals gradientNoise(p).
/// The trilinear blend is expanded as a polynomial in the fade weights, so the gradient is the same blend of the corner
/// gradients plus the chain-rule term of the fade.
/// @use normals of noise-displaced surfaces without extra samples, 3D curl flow, slope-aware 3D fBm, lit nebulae
/// @range value in about [-1,1]
/// @see https://iquilezles.org/articles/gradientnoise/
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

/// Fractional Brownian motion over 2D gradient noise. Each octave is rotated to hide lattice alignment.
/// persistence 0.5 is the classic look (H = 1). Lower is smoother, higher is rougher.
/// @use clouds, terrain, dirt, roughness breakup, warping input
/// @range about [-1,1]
/// @see https://iquilezles.org/articles/fbm/
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

/// 2D fBm with the usual defaults (lacunarity 2.01 so octave peaks never re-align, persistence 0.5).
/// @use quick organic variation
/// @range about [-1,1]
float fbm(vec2 p, int octaves) { return fbm(p, octaves, 2.01, 0.5); }

/// Screen-filtered 2D fBm with the same spectrum as fbm(p, octaves). Pass the pixel width in input lattice cells.
/// Each octave fades between 4 and 2 pixels per cell. Unresolved bands contribute their mean (zero).
/// @use distant rock grain, mineral strata, terrain snow masks, moving procedural materials
/// @see https://pbr-book.org/3ed-2018/Texture/Noise (footprint-based octave clamping)
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

/// fBm over 3D gradient noise.
/// @use clouds, smoke, nebulae, solid marble
/// @range about [-1,1]
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

/// Terrain fBm where each octave is damped by the accumulated slope, so detail settles in valleys
/// and steep slopes stay smooth. This gives an eroded look without simulation.
/// @use mountains, dunes, rocky ground
/// @range about [-0.5,1]
/// @see https://iquilezles.org/articles/morenoise/
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

/// Ridged multifractal: inverted |noise| creates sharp crests; each octave is weighted by the previous one,
/// so detail gathers along ridges.
/// @use mountain ridges, lightning-like veins, canyons, eroded rock, alien landscapes
/// @range about [0,1]
float ridgedFbm(vec2 p, int octaves) {
    const mat2 octaveRotation = mat2(0.8, 0.6, -0.6, 0.8);
    float sum = 0.0, amp = 0.5, weight = 1.0, norm = 0.0;
    for (int i = 0; i < FBM_MAX_OCTAVES; i++) {
        if (i >= octaves) break;
        float n = 1.0 - abs(gradientNoise(p) * 1.4);
        n *= n;
        sum += amp * n * weight;
        norm += amp;
        weight = clamp(n * 2.0, 0.0, 1.0);
        amp *= 0.5;
        p = 2.01 * (octaveRotation * p) + vec2(17.13, -3.71);
    }
    return sum / norm;
}

/// Perlin's turbulence: fBm of |noise|. The absolute value folds every zero crossing into a sharp crease, at every scale.
/// @use fire, marble veins (sin(x + k * turbulence)), smoke wisps, electric glow, lava cracks, wind-torn clouds
/// @range [0,1]
/// @see Perlin, "An Image Synthesizer", SIGGRAPH 1985
float turbulence(vec2 p, int octaves) {
    const mat2 octaveRotation = mat2(0.8, 0.6, -0.6, 0.8);
    float sum = 0.0, amp = 1.0, norm = 0.0;
    for (int i = 0; i < FBM_MAX_OCTAVES; i++) {
        if (i >= octaves) break;
        sum += amp * abs(gradientNoise(p));
        norm += amp;
        amp *= 0.5;
        p = 2.01 * (octaveRotation * p) + vec2(17.13, -3.71);
    }
    return 1.4 * sum / norm;   // |2D gradient noise| <= sqrt(2)/2
}
float turbulence(vec3 p, int octaves) {
    const mat3 octaveRotation = mat3(0.00, 0.80, 0.60,
                                     -0.80, 0.36, -0.48,
                                     -0.60, -0.48, 0.64);
    float sum = 0.0, amp = 1.0, norm = 0.0;
    for (int i = 0; i < FBM_MAX_OCTAVES; i++) {
        if (i >= octaves) break;
        sum += amp * min(abs(gradientNoise(p)), 1.0);
        norm += amp;
        amp *= 0.5;
        p = 2.01 * (octaveRotation * p) + vec3(17.13, -3.71, 5.29);
    }
    return sum / norm;
}

/// Billowy multifractal: |noise| per octave makes puffy rounded lobes split by sharp creases (the inverse of ridgedFbm).
/// Each octave is weighted by the previous one, so detail gathers on the lobes while the creases stay clean.
/// @use cumulus and cauliflower tops, puffy smoke, foam clumps, rock piles, cotton, popcorn-like bumps
/// @range [0,1]
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

/// Two-level domain warp f(p + s*r), r = fbm(p + s*q), q = fbm(p). Writes the intermediate warp
/// vectors so they can drive color. Cost is 5 fBm calls.
/// @use marble, smoke, agate, alien skies, painterly backgrounds
/// @range about [-1,1]
/// @see https://iquilezles.org/articles/warp/
float warpedFbm(vec2 p, float strength, int octaves, out vec2 q, out vec2 r) {
    q = vec2(fbm(p, octaves), fbm(p + vec2(3.1, 7.4), octaves));
    r = vec2(fbm(p + strength * q + vec2(9.7, 2.6), octaves),
             fbm(p + strength * q + vec2(6.3, 1.9), octaves));
    return fbm(p + strength * r, octaves);
}

/// Divergence-free 2D flow field: the curl of scalar gradient noise.
/// Particles advected by it swirl without bunching up.
/// @use smoke advection, flowing paint, hair/fur flow direction, wind
vec2 curlNoise(vec2 p) {
    vec3 n = gradientNoiseD(p);
    return vec2(n.z, -n.y);
}

/// Divergence-free 3D flow field: the curl of a vector potential whose components are three decorrelated
/// gradientNoiseD fields. Analytic, so it costs three noise evaluations and no finite differences.
/// @use smoke and fire advection, swirling particles and sparks, wisps, 3D flow-aligned strokes, magic trails
/// @see Bridson, Hourihan and Nordenstam, "Curl-Noise for Procedural Fluid Flow", SIGGRAPH 2007
vec3 curlNoise(vec3 p) {
    vec3 a = gradientNoiseD(p).yzw;                                   // grad of potential.x
    vec3 b = gradientNoiseD(p + vec3(31.416, -47.853, 12.793)).yzw;   // grad of potential.y
    vec3 c = gradientNoiseD(p + vec3(-23.719, 18.274, -56.183)).yzw;  // grad of potential.z
    return vec3(c.y - b.z, a.z - c.x, b.x - a.y);
}

/// Cellular (Worley/Voronoi) noise. Returns vec4(F1, F2, nearestCell.xy).
/// F1 = distance to nearest feature point, F2 = second nearest. nearestCell is integer-valued, so hash it for
/// per-cell randomness. jitter in [0,1] (0 = regular grid). Searches 3x3 cells. F2 can be slightly off at jitter 1.
/// @use cells, scales, stones, cracked mud (F2-F1), caustics, foam, sparkles
/// @range F1 in [0, ~1.2]
/// @see https://iquilezles.org/articles/voronoilines/
vec4 voronoi(vec2 p, float jitter) {
    vec2 ip = floor(p), fp = fract(p);
    float f1 = 8.0, f2 = 8.0;
    vec2 nearest = vec2(0.0);
    for (int j = -1; j <= 1; j++)
    for (int i = -1; i <= 1; i++) {
        vec2 g = vec2(float(i), float(j));
        vec2 r = g + jitter * hash22(ip + g) - fp;
        float d = dot(r, r);
        if (d < f1) { f2 = f1; f1 = d; nearest = ip + g; }
        else if (d < f2) { f2 = d; }
    }
    return vec4(sqrt(f1), sqrt(f2), nearest);
}

/// Exact distance to Voronoi cell borders, found in two passes: nearest point, then nearest bisector.
/// Returns vec3(borderDistance, vectorToNearestPoint). Unlike F2-F1, the border distance is a true distance,
/// so lines have constant width.
/// @use cell walls, stained glass leading, cracked earth, giraffe and leaf-vein patterns, crackle glaze
/// @see https://iquilezles.org/articles/voronoilines/
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

/// Smooth Voronoi: a soft-min of distances to feature points. Cells become blobs with no creases.
/// k controls smoothness (large = sharper; around 8-32). Searches 5x5 cells so low k has no seams.
/// @use organic cell clusters, bubbles, lumpy surfaces, skin pores
/// @see https://iquilezles.org/articles/smoothvoronoi/
float smoothVoronoi(vec2 p, float k) {
    vec2 ip = floor(p), fp = fract(p);
    float acc = 0.0;
    for (int j = -2; j <= 2; j++)
    for (int i = -2; i <= 2; i++) {
        vec2 g = vec2(float(i), float(j));
        vec2 r = g + hash22(ip + g) - fp;
        acc += exp2(-k * length(r));
    }
    return -log2(acc) / k;
}

/// Bounded hybrid terrain recipe: broad gradient fBm, warped ridged detail and cellular breakup.
/// This is an artistic composition, not Musgrave's named hybrid-multifractal algorithm. Coordinates are
/// warped before all three fields are sampled; octaves is clamped to [1, FBM_MAX_OCTAVES].
/// @use game heightmaps, rocky islands, biome elevation masks, alien terrain
/// @range [0,1]
/// @see https://thebookofshaders.com/13/ (octaves, ridges and warping); https://thebookofshaders.com/12/ (cells)
float hybridTerrainHeight(vec2 p, int octaves) {
    int count = clamp(octaves, 1, FBM_MAX_OCTAVES);
    vec2 warp = vec2(fbm(p * 0.35, count), fbm(p * 0.35 + vec2(19.1, -7.3), count));
    vec2 q = p + 0.8 * warp;
    float broad = 0.5 + 0.5 * fbm(q * 0.45, count);
    float ridges = ridgedFbm(q * 1.7, count);
    float cells = 1.0 - clamp(voronoi(q * 2.5, 0.75).x, 0.0, 1.0);
    return clamp(0.45 * broad + 0.45 * ridges + 0.10 * cells, 0.0, 1.0);
}

/// Interleaved gradient noise: cheap per-pixel noise whose 3x3 neighborhoods are nearly stratified. Not a hash.
/// pixel must be integer pixel coordinates (floor(fragCoord)). The frame overload animates it for accumulation and TAA.
/// @use removing banding before 8-bit output, jittering ray start offsets, stochastic transparency
/// @range [0,1)
/// @see Jimenez 2014, "Next Generation Post Processing in Call of Duty: Advanced Warfare"
float interleavedGradientNoise(vec2 pixel) {
    return fract(52.9829189 * fract(dot(pixel, vec2(0.06711056, 0.00583715))));
}
float interleavedGradientNoise(vec2 pixel, int frame) {
    return interleavedGradientNoise(pixel + 5.588238 * float(frame % 64));
}

/// R2 low-discrepancy dither mask (plastic-constant sequence), folded by a triangle wave for an even spread.
/// @use ordered dithering without textures, posterized and retro looks, sample offsets
/// @range [0,1)
/// @see https://extremelearning.com.au/unreasonable-effectiveness-of-quasirandom-sequences/
float r2Dither(vec2 pixel) {
    float v = fract(dot(pixel, vec2(0.7548776662, 0.5698402910)));
    return v < 0.5 ? 2.0 * v : 2.0 - 2.0 * v;
}
