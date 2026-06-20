# Formula-Code Couples: Heightfields, Density Fields, and Atmosphere

Use this file when the task needs equation-to-code translation for heightfield synthesis, stratified material fields, instanced density fields, accumulation masks, atmosphere, coverage fields, and heightfield rendering.

Each couple has:

- Equation: the compact mathematical idea.
- GLSL: an original Shadertoy-friendly implementation.
- Visual result: what should appear when the formula is correct.
- Debug view: the simplest way to inspect it.

## 1. Smooth Cell Interpolation

Equation:

```text
i = floor(p)
f = fract(p)
s(f) = f^2(3 - 2f) or f^3(6f^2 - 15f + 10)
h = bilerp(h00, h10, h01, h11, s)
```

GLSL:

```glsl
float hash12(vec2 p) {
    vec3 p3 = fract(vec3(p.xyx) * 0.1031);
    p3 += dot(p3, p3.yzx + 33.33);
    return fract((p3.x + p3.y) * p3.z);
}

float smoothCell(vec2 p) {
    vec2 i = floor(p);
    vec2 f = fract(p);
    vec2 s = f*f*(3.0 - 2.0*f);
    float h00 = hash12(i + vec2(0.0, 0.0));
    float h10 = hash12(i + vec2(1.0, 0.0));
    float h01 = hash12(i + vec2(0.0, 1.0));
    float h11 = hash12(i + vec2(1.0, 1.0));
    return mix(mix(h00, h10, s.x), mix(h01, h11, s.x), s.y);
}
```

Visual result:

A smooth quilt of hills. No hard cell borders, but the cell structure may still be visible at low frequency.

Debug view:

```glsl
vec3 debugSmoothCell(vec2 p) {
    return vec3(smoothCell(p));
}
```

## 2. Quintic Interpolation for Better Normals

Equation:

```text
s(f) = f^3( f(6f - 15) + 10 )
```

GLSL:

```glsl
vec2 quintic(vec2 f) {
    return f*f*f*(f*(f*6.0 - 15.0) + 10.0);
}

float smoothCellQuintic(vec2 p) {
    vec2 i = floor(p);
    vec2 f = fract(p);
    vec2 s = quintic(f);
    float h00 = hash12(i + vec2(0.0, 0.0));
    float h10 = hash12(i + vec2(1.0, 0.0));
    float h01 = hash12(i + vec2(0.0, 1.0));
    float h11 = hash12(i + vec2(1.0, 1.0));
    return mix(mix(h00, h10, s.x), mix(h01, h11, s.x), s.y);
}
```

Visual result:

Smoother heightfield lighting near cell borders. Normals should not show obvious seams.

Debug view:

Show normals from finite differences; seams mean interpolation or sampling is wrong.

## 3. Polynomial Patch Terrain

Equation:

```text
f_ij(x,z) = a + bx + cz + d xz + e x^2 + f z^2 + ...
```

GLSL:

```glsl
float patchPoly(vec2 f, vec4 coeffA, vec4 coeffB) {
    float x = f.x;
    float z = f.y;
    return coeffA.x
         + coeffA.y*x
         + coeffA.z*z
         + coeffA.w*x*z
         + coeffB.x*x*x
         + coeffB.y*z*z
         + coeffB.z*x*x*z
         + coeffB.w*x*z*z;
}
```

Visual result:

A cell-local scalar surface that can make ridges, basins, and slopes with more control than simple value noise.

Debug view:

Show one patch in isolation as a height color before layering many cells.

## 4. Rotated Octave Matrix

Equation:

```text
F(p) = sum_n a_n N(M^n p)
a_n = 2^-n
```

GLSL:

```glsl
float fbmTerrain(vec2 p) {
    mat2 m = mat2(0.8, -0.6, 0.6, 0.8) * 2.03;
    float h = 0.0;
    float a = 0.55;
    for (int i = 0; i < 7; i++) {
        h += a * smoothCellQuintic(p);
        p = m * p + 17.3;
        a *= 0.5;
    }
    return h;
}
```

Visual result:

Fractal mountain detail with fewer grid-aligned artifacts. Large forms remain if a separate base shape is added.

Debug view:

```glsl
vec3 debugOctaves(vec2 p) {
    return vec3(fbmTerrain(p));
}
```

## 5. Base Shape Plus Detail

Equation:

```text
h(p) = H_base(p) + A_detail F(p)
```

GLSL:

```glsl
float terrainHeight(vec2 p) {
    float ridge = 0.9*exp(-0.18*abs(p.x + 0.35*p.y));
    float basin = -0.55*exp(-0.08*dot(p - vec2(1.5, -2.0), p - vec2(1.5, -2.0)));
    float detail = fbmTerrain(p*0.45);
    return ridge + basin + 0.38*detail;
}
```

Visual result:

Readable low-frequency height composition with fractal detail. The field should not become an even noisy blanket.

Debug view:

Show `ridge + basin` alone, then `detail` alone, then the combined height.

## 6. Heightfield Normal

Equation:

```text
n = normalize( (-dh/dx, eps, -dh/dz) )
```

GLSL:

```glsl
vec3 terrainNormal(vec2 p) {
    float e = 0.02;
    float h = terrainHeight(p);
    float hx = terrainHeight(p + vec2(e, 0.0)) - h;
    float hz = terrainHeight(p + vec2(0.0, e)) - h;
    return normalize(vec3(-hx, e, -hz));
}
```

Visual result:

The heightfield responds to light as a continuous surface. Steep ridges and basins should be visible under diffuse light.

Debug view:

```glsl
vec3 debugTerrainNormal(vec2 p) {
    return 0.5 + 0.5*terrainNormal(p);
}
```

## 7. Heightfield Ray March

Equation:

```text
find t where ro_y + t rd_y = h(ro_xz + t rd_xz)
```

GLSL:

```glsl
float marchTerrain(vec3 ro, vec3 rd) {
    float t = 0.0;
    float lastDiff = 0.0;
    for (int i = 0; i < 180; i++) {
        vec3 p = ro + rd*t;
        float diff = p.y - terrainHeight(p.xz);
        if (diff < 0.0) {
            float lo = t - max(0.02, 0.5*lastDiff);
            float hi = t;
            for (int j = 0; j < 5; j++) {
                float mid = 0.5*(lo + hi);
                vec3 mp = ro + rd*mid;
                if (mp.y > terrainHeight(mp.xz)) lo = mid; else hi = mid;
            }
            return hi;
        }
        lastDiff = diff;
        t += max(0.03, 0.35*diff);
        if (t > 220.0) break;
    }
    return -1.0;
}
```

Visual result:

A heightfield surface rendered from a camera without polygon geometry.

Debug view:

Show hit distance as fog-like grayscale; missing/black holes mean step size is too large or the heightfield slope is too steep.

## 8. Diffuse Terrain Lighting

Equation:

```text
L_diff = max(dot(n, l), 0)
```

GLSL:

```glsl
vec3 terrainDiffuse(vec3 p, vec3 n, vec3 lightDir) {
    float ndl = max(dot(n, lightDir), 0.0);
    vec3 base = mix(vec3(0.22, 0.18, 0.14), vec3(0.52, 0.48, 0.38), smoothstep(0.0, 1.2, p.y));
    return base * (0.18 + 1.35*ndl);
}
```

Visual result:

Surfaces facing the dominant light are bright; back-facing slopes fall into cool shadow.

Debug view:

Show `vec3(ndl)` before mixing material colors.

## 9. Heightfield Self Shadow

Equation:

```text
shadow = min_t smoothstep(0, k t, rayHeight(t) - terrainHeight(xz + t l_xz))
```

GLSL:

```glsl
float terrainShadow(vec2 xz, float h, vec3 lightDir) {
    float sh = 1.0;
    float t = 0.08;
    for (int i = 0; i < 48; i++) {
        vec2 q = xz + lightDir.xz*t;
        float rayH = h + lightDir.y*t;
        float clearance = rayH - terrainHeight(q);
        sh = min(sh, smoothstep(0.0, 0.18*t, clearance));
        t += 0.15 + 0.08*t;
    }
    return clamp(sh, 0.0, 1.0);
}
```

Visual result:

Elevated heightfield regions cast broad readable shadows over lower regions. The effect should create scale and depth.

Debug view:

Show `vec3(shadow)` only. Harsh flicker means too few steps or thresholds too tight.

## 10. Slope Mask

Equation:

```text
slope = 1 - n_y
mask = smoothstep(a, b, slope)
```

GLSL:

```glsl
float slopeMask(vec3 n) {
    float slope = 1.0 - n.y;
    return smoothstep(0.18, 0.65, slope);
}
```

Visual result:

Steep slopes can select channel B; flatter surfaces can select channel A or channel C.

Debug view:

Show `slopeMask(n)` as grayscale before material mixing.

## 11. Altitude Mask

Equation:

```text
mask = smoothstep(low, high, h)
```

GLSL:

```glsl
float altitudeMask(float h) {
    return smoothstep(0.65, 1.6, h);
}
```

Visual result:

A height-gated material, haze, or accumulation channel appears only above a chosen scalar threshold.

Debug view:

Show the mask alone and move the thresholds until it reads compositionally.

## 12. Stratified Three-Channel Material Mix

Equation:

```text
channelB = slope
channelC = altitude * upward
channelA = (1-channelB)(1-channelC)
color = channelA Ca + channelB Cb + channelC Cc
```

GLSL:

```glsl
vec3 terrainMaterial(vec3 p, vec3 n) {
    float slope = slopeMask(n);
    float high = altitudeMask(p.y);
    float channelC = high * smoothstep(0.45, 0.9, n.y);
    float channelA = (1.0 - slope) * (1.0 - channelC);
    vec3 channelACol = vec3(0.18, 0.34, 0.12);
    vec3 channelBCol = vec3(0.38, 0.34, 0.30);
    vec3 channelCCol = vec3(0.86, 0.90, 0.94);
    vec3 col = channelACol*channelA + channelBCol*slope*(1.0 - channelC) + channelCCol*channelC;
    return col / max(0.001, channelA + slope*(1.0 - channelC) + channelC);
}
```

Visual result:

Natural material zoning: one channel on gentle slopes, one on steep slopes, and one on high/upward-facing regions.

Debug view:

Show the three material masks as RGB channels.

## 13. Atmospheric Fog

Equation:

```text
f = 1 - exp(-density * distance)
color = mix(surface, fogColor, f)
```

GLSL:

```glsl
vec3 applyFog(vec3 col, vec3 fogCol, float dist) {
    float f = 1.0 - exp(-0.022*dist);
    return mix(col, fogCol, clamp(f, 0.0, 1.0));
}
```

Visual result:

Distant surfaces fade into atmospheric color, giving depth and scale.

Debug view:

Show `vec3(f)` to verify the fog curve before coloring.

## 14. Height Fog

Equation:

```text
fog *= exp(-falloff * y)
```

GLSL:

```glsl
float heightFog(float dist, float y) {
    float base = 1.0 - exp(-0.018*dist);
    float low = exp(-0.65*max(y, 0.0));
    return clamp(base * low, 0.0, 1.0);
}
```

Visual result:

Low-altitude regions hold more haze than exposed high regions.

Debug view:

Render haze as white over black; low-altitude basins should glow softly.

## 15. Sky Gradient

Equation:

```text
t = smoothstep(y0, y1, rd_y)
sky = mix(horizon, zenith, t)
```

GLSL:

```glsl
vec3 skyColor(vec3 rd, vec3 sunDir) {
    float t = smoothstep(-0.15, 0.65, rd.y);
    vec3 horizon = vec3(0.78, 0.86, 0.94);
    vec3 zenith = vec3(0.20, 0.45, 0.78);
    float sun = pow(max(dot(rd, sunDir), 0.0), 450.0);
    return mix(horizon, zenith, t) + sun*vec3(1.0, 0.75, 0.42);
}
```

Visual result:

A readable background radiance field with a warm directional accent and cooler zenith.

Debug view:

Render sky alone before terrain.

## 16. Atmospheric Coverage Field

Equation:

```text
coverage = smoothstep(c, c+w, fbm(p + warp(p)))
```

GLSL:

```glsl
float coverageMask(vec2 p, float coverage) {
    vec2 warp = vec2(fbmTerrain(p*0.18 + 4.1), fbmTerrain(p*0.18 - 2.8));
    float d = fbmTerrain(p + 0.55*warp);
    return smoothstep(coverage, coverage + 0.18, d);
}
```

Visual result:

Soft coverage-field masses with natural breakup. Big masses should exist before high-frequency details.

Debug view:

Show the coverage mask as grayscale over the background field.

## 17. Coverage Horizon Fade

Equation:

```text
coverage *= smoothstep(horizonLow, horizonHigh, y)
```

GLSL:

```glsl
float coverageHorizonFade(vec2 uv) {
    return smoothstep(-0.08, 0.22, uv.y);
}
```

Visual result:

Coverage-field contrast fades near the horizon so the background supports depth.

Debug view:

Show the fade gradient alone.

## 18. Directional Coverage Lighting

Equation:

```text
lit = coverage * (ambient + sunTint * phase)
```

GLSL:

```glsl
vec3 shadeCloud(float mask, vec2 p, vec2 sunDir) {
    float edge = coverageMask(p + 0.05*sunDir, 0.52) - mask;
    float rim = smoothstep(0.0, 0.18, edge);
    return mask * mix(vec3(0.78, 0.82, 0.88), vec3(1.0, 0.86, 0.62), rim);
}
```

Visual result:

Coverage fields have warm highlights on the light-facing side and cooler shaded interiors.

Debug view:

Show `rim` as grayscale to ensure it is directional.

## 19. Instance Placement Grid

Equation:

```text
cell = floor(xz / spacing)
local = fract(xz / spacing) - 1/2
rand = hash(cell)
```

GLSL:

```glsl
vec2 instanceCell(vec2 xz, float spacing, out vec2 id) {
    vec2 g = xz / spacing;
    id = floor(g);
    return fract(g) - 0.5;
}
```

Visual result:

Stable instance positions in world space. Instances do not swim as the camera moves.

Debug view:

Render cell IDs as color.

## 20. Instance Suitability Field

Equation:

```text
density = altitude * flatness * moisture * distanceFade
```

GLSL:

```glsl
float instanceSuitability(float height, float slope, float moisture, float dist) {
    float altitude = smoothstep(0.05, 0.35, height) * smoothstep(1.0, 0.65, height);
    float flatness = smoothstep(0.85, 0.35, slope);
    float distanceFade = smoothstep(160.0, 20.0, dist);
    return altitude * flatness * moisture * distanceFade;
}
```

Visual result:

Instances appear in controlled zones rather than covering every surface.

Debug view:

Show suitability as a heightfield overlay.

## 21. Volumetric Clump Density

Equation:

```text
density = crown(xz) * vertical(y) * breakup(xz)
```

GLSL:

```glsl
float clumpDensity(vec3 p, vec2 cellId) {
    float crown = 1.0 - smoothstep(0.35, 0.95, length(p.xz));
    float vertical = smoothstep(-0.3, 0.15, p.y) * smoothstep(1.1, 0.45, p.y);
    float breakup = 0.65 + 0.35*fbmTerrain(p.xz*3.5 + cellId);
    return crown * vertical * breakup;
}
```

Visual result:

Instanced details read as soft volumes or clumps rather than isolated dots.

Debug view:

Show density as white alpha over the heightfield.

## 22. Upward-Facing Accumulation Mask

Equation:

```text
accumulation = upwardNormal * altitude + noise
```

GLSL:

```glsl
float upwardAccumulation(vec3 n, float height, float noiseValue) {
    float upward = smoothstep(0.35, 0.9, n.y);
    float altitude = smoothstep(-0.2, 0.6, height);
    return clamp(upward * altitude + 0.18*noiseValue, 0.0, 1.0);
}
```

Visual result:

The accumulation material collects on upward-facing and elevated surfaces, not uniformly on every face.

Debug view:

Show the accumulation mask as grayscale before mixing the bright material.

## 23. Layered Sinusoidal Ridge

Equation:

```text
d = |x - sum_i A_i sin(w_i y + phi_i)| - radius(y)
```

GLSL:

```glsl
float layeredRidge(vec2 p) {
    float wave = 0.10*sin(11.0*p.y) + 0.05*sin(23.0*p.y + 1.7);
    float radius = max(0.015, 0.08 - 0.03*p.y);
    return abs(p.x - wave) - radius;
}
```

Visual result:

Stylized ridge field with organic waviness.

Debug view:

Render `smoothstep(0.02, -0.01, layeredRidge(p))`.

## 24. Curvilinear Basin/Path Mask

Equation:

```text
lineMask = exp(-k * distanceToCurve^2)
```

GLSL:

```glsl
float curvilinearPathMask(vec2 p) {
    float center = 0.35*sin(0.35*p.y) + 0.18*sin(0.9*p.y + 1.2);
    float d = abs(p.x - center);
    return exp(-8.0*d*d);
}
```

Visual result:

A winding basin/path/shadow corridor that gives the heightfield compositional direction.

Debug view:

Show the curve mask as white over height.

## 25. Distance-Based Detail Reduction

Equation:

```text
detailAmount = smoothstep(far, near, distance)
```

GLSL:

```glsl
float detailFade(float dist) {
    return smoothstep(180.0, 35.0, dist);
}
```

Visual result:

Near regions have crisp detail, distant regions become broad value shapes. This avoids noisy far fields.

Debug view:

Show `detailFade` as grayscale over hit distance.

## 26. Complete Terrain Shade Skeleton

Equation:

```text
color = fog( material(p,n) * light(n,l) * shadow(p,l), distance )
```

GLSL:

```glsl
vec3 shadeTerrain(vec3 ro, vec3 rd, vec3 sunDir) {
    float t = marchTerrain(ro, rd);
    vec3 sky = skyColor(rd, sunDir);
    if (t < 0.0) return sky;

    vec3 p = ro + rd*t;
    vec3 n = terrainNormal(p.xz);
    float ndl = max(dot(n, sunDir), 0.0);
    float sh = terrainShadow(p.xz, p.y, sunDir);
    vec3 mat = terrainMaterial(p, n);
    vec3 col = mat * (0.18 + 1.35*ndl*sh);
    float fog = heightFog(t, p.y);
    return mix(col, sky, fog);
}
```

Visual result:

A coherent heightfield rendering pipeline: scalar shape, material zones, light, shadow, and atmosphere.

Debug view:

Replace the final return with individual terms: `mat`, `vec3(ndl)`, `vec3(sh)`, `vec3(fog)`.
