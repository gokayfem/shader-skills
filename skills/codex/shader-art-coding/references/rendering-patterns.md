# Rendering Patterns

## Raymarching Structure

Use a clear SDF scene function and a bounded marcher:

```glsl
struct Hit {
    float d;
    float mat;
};

Hit mapScene(vec3 p) {
    Hit h = Hit(1e5, 0.0);
    float ground = p.y + 1.0;
    h = Hit(ground, 1.0);

    float body = sdSphere(p - vec3(0.0, 0.0, 3.0), 1.0);
    if (body < h.d) h = Hit(body, 2.0);
    return h;
}

Hit raymarch(vec3 ro, vec3 rd) {
    float t = 0.0;
    float mat = 0.0;
    for (int i = 0; i < 96; i++) {
        vec3 p = ro + rd*t;
        Hit h = mapScene(p);
        if (h.d < 0.001*t) return Hit(t, h.mat);
        if (t > 80.0) break;
        t += h.d;
        mat = h.mat;
    }
    return Hit(-1.0, mat);
}
```

Scale epsilon with distance to reduce shimmer in far geometry.

## Normals

Use tetrahedral sampling for compact normals:

```glsl
vec3 calcNormal(vec3 p) {
    vec2 e = vec2(0.001, -0.001);
    return normalize(
        e.xyy*mapScene(p + e.xyy).d +
        e.yyx*mapScene(p + e.yyx).d +
        e.yxy*mapScene(p + e.yxy).d +
        e.xxx*mapScene(p + e.xxx).d
    );
}
```

If the surface has procedural displacement, sample the same displaced field for normals.

## Lighting

Start with one directional light, then add fill and ambient:

```glsl
vec3 shade(vec3 p, vec3 n, vec3 rd, float mat) {
    vec3 ldir = normalize(vec3(0.6, 0.8, 0.4));
    float ndl = max(dot(n, ldir), 0.0);
    float fres = pow(1.0 + dot(n, rd), 5.0);
    vec3 albedo = materialColor(mat, p);
    vec3 col = albedo * (0.18 + 1.25*ndl);
    col += 0.15*fres;
    return col;
}
```

Keep material identity visible under diffuse light before adding specular polish.

## Ambient Occlusion and Soft Shadow

Use small, bounded approximations:

```glsl
float calcAO(vec3 p, vec3 n) {
    float occ = 0.0;
    float sca = 1.0;
    for (int i = 0; i < 5; i++) {
        float h = 0.02 + 0.12*float(i)/4.0;
        float d = mapScene(p + n*h).d;
        occ += (h - d) * sca;
        sca *= 0.75;
    }
    return clamp(1.0 - 2.0*occ, 0.0, 1.0);
}

float softShadow(vec3 ro, vec3 rd, float mint, float maxt, float k) {
    float res = 1.0;
    float t = mint;
    for (int i = 0; i < 48; i++) {
        float h = mapScene(ro + rd*t).d;
        if (h < 0.001) return 0.0;
        res = min(res, k*h/t);
        t += clamp(h, 0.02, 0.5);
        if (t > maxt) break;
    }
    return clamp(res, 0.0, 1.0);
}
```

Use these as art controls, not as a substitute for good shape design.

## Heightfields and Atmosphere

For landscapes, separate:

- Terrain height function.
- Surface normal from height differences.
- Material masks from altitude, slope, moisture/noise, and distance.
- Sky color and sun direction.
- Fog based on distance and height.

Atmosphere should reinforce depth:

```glsl
vec3 applyFog(vec3 col, vec3 fogCol, float dist) {
    float f = 1.0 - exp(-0.025*dist);
    return mix(col, fogCol, clamp(f, 0.0, 1.0));
}
```

## Antialiasing

For analytic 2D masks, use derivatives when available:

```glsl
float aaStep(float edge, float x) {
    float w = fwidth(x);
    return smoothstep(edge - w, edge + w, x);
}
```

For SDF edges, avoid fixed-width smoothing in screen space unless the scale is fixed. Derivative-aware or distance-scaled thresholds usually look better.

## Color Management

If lighting can exceed 1.0, tonemap and gamma-correct:

```glsl
vec3 tonemap(vec3 x) {
    x = max(x, 0.0);
    return x / (1.0 + x);
}

vec3 toSRGB(vec3 x) {
    return pow(max(x, 0.0), vec3(1.0/2.2));
}
```

## Practical Renderer Scaffolding

For complex formula-heavy shaders, keep these renderer pieces stable while changing the scene:

- Camera ray generation.
- `mapScene` returning distance and material ID.
- Normal calculation using the same map as rendering.
- Ambient occlusion and soft shadow functions.
- Background/sky.
- Tonemap/gamma.
- Optional cheap dithering after color grading.

This lets formula experiments focus on one field at a time. Do not rewrite the renderer while testing a new SDF, terrain patch, or material mask unless the renderer is the problem.
