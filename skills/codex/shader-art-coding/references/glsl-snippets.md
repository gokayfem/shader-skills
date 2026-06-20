# GLSL Snippets

These snippets are original Shadertoy-compatible building blocks. Adapt them to the user's scene.

## Hash and Noise

```glsl
float hash12(vec2 p) {
    vec3 p3 = fract(vec3(p.xyx) * 0.1031);
    p3 += dot(p3, p3.yzx + 33.33);
    return fract((p3.x + p3.y) * p3.z);
}

float noise(vec2 p) {
    vec2 i = floor(p);
    vec2 f = fract(p);
    vec2 u = f*f*(3.0 - 2.0*f);
    float a = hash12(i + vec2(0.0, 0.0));
    float b = hash12(i + vec2(1.0, 0.0));
    float c = hash12(i + vec2(0.0, 1.0));
    float d = hash12(i + vec2(1.0, 1.0));
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

float fbm(vec2 p) {
    float f = 0.0;
    float a = 0.5;
    mat2 m = mat2(1.6, 1.2, -1.2, 1.6);
    for (int i = 0; i < 5; i++) {
        f += a * noise(p);
        p = m * p + 11.7;
        a *= 0.5;
    }
    return f;
}
```

## 2D Masks

```glsl
float sdCircle(vec2 p, float r) {
    return length(p) - r;
}

float sdRoundBox(vec2 p, vec2 b, float r) {
    vec2 q = abs(p) - b + r;
    return length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - r;
}

float fillMask(float d) {
    return smoothstep(fwidth(d), -fwidth(d), d);
}

float strokeMask(float d, float width) {
    float w = fwidth(d);
    return smoothstep(width + w, width - w, abs(d));
}
```

## Camera

```glsl
mat3 lookAt(vec3 ro, vec3 ta, float roll) {
    vec3 ww = normalize(ta - ro);
    vec3 uu = normalize(cross(ww, vec3(sin(roll), cos(roll), 0.0)));
    vec3 vv = cross(uu, ww);
    return mat3(uu, vv, ww);
}

vec3 cameraRay(vec2 uv, vec3 ro, vec3 ta, float lens) {
    mat3 cam = lookAt(ro, ta, 0.0);
    return normalize(cam * vec3(uv, lens));
}
```

## SDF Utility Extras

```glsl
float smin(float a, float b, float k) {
    float h = clamp(0.5 + 0.5*(b - a)/k, 0.0, 1.0);
    return mix(b, a, h) - k*h*(1.0 - h);
}

float sdEllipsoid(vec3 p, vec3 r) {
    float k0 = length(p / r);
    float k1 = length(p / (r*r));
    return k0 * (k0 - 1.0) / k1;
}

float sdTorus(vec3 p, vec2 t) {
    vec2 q = vec2(length(p.xz) - t.x, p.y);
    return length(q) - t.y;
}

float opSmoothSub(float a, float b, float k) {
    float h = clamp(0.5 - 0.5*(b + a)/k, 0.0, 1.0);
    return mix(a, -b, h) + k*h*(1.0 - h);
}
```

## Debug Displays

```glsl
vec3 contourDebug(float d) {
    float bands = 0.5 + 0.5*cos(80.0*d);
    float surface = smoothstep(0.02, 0.0, abs(d));
    return mix(vec3(bands), vec3(1.0, 0.35, 0.1), surface);
}

vec3 idColor(float id) {
    return vec3(fract(id*0.37), fract(id*0.61), fract(id*0.83));
}
```

## Heightfield Helpers

```glsl
float smoothCellHeight(vec2 p) {
    vec2 i = floor(p);
    vec2 f = fract(p);
    vec2 u = f*f*f*(f*(f*6.0 - 15.0) + 10.0);
    float a = hash12(i + vec2(0.0, 0.0));
    float b = hash12(i + vec2(1.0, 0.0));
    float c = hash12(i + vec2(0.0, 1.0));
    float d = hash12(i + vec2(1.0, 1.0));
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

vec3 heightNormal(vec2 p) {
    float e = 0.02;
    float h = terrainHeight(p);
    return normalize(vec3(
        h - terrainHeight(p + vec2(e, 0.0)),
        e,
        h - terrainHeight(p + vec2(0.0, e))
    ));
}
```

`heightNormal` expects the shader to define `terrainHeight(vec2)`.

## Complete Minimal Raymarch Shader

```glsl
#define DEBUG_MODE 0

struct Hit {
    float d;
    float mat;
};

mat3 lookAt(vec3 ro, vec3 ta, float roll) {
    vec3 ww = normalize(ta - ro);
    vec3 uu = normalize(cross(ww, vec3(sin(roll), cos(roll), 0.0)));
    vec3 vv = cross(uu, ww);
    return mat3(uu, vv, ww);
}

vec3 cameraRay(vec2 uv, vec3 ro, vec3 ta, float lens) {
    mat3 cam = lookAt(ro, ta, 0.0);
    return normalize(cam * vec3(uv, lens));
}

float hash12(vec2 p) {
    vec3 p3 = fract(vec3(p.xyx) * 0.1031);
    p3 += dot(p3, p3.yzx + 33.33);
    return fract((p3.x + p3.y) * p3.z);
}

float sdSphere(vec3 p, float r) {
    return length(p) - r;
}

float sdBox(vec3 p, vec3 b) {
    vec3 q = abs(p) - b;
    return length(max(q, 0.0)) + min(max(q.x, max(q.y, q.z)), 0.0);
}

float opSmoothUnion(float a, float b, float k) {
    float h = clamp(0.5 + 0.5*(b - a)/k, 0.0, 1.0);
    return mix(b, a, h) - k*h*(1.0 - h);
}

Hit mapScene(vec3 p) {
    Hit h = Hit(p.y + 0.75, 1.0);

    vec3 q = p - vec3(0.0, 0.0, 3.0);
    float body = sdSphere(q, 0.85);
    float cut = sdBox(q - vec3(0.0, -0.25, 0.0), vec3(1.2, 0.2, 1.2));
    body = max(body, -cut);

    if (body < h.d) h = Hit(body, 2.0);
    return h;
}

Hit raymarch(vec3 ro, vec3 rd) {
    float t = 0.0;
    float mat = 0.0;
    for (int i = 0; i < 96; i++) {
        vec3 p = ro + rd*t;
        Hit h = mapScene(p);
        if (h.d < 0.001*t + 0.0005) return Hit(t, h.mat);
        if (t > 60.0) break;
        t += h.d;
        mat = h.mat;
    }
    return Hit(-1.0, mat);
}

vec3 calcNormal(vec3 p) {
    vec2 e = vec2(0.001, -0.001);
    return normalize(
        e.xyy*mapScene(p + e.xyy).d +
        e.yyx*mapScene(p + e.yyx).d +
        e.yxy*mapScene(p + e.yxy).d +
        e.xxx*mapScene(p + e.xxx).d
    );
}

vec3 materialColor(float mat, vec3 p) {
    if (mat < 1.5) return vec3(0.34, 0.30, 0.25);
    float grain = hash12(floor(p.xz*12.0));
    return mix(vec3(0.75, 0.42, 0.28), vec3(0.95, 0.72, 0.48), grain);
}

vec3 tonemap(vec3 x) {
    x = max(x, 0.0);
    return x / (1.0 + x);
}

void mainImage(out vec4 fragColor, in vec2 fragCoord) {
    vec2 uv = (2.0*fragCoord - iResolution.xy) / iResolution.y;

    float time = iTime;
    vec3 ro = vec3(2.6*sin(0.2*time), 1.1, -1.2 + 2.6*cos(0.2*time));
    vec3 ta = vec3(0.0, -0.05, 3.0);
    vec3 rd = cameraRay(uv, ro, ta, 1.8);

    vec3 sky = mix(vec3(0.62, 0.76, 0.95), vec3(0.08, 0.11, 0.16), smoothstep(-0.2, 0.8, uv.y));
    vec3 col = sky;

    Hit hit = raymarch(ro, rd);
    if (hit.d > 0.0) {
        vec3 p = ro + rd*hit.d;
        vec3 n = calcNormal(p);
        vec3 ldir = normalize(vec3(0.5, 0.8, -0.3));
        float ndl = max(dot(n, ldir), 0.0);
        float fres = pow(1.0 + dot(n, rd), 5.0);
        vec3 albedo = materialColor(hit.mat, p);
        col = albedo * (0.18 + 1.25*ndl) + 0.16*fres;
        float fog = 1.0 - exp(-0.025*hit.d*hit.d);
        col = mix(col, sky, clamp(fog, 0.0, 1.0));

        if (DEBUG_MODE == 1) col = 0.5 + 0.5*n;
        if (DEBUG_MODE == 2) col = vec3(fract(hit.mat*0.37), fract(hit.mat*0.61), fract(hit.mat*0.83));
    }

    col = pow(tonemap(col), vec3(1.0/2.2));
    fragColor = vec4(col, 1.0);
}
```
