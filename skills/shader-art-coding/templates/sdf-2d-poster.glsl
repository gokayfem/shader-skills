// Flat-graphic poster: dusk sky, striped sun, layered ridges, foliage and stars, all from 2D distance fields.
// Shows: 2D SDF primitives, antialiasing with a fixed pixel size, Oklab gradients, polar repetition for petals,
// per-cell randomness, layered composition back-to-front, film grain, Neutral tonemap for palette fidelity.
// @render time=3.0

#define DEBUG_MODE 0   // 0 final | 1 distance field of the foliage layer | 5 layer masks

// ---- palette (sRGB picks, decoded to linear before use) ----
const vec3 SKY_TOP = vec3(0.13, 0.10, 0.30);
const vec3 SKY_HORIZON = vec3(0.98, 0.52, 0.36);
const vec3 SUN = vec3(1.0, 0.86, 0.55);
const vec3 RIDGE_FAR = vec3(0.55, 0.30, 0.42);
const vec3 RIDGE_NEAR = vec3(0.20, 0.12, 0.26);
const vec3 FOLIAGE = vec3(0.07, 0.05, 0.12);

// Ridge silhouette: y below the line is "inside". Returns an approximate signed distance (good enough for AA).
float ridge(vec2 uv, float base, float amp, float freq, float seed) {
    float h = base + amp * fbm(vec2(uv.x * freq + seed, seed), 5);
    return (uv.y - h) / sqrt(1.0 + 4.0 * amp * amp * freq * freq);   // divide by an approximate slope bound
}

// A palm-like plant: a curved trunk plus a fan of vesica leaves around its crown.
float plant(vec2 p) {
    float trunk = sdQuadraticBezier(p, vec2(0.0, -1.0), vec2(0.12, -0.45), vec2(0.02, 0.0)) - 0.025;
    vec2 q = p - vec2(0.02, 0.0);
    q = domPolar(q, 9.0);                 // 9 leaves, each pointing along +X in its sector
    q.y += 0.06 * q.x * q.x;              // droop
    float leaf = sdVesica((q - vec2(0.24, 0.0)).yx, 0.32, 0.27);
    return min(trunk, leaf);
}

void mainImage(out vec4 fragColor, in vec2 fragCoord) {
    vec2 uv = (2.0 * fragCoord - iResolution.xy) / iResolution.y;
    float px = 2.0 / iResolution.y;

    // sky: perceptual gradient
    vec3 col = mixOklab(srgbToLinear(SKY_HORIZON), srgbToLinear(SKY_TOP), smoothstep(-0.3, 1.0, uv.y));

    // stars: one random star per grid cell, fading toward the horizon
    vec2 cell = floor(uv * 18.0);
    vec3 rnd = hash32(cell);
    float star = sdCircle(uv * 18.0 - cell - 0.2 - 0.6 * rnd.xy, 0.02 + 0.05 * rnd.z * rnd.z) / 18.0;
    float twinkle = 0.6 + 0.4 * sin(iTime * (1.0 + 3.0 * rnd.z) + 6.0 * rnd.x);
    col += vec3(1.0, 0.95, 0.9) * fillAA(star, px) * smoothstep(0.1, 0.8, uv.y) * twinkle;

    // sun with retro gaps that thicken toward its bottom edge
    vec2 sp = uv - vec2(0.42, 0.18);
    float sun = sdCircle(sp, 0.36);
    float gaps = filteredStripes(sp.y * 14.0 + 0.5, px * 14.0);
    float gapMask = mix(1.0, gaps, (1.0 - smoothstep(-0.3, 0.0, sp.y)));
    col = mix(col, srgbToLinear(SUN) * 1.6, fillAA(sun, px) * gapMask);
    col += srgbToLinear(SUN) * 0.25 * exp(-6.0 * max(sun, 0.0));   // glow

    // ridges, back to front
    col = mix(col, srgbToLinear(RIDGE_FAR), fillAA(ridge(uv, -0.1, 0.18, 1.2, 3.0), px));
    col = mix(col, mixOklab(srgbToLinear(RIDGE_FAR), srgbToLinear(RIDGE_NEAR), 0.6),
              fillAA(ridge(uv, -0.32, 0.14, 1.8, 11.0), px));
    col = mix(col, srgbToLinear(RIDGE_NEAR), fillAA(ridge(uv, -0.55, 0.10, 2.6, 23.0), px));

    // foreground plants
    float plants = min(plant((uv - vec2(-0.95, 0.15)) / 0.9) * 0.9, plant((uv - vec2(1.25, -0.05)) * vec2(-1.0, 1.0) / 0.7) * 0.7);
    col = mix(col, srgbToLinear(FOLIAGE), fillAA(plants, px));

#if DEBUG_MODE == 1
    fragColor = vec4(debugDistance(plants), 1.0);
    return;
#elif DEBUG_MODE == 5
    fragColor = vec4(fillAA(sun, px), fillAA(ridge(uv, -0.1, 0.18, 1.2, 3.0), px), fillAA(plants, px), 1.0);
    return;
#endif

    // finish: grain in linear, palette-faithful tonemap, encode, dither
    col = filmGrain(col, 0.04, hash13(vec3(fragCoord, iFrame)) * 2.0 - 1.0);
    col = linearToSrgb(tonemapNeutral(col)) + ditherTPDF(fragCoord, iFrame);
    fragColor = vec4(col, 1.0);
}
