# Formula-Code Couples: Materials, Color, Composition, and Polish

Use this file when the task needs equation-to-code translation for color ramps, masks, material layering, lighting, antialiasing, composition overlays, texture-like detail, or final polish.

Each couple has:

- Equation: the compact mathematical idea.
- GLSL: an original Shadertoy-friendly implementation.
- Visual result: what should appear when the formula is correct.
- Debug view: the simplest way to inspect it.

## 1. Smoothstep Mask

Equation:

```text
m = S(a, b, x)
```

GLSL:

```glsl
float softMask(float a, float b, float x) {
    return smoothstep(a, b, x);
}
```

Visual result:

A soft transition from black to white. This is the base for almost every controllable material boundary.

Debug view:

```glsl
fragColor = vec4(vec3(softMask(a, b, x)), 1.0);
```

## 2. Derivative Antialiasing

Equation:

```text
w = fwidth(x)
m = smoothstep(edge - w, edge + w, x)
```

GLSL:

```glsl
float aaStep(float edge, float x) {
    float w = fwidth(x);
    return smoothstep(edge - w, edge + w, x);
}
```

Visual result:

Sharp analytic masks become clean instead of stair-stepped.

Debug view:

Compare `step(edge, x)` and `aaStep(edge, x)` side by side.

## 3. Fill Mask From SDF

Equation:

```text
fill = smoothstep(w, -w, d)
```

GLSL:

```glsl
float sdfFill(float d) {
    float w = fwidth(d);
    return smoothstep(w, -w, d);
}
```

Visual result:

A filled 2D shape with antialiased edges.

Debug view:

Show `vec3(sdfFill(d))`.

## 4. Stroke Mask From SDF

Equation:

```text
stroke = smoothstep(width+w, width-w, |d|)
```

GLSL:

```glsl
float sdfStroke(float d, float width) {
    float w = fwidth(d);
    return smoothstep(width + w, width - w, abs(d));
}
```

Visual result:

A clean outline or ring around a shape.

Debug view:

Show stroke alone before compositing.

## 5. Cosine Palette

Equation:

```text
color(t) = a + b cos(2pi(c t + d))
```

GLSL:

```glsl
vec3 palette(float t, vec3 a, vec3 b, vec3 c, vec3 d) {
    return a + b*cos(6.2831853*(c*t + d));
}
```

Visual result:

Smooth repeating color harmonies useful for abstract shaders, heatmaps, irises, skies, and debug colors.

Debug view:

Render `palette(uv.x, ...)` as a horizontal strip.

## 6. Warm Light, Cool Shadow

Equation:

```text
color = albedo * (ambient + diffuse * warm) + shadowTint * (1-diffuse)
```

GLSL:

```glsl
vec3 warmCoolDiffuse(vec3 albedo, vec3 n, vec3 l) {
    float ndl = max(dot(n, l), 0.0);
    vec3 warm = vec3(1.08, 0.92, 0.72);
    vec3 cool = vec3(0.35, 0.45, 0.70);
    return albedo * (0.18*cool + ndl*warm);
}
```

Visual result:

Objects feel lit by sunlight rather than gray diffuse light.

Debug view:

Show `vec3(ndl)` and then the warm/cool shaded output.

## 7. Fresnel Rim

Equation:

```text
F = (1 + dot(n, rd))^p
```

GLSL:

```glsl
float fresnelRim(vec3 n, vec3 rd, float power) {
    return pow(clamp(1.0 + dot(n, rd), 0.0, 1.0), power);
}
```

Visual result:

A soft rim highlight near silhouettes, useful for subsurface, waxy, crystalline, reflective, or stylized material regions.

Debug view:

Show rim as grayscale; it should hug the silhouette.

## 8. Normal-View Localized Warmth Mask

Equation:

```text
mask = regionMask * max(dot(n, -view), 0)^p
```

GLSL:

```glsl
float normalViewWarmth(vec3 n, vec3 viewDir, float regionMask) {
    float facing = pow(clamp(dot(n, -viewDir), 0.0, 1.0), 2.0);
    return regionMask * facing;
}
```

Visual result:

Soft localized warmth on selected convex material regions.

Debug view:

Show the localized warmth mask over a neutral albedo.

## 9. Sparse Stochastic Spot Field

Equation:

```text
cell = floor(uv N)
q = fract(uv N)-1/2
spot = exp(-k ||q||^2 / r)
spot *= step(threshold, hash(cell))
```

GLSL:

```glsl
float sparseSpotField(vec2 uv) {
    vec2 cell = floor(uv*25.0);
    vec2 q = fract(uv*25.0) - 0.5;
    float r = 0.08 + 0.12*hash12(cell);
    float spot = exp(-24.0*dot(q, q) / r);
    return spot * step(0.78, hash12(cell + 19.1));
}
```

Visual result:

Sparse stochastic spots on a mapped surface. They should appear only inside a deliberate region mask.

Debug view:

Show sparse spots as black dots on white before material mixing.

## 10. Radial-Cell ID Albedo Variation

Equation:

```text
color = mix(C1, C2, hash(id))
```

GLSL:

```glsl
vec3 cellIdAlbedo(vec2 id) {
    float h = hash12(id);
    return mix(vec3(0.42, 0.10, 0.06), vec3(0.86, 0.28, 0.14), h);
}
```

Visual result:

Individual modular cells vary in albedo while remaining one material family.

Debug view:

Render cell IDs as colors before lighting.

## 11. Local Cell Grain

Equation:

```text
grain = fbm(localUV * frequency + seed)
color *= a + b grain
```

GLSL:

```glsl
vec3 applyCellGrain(vec3 color, vec2 localUv, vec2 id) {
    float seed = 13.0*hash12(id);
    float grain = fbm(localUv*18.0 + seed);
    return color * (0.78 + 0.32*grain);
}
```

Visual result:

Rough local-coordinate detail that stays attached to each modular cell.

Debug view:

Show grain only in local coordinates. If it swims or crosses interstitial boundaries, the coordinate space is wrong.

## 12. Interstitial Boundary Mask

Equation:

```text
mask = smoothstep(edge0, edge1, distanceFromMortar)
```

GLSL:

```glsl
float interstitialToCellMask(float distFromInterstitial) {
    return smoothstep(0.02, 0.08, distFromInterstitial);
}
```

Visual result:

Clean transition from an interstitial material to a cell material.

Debug view:

Show the mask as grayscale; the interstitial field should be continuous.

## 13. Material Lambda Mix

Equation:

```text
C = C0(1-lambda) + C1 lambda
```

GLSL:

```glsl
vec3 lambdaMix(vec3 a, vec3 b, float lambda) {
    return mix(a, b, clamp(lambda, 0.0, 1.0));
}
```

Visual result:

Any material blend becomes inspectable through one scalar control.

Debug view:

Always show `lambda` alone first.

## 14. Multi-Material Weighted Sum

Equation:

```text
C = sum_i w_i C_i / sum_i w_i
```

GLSL:

```glsl
vec3 weightedMaterial(vec3 c0, vec3 c1, vec3 c2, vec3 w) {
    w = max(w, vec3(0.0));
    return (c0*w.x + c1*w.y + c2*w.z) / max(0.001, w.x + w.y + w.z);
}
```

Visual result:

Three-channel material blends remain normalized and controllable.

Debug view:

Show `w` as RGB.

## 15. Edge Wear

Equation:

```text
wear = smoothstep(a, b, abs(distanceToEdge))
```

GLSL:

```glsl
float edgeWear(float bevelDistance, float noiseValue) {
    float edge = smoothstep(0.0, 0.08, bevelDistance);
    return clamp((1.0 - edge) + 0.25*noiseValue, 0.0, 1.0);
}
```

Visual result:

Lighter worn edges on any beveled or high-contact material.

Debug view:

Show wear mask before using it in albedo or roughness.

## 16. Dirt or Stain Gravity Bias

Equation:

```text
stain = noise(worldXZ) * smoothstep(top, bottom, y)
```

GLSL:

```glsl
float verticalStain(vec3 p) {
    float flow = smoothstep(0.7, -0.4, p.y);
    float breakup = fbm(vec2(p.x*1.7, p.z*1.7));
    return flow * smoothstep(0.35, 0.8, breakup);
}
```

Visual result:

Weathering, moss, or grime collects downward rather than randomly everywhere.

Debug view:

Show stain as a mask over a neutral material.

## 17. Specular Lobe

Equation:

```text
spec = max(dot(reflect(-l,n), -rd), 0)^shininess
```

GLSL:

```glsl
float specularLobe(vec3 n, vec3 l, vec3 rd, float shininess) {
    vec3 r = reflect(-l, n);
    return pow(max(dot(r, -rd), 0.0), shininess);
}
```

Visual result:

Controlled highlights on glossy, wet, polished, crystalline, or reflective material regions.

Debug view:

Show specular alone. If it covers the whole object, shininess is too low or normals are wrong.

## 18. Radial Chromatic Ring Field

Equation:

```text
r = ||uv||
a = atan(y,x)
iris = palette(r + noise(a,r))
```

GLSL:

```glsl
vec3 radialRingColor(vec2 uv) {
    float r = length(uv);
    float a = atan(uv.y, uv.x);
    float fibers = 0.5 + 0.5*sin(34.0*a + 8.0*fbm(vec2(r*5.0, a)));
    vec3 inner = vec3(0.16, 0.08, 0.03);
    vec3 outer = vec3(0.55, 0.32, 0.12);
    vec3 col = mix(inner, outer, smoothstep(0.08, 0.45, r));
    col += 0.18*fibers*(1.0 - smoothstep(0.35, 0.48, r));
    return col;
}
```

Visual result:

Radial chromatic detail with an inner disk and outer ring.

Debug view:

Render the radial ring field in 2D before mapping it onto a surface.

## 19. Composition Thirds Overlay

Equation:

```text
line = smoothstep(width, 0, |uv.x - k|)
```

GLSL:

```glsl
float lineAt(float x, float k, float w) {
    return smoothstep(w, 0.0, abs(x - k));
}

vec3 thirdsOverlay(vec2 uv, vec3 col) {
    float l = 0.0;
    l = max(l, lineAt(uv.x, -1.0/3.0, 0.006));
    l = max(l, lineAt(uv.x,  1.0/3.0, 0.006));
    l = max(l, lineAt(uv.y, -1.0/3.0, 0.006));
    l = max(l, lineAt(uv.y,  1.0/3.0, 0.006));
    return mix(col, vec3(1.0, 0.85, 0.1), 0.5*l);
}
```

Visual result:

Temporary composition guides to place focal objects and horizon.

Debug view:

Use only during composition, then remove or gate behind a debug constant.

## 20. Golden Ratio Overlay

Equation:

```text
phi^-1 = 0.618...
```

GLSL:

```glsl
vec3 goldenOverlay(vec2 uv, vec3 col) {
    float g = 0.6180339;
    float l = 0.0;
    l = max(l, lineAt(uv.x, -g, 0.005));
    l = max(l, lineAt(uv.x,  g, 0.005));
    l = max(l, lineAt(uv.y, -g, 0.005));
    l = max(l, lineAt(uv.y,  g, 0.005));
    return mix(col, vec3(0.2, 0.9, 1.0), 0.45*l);
}
```

Visual result:

Subtle framing guides for dominant silhouettes, horizons, and focal masses.

Debug view:

Toggle with `#define DEBUG_COMPOSITION 1`.

## 21. Tonemap

Equation:

```text
y = x / (1 + x)
```

GLSL:

```glsl
vec3 tonemap(vec3 x) {
    x = max(x, 0.0);
    return x / (1.0 + x);
}
```

Visual result:

Bright lighting compresses gracefully instead of clipping.

Debug view:

Compare before/after on a high-contrast light.

## 22. Gamma to sRGB

Equation:

```text
srgb = linear^(1/2.2)
```

GLSL:

```glsl
vec3 toSRGB(vec3 x) {
    return pow(max(x, 0.0), vec3(1.0/2.2));
}
```

Visual result:

Colors look less dark/muddy in standard display output.

Debug view:

Apply as the final step only.

## 23. Cheap Dithering

Equation:

```text
color += hash(pixel) / 255
```

GLSL:

```glsl
vec3 cheapDither(vec3 col, vec2 fragCoord) {
    float h = hash12(fragCoord);
    return col + (h - 0.5) / 255.0;
}
```

Visual result:

Reduced banding in smooth gradients, fog, and sky.

Debug view:

Zoom into gradients; dithering should be barely visible.

## 24. Slow Procedural Motion

Equation:

```text
offset(t) = A (noise(wt + seed) - 1/2)
```

GLSL:

```glsl
float slowNoise(float t, float seed) {
    return fbm(vec2(0.13*t, seed));
}

vec3 subtleCameraOffset(float t) {
    return 0.01*vec3(
        slowNoise(t, 1.0) - 0.5,
        slowNoise(t, 2.0) - 0.5,
        slowNoise(t, 3.0) - 0.5
    );
}
```

Visual result:

Camera or subject breathes subtly. Motion should feel alive, not unstable.

Debug view:

Print/plot the offset mentally: it should be low amplitude and low frequency.

## 25. Looping Time Phase

Equation:

```text
phase = 2pi fract(t / duration)
```

GLSL:

```glsl
float loopPhase(float t, float duration) {
    return 6.2831853 * fract(t / duration);
}
```

Visual result:

Reusable loop control for periodic field motion, aperture changes, flow, or camera orbit.

Debug view:

Animate `sin(phase)` and confirm it loops without a jump.

## 26. Temporal Aperture Pulse Mask

Equation:

```text
blink = smooth pulse over phase
```

GLSL:

```glsl
float aperturePulseAmount(float t) {
    float ph = fract(t / 4.0);
    float close = smoothstep(0.02, 0.05, ph);
    float open = smoothstep(0.11, 0.16, ph);
    return close * (1.0 - open);
}
```

Visual result:

A parameter closes briefly and reopens, useful for any aperture-like animated feature.

Debug view:

Show aperture openness as a number or mask before driving geometry.

## 27. Background Blur Weight

Equation:

```text
color' = sum_i w_i color(uv + offset_i) / sum_i w_i
```

GLSL:

```glsl
vec3 tinyBlur(sampler2D tex, vec2 uv, vec2 px) {
    vec3 c = vec3(0.0);
    float wsum = 0.0;
    for (int i = -2; i <= 2; i++) {
        float w = 1.0 - abs(float(i))/3.0;
        c += w * texture(tex, uv + px*float(i)).rgb;
        wsum += w;
    }
    return c / wsum;
}
```

Visual result:

Soft background or post-effect blur when texture input is available. For pure procedural scenes, use distance/fog instead.

Debug view:

Compare blurred and original background side by side.

## 28. Material Debug Router

Equation:

```text
debugMode selects scalar/vector field
```

GLSL:

```glsl
vec3 debugMaterialRouter(int mode, vec3 col, vec3 n, float mat, float depth, float mask) {
    if (mode == 1) return 0.5 + 0.5*n;
    if (mode == 2) return vec3(fract(mat*0.37), fract(mat*0.61), fract(mat*0.83));
    if (mode == 3) return vec3(depth * 0.03);
    if (mode == 4) return vec3(mask);
    return col;
}
```

Visual result:

A shader that can be inspected without rewriting code.

Debug view:

This is itself the debug view; keep it cheap and removable.
