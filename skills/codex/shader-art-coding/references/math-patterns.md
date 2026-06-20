# Math Patterns

## Coordinates

Use centered coordinates for Shadertoy:

```glsl
vec2 uv = (2.0*fragCoord - iResolution.xy) / iResolution.y;
```

Create local spaces by translating, rotating, scaling, mirroring, or repeating before evaluating a shape. Prefer local-space transforms over complicated global formulas.

Common transforms:

- Mirror symmetry: `p.x = abs(p.x);`
- Polar angle: `float a = atan(p.y, p.x);`
- Radius: `float r = length(p);`
- Cell ID: `vec2 id = floor(p);`
- Local cell coordinate: `vec2 q = fract(p) - 0.5;`
- Rotate a local part instead of rotating the whole scene when posing features.

```glsl
mat2 rot(float a) {
    float s = sin(a), c = cos(a);
    return mat2(c, -s, s, c);
}
```

## Shaping Functions

Use shaping functions as art controls:

- `smoothstep(a, b, x)` creates soft masks and antialias-like transitions.
- `pow(x, k)` remaps contrast; `k > 1` tightens highlights, `k < 1` opens shadows.
- `exp(-k*x)` creates haze, glow falloff, and atmospheric depth.
- `sin` and `cos` are best as periodic controls, not random detail.
- `mix(a, b, t)` should usually receive a named mask.

Name masks after their visual role:

```glsl
float rimMask = smoothstep(0.15, 0.0, abs(d));
float accumulationLine = smoothstep(0.35, 0.75, height + 0.2*noise);
```

## SDF Basics

An SDF returns negative inside, zero on the surface, positive outside. Keep primitive helpers small and composable.

Useful primitives:

```glsl
float sdSphere(vec3 p, float r) {
    return length(p) - r;
}

float sdBox(vec3 p, vec3 b) {
    vec3 q = abs(p) - b;
    return length(max(q, 0.0)) + min(max(q.x, max(q.y, q.z)), 0.0);
}

float sdCapsule(vec3 p, vec3 a, vec3 b, float r) {
    vec3 pa = p - a, ba = b - a;
    float h = clamp(dot(pa, ba) / dot(ba, ba), 0.0, 1.0);
    return length(pa - ba*h) - r;
}
```

Composition:

```glsl
float opUnion(float a, float b) { return min(a, b); }
float opSub(float a, float b) { return max(a, -b); }
float opIntersect(float a, float b) { return max(a, b); }

float opSmoothUnion(float a, float b, float k) {
    float h = clamp(0.5 + 0.5*(b - a)/k, 0.0, 1.0);
    return mix(b, a, h) - k*h*(1.0 - h);
}
```

Use smooth union for organic blending and hard boolean operations for manufactured or carved forms.

Smooth subtraction is useful for eyeaperture bands, sockets, grooves, dents, and soft cuts:

```glsl
float opSmoothSub(float a, float b, float k) {
    float h = clamp(0.5 - 0.5*(b + a)/k, 0.0, 1.0);
    return mix(a, -b, h) + k*h*(1.0 - h);
}
```

## Repetition and IDs

Separate repetition from variation:

```glsl
vec2 cell = floor(p.xz);
vec2 local = fract(p.xz) - 0.5;
float rnd = hash12(cell);
```

For circular repetition, use angle sectors and radius independently. Keep the sector ID for material variation.

## Noise, FBM, and Domain Warping

Use noise in tiers:

- Low frequency: silhouette, heightfield height, coverage masses.
- Mid frequency: material breakup, vegetation clumps, stone surface.
- High frequency: grain, pores, scratches, sparkle.

FBM pattern:

```glsl
float fbm(vec2 p) {
    float a = 0.5;
    float f = 0.0;
    mat2 m = mat2(1.6, 1.2, -1.2, 1.6);
    for (int i = 0; i < 5; i++) {
        f += a * noise(p);
        p = m * p;
        a *= 0.5;
    }
    return f;
}
```

Domain warping means using one field to move the coordinates of another:

```glsl
vec2 warp = vec2(fbm(p + 3.1), fbm(p - 2.7));
float pattern = fbm(p + 0.35*warp);
```

Keep warp amplitude small until the base structure works.

## Formula Debugging

Every scalar field can become an image. Before using a scalar to drive color, geometry, or animation, show it:

```glsl
fragColor = vec4(vec3(fieldValue), 1.0);
fragColor = vec4(vec3(smoothstep(a, b, fieldValue)), 1.0);
fragColor = vec4(0.5 + 0.5*normal, 1.0);
```

If a field is too noisy in grayscale, it will be too noisy after coloring. If a mask has bad edges, material polish will not save it.

## Palettes

Use palettes as controlled ramps, not random RGB constants:

```glsl
vec3 palette(float t, vec3 a, vec3 b, vec3 c, vec3 d) {
    return a + b*cos(6.28318*(c*t + d));
}
```

For natural scenes, combine physically motivated value ramps with gentle hue shifts: warmer light, cooler shadow, lower saturation in distance.
