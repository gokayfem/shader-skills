# Formula-Code Couples: Implicit Geometry and SDF Operators

Use this file when the task needs equation-to-code translation for implicit primitives, smooth composition, local coordinate transforms, modular radial structures, bilateral feature fields, anisotropic strand fields, shell intersections, or mathematical sculpting.

Each couple has:

- Equation: the compact mathematical idea.
- GLSL: an original Shadertoy-friendly implementation.
- Visual result: what should appear when the formula is correct.
- Debug view: the simplest way to inspect it.

## 1. Sphere SDF

Equation:

```text
d(p) = ||p - c|| - r
```

GLSL:

```glsl
float sdSphere(vec3 p, vec3 c, float r) {
    return length(p - c) - r;
}
```

Visual result:

A perfectly round volume. Normal debug should show a smooth radial rainbow with no seams.

Debug view:

```glsl
vec3 debugSphere(vec3 p) {
    float d = sdSphere(p, vec3(0.0), 1.0);
    return vec3(0.5 + 0.5*cos(80.0*d));
}
```

## 2. Ellipsoid Approximation

Equation:

```text
k0 = ||p / r||
k1 = ||p / r^2||
d(p) = k0(k0 - 1) / k1
```

GLSL:

```glsl
float sdEllipsoid(vec3 p, vec3 r) {
    float k0 = length(p / r);
    float k1 = length(p / (r*r));
    return k0 * (k0 - 1.0) / k1;
}
```

Visual result:

A soft stretched sphere suitable for any anisotropic lobe, capsule-like mass, soft shell, ovoid inclusion, or rounded protrusion.

Debug view:

```glsl
vec3 debugEllipsoid(vec3 p) {
    float d = sdEllipsoid(p, vec3(0.7, 1.0, 0.55));
    return mix(vec3(fract(16.0*d)), vec3(1.0, 0.25, 0.1), smoothstep(0.02, 0.0, abs(d)));
}
```

## 3. Box SDF

Equation:

```text
q = abs(p) - b
d(p) = ||max(q, 0)|| + min(max(qx, qy, qz), 0)
```

GLSL:

```glsl
float sdBox(vec3 p, vec3 b) {
    vec3 q = abs(p) - b;
    return length(max(q, 0.0)) + min(max(q.x, max(q.y, q.z)), 0.0);
}
```

Visual result:

A rectangular implicit solid with correct exterior edge distances and negative inside distance.

Debug view:

```glsl
vec3 debugBox(vec3 p) {
    float d = sdBox(p, vec3(0.8, 0.25, 0.35));
    return vec3(smoothstep(0.08, -0.02, d));
}
```

## 4. Rounded Box

Equation:

```text
d_round(p) = d_box(p, b - r) - r
```

GLSL:

```glsl
float sdRoundBox(vec3 p, vec3 b, float r) {
    vec3 q = abs(p) - (b - vec3(r));
    return length(max(q, 0.0)) + min(max(q.x, max(q.y, q.z)), 0.0) - r;
}
```

Visual result:

A rectangular implicit solid with bevel-like soft edges. Use when a hard-edged element needs a finite edge radius.

Debug view:

```glsl
vec3 debugRoundBox(vec3 p) {
    float d = sdRoundBox(p, vec3(0.8, 0.25, 0.35), 0.05);
    return vec3(0.5 + 0.5*cos(70.0*d));
}
```

## 5. Capsule

Equation:

```text
h = clamp(dot(p-a, b-a) / dot(b-a, b-a), 0, 1)
d(p) = ||p - a - h(b-a)|| - r
```

GLSL:

```glsl
float sdCapsule(vec3 p, vec3 a, vec3 b, float r) {
    vec3 pa = p - a;
    vec3 ba = b - a;
    float h = clamp(dot(pa, ba) / dot(ba, ba), 0.0, 1.0);
    return length(pa - ba*h) - r;
}
```

Visual result:

A swept segment with rounded endpoints. Use for any path-aligned tube, connector, appendage, rail, filament, or strap-like element.

Debug view:

```glsl
vec3 debugCapsule(vec3 p) {
    float d = sdCapsule(p, vec3(-0.6, 0.0, 0.0), vec3(0.6, 0.3, 0.0), 0.12);
    return mix(vec3(0.1), vec3(0.9), smoothstep(0.03, -0.03, d));
}
```

## 6. Torus and Tube

Equation:

```text
q = ( ||p_xz|| - R, p_y )
d(p) = ||q|| - r
```

GLSL:

```glsl
float sdTorus(vec3 p, vec2 t) {
    vec2 q = vec2(length(p.xz) - t.x, p.y);
    return length(q) - t.y;
}
```

Visual result:

A ring or continuous circular tube. Useful for any annular rim, orbital band, circular seam, or toroidal connector.

Debug view:

```glsl
vec3 debugTorus(vec3 p) {
    float d = sdTorus(p, vec2(0.8, 0.08));
    return vec3(smoothstep(0.04, -0.02, d));
}
```

## 7. Hard Boolean Operations

Equation:

```text
union:        min(a, b)
intersection: max(a, b)
subtraction:  max(a, -b)
```

GLSL:

```glsl
float opUnion(float a, float b) { return min(a, b); }
float opIntersect(float a, float b) { return max(a, b); }
float opSub(float a, float b) { return max(a, -b); }
```

Visual result:

Hard manufactured seams, crisp cuts, sockets, holes, and intersections.

Debug view:

```glsl
vec3 debugBoolean(float a, float b) {
    float d = opSub(a, b);
    return vec3(0.5 + 0.5*cos(90.0*d));
}
```

## 8. Polynomial Smooth Minimum

Equation:

```text
h = clamp(1/2 + (b-a)/(2k), 0, 1)
smin(a,b,k) = mix(b,a,h) - k h(1-h)
```

GLSL:

```glsl
float smin(float a, float b, float k) {
    float h = clamp(0.5 + 0.5*(b - a)/k, 0.0, 1.0);
    return mix(b, a, h) - k*h*(1.0 - h);
}
```

Visual result:

Two implicit primitives merge into one continuous differentiable-looking surface. Use for organic unions, soft accumulations, and clay-like blends.

Debug view:

```glsl
vec3 debugSmin(vec3 p) {
    float a = sdSphere(p - vec3(-0.35, 0.0, 0.0), vec3(0.0), 0.55);
    float b = sdSphere(p - vec3( 0.35, 0.0, 0.0), vec3(0.0), 0.55);
    float d = smin(a, b, 0.35);
    return vec3(0.5 + 0.5*cos(70.0*d));
}
```

## 9. Smooth Subtraction

Equation:

```text
h = clamp(1/2 - (b+a)/(2k), 0, 1)
ssub(a,b,k) = mix(a, -b, h) + k h(1-h)
```

GLSL:

```glsl
float smoothSub(float a, float b, float k) {
    float h = clamp(0.5 - 0.5*(b + a)/k, 0.0, 1.0);
    return mix(a, -b, h) + k*h*(1.0 - h);
}
```

Visual result:

A soft subtractive depression, groove, socket, seam, indentation, or eroded cut.

Debug view:

```glsl
vec3 debugSmoothSub(vec3 p) {
    float carrier = sdEllipsoid(p, vec3(0.7, 0.95, 0.55));
    float socket = sdSphere(p, vec3(0.25, 0.2, -0.45), 0.28);
    float d = smoothSub(carrier, socket, 0.12);
    return vec3(0.5 + 0.5*cos(85.0*d));
}
```

## 10. Local Transform Before Evaluation

Equation:

```text
q = R^-1 (p - c) / s
d_world(p) = s d_local(q)
```

GLSL:

```glsl
mat2 rot(float a) {
    float s = sin(a), c = cos(a);
    return mat2(c, -s, s, c);
}

float posedEllipsoid(vec3 p, vec3 c, vec3 r, float yaw) {
    vec3 q = p - c;
    q.xz = rot(-yaw) * q.xz;
    return sdEllipsoid(q, r);
}
```

Visual result:

The object moves, rotates, or poses without complicating the primitive formula. Use this for any local feature, modular cell, tilted lobe, leaning instance, or shell component.

Debug view:

```glsl
vec3 debugLocalTransform(vec3 p) {
    float d = posedEllipsoid(p, vec3(0.0), vec3(0.45, 0.8, 0.35), 0.45);
    return vec3(smoothstep(0.05, -0.02, d));
}
```

## 11. Mirror Symmetry With Later Breakage

Equation:

```text
q_x = |p_x|
```

GLSL:

```glsl
vec3 mirrorX(vec3 p) {
    p.x = abs(p.x);
    return p;
}
```

Visual result:

Symmetric paired features or bilateral ornamentation. Add small ID-based offsets afterward to avoid a dead mirrored look.

Debug view:

```glsl
vec3 debugMirror(vec3 p) {
    vec3 q = mirrorX(p);
    float d = sdSphere(q - vec3(0.32, 0.15, 0.0), vec3(0.0), 0.15);
    return vec3(smoothstep(0.03, -0.01, d));
}
```

## 12. Contour Debug for Distance Fields

Equation:

```text
bands(d) = 1/2 + 1/2 cos(w d)
surface(d) = smoothstep(e, 0, |d|)
```

GLSL:

```glsl
vec3 contourDebug(float d) {
    float bands = 0.5 + 0.5*cos(80.0*d);
    float surface = smoothstep(0.02, 0.0, abs(d));
    return mix(vec3(bands), vec3(1.0, 0.35, 0.1), surface);
}
```

Visual result:

Evenly spaced lines around clean SDFs. Kinks, pinches, and uneven spacing reveal bad blending, wrong scale, or non-distance operations.

Debug view:

Use this directly as the material color while building geometry.

## 13. Cylindrical Sector Coordinates

Equation:

```text
a = atan(z, x)
r = ||(x,z)||
u = a / 2pi * N
id = floor(u)
local = fract(u) - 1/2
```

GLSL:

```glsl
vec3 cylindricalSector(vec3 p, float sectors, out float sectorId) {
    float a = atan(p.z, p.x);
    float r = length(p.xz);
    float u = a / 6.2831853 * sectors;
    sectorId = floor(u);
    float localAngle = fract(u) - 0.5;
    return vec3(localAngle, p.y, r);
}
```

Visual result:

The scene splits into repeated radial slices. Each slice has a stable ID for color or structural variation.

Debug view:

```glsl
vec3 debugSector(vec3 p) {
    float id;
    vec3 q = cylindricalSector(p, 12.0, id);
    return vec3(fract(id*0.17), fract(id*0.37), fract(id*0.61)) * smoothstep(0.48, 0.0, abs(q.x));
}
```

## 14. Radial Row-Cell Local Space

Equation:

```text
row = floor(y / H)
u = a / 2pi * N + 0.5 mod(row, 2)
sector = floor(u)
local = (fract(u)-1/2, fract(y/H)-1/2, r-R)
```

GLSL:

```glsl
vec3 radialRowCellSpace(vec3 p, float sectors, float rowHeight, float radius, out vec2 id) {
    float row = floor(p.y / rowHeight);
    float stagger = 0.5 * mod(row, 2.0);
    float a = atan(p.z, p.x);
    float u = a / 6.2831853 * sectors + stagger;
    float sector = floor(u);
    id = vec2(sector, row);
    float lu = fract(u) - 0.5;
    float lv = fract(p.y / rowHeight) - 0.5;
    float lr = length(p.xz) - radius;
    return vec3(lu, lv, lr);
}
```

Visual result:

Staggered modular cells wrapping around a cylinder. If the equation is correct, alternate rows shift by half a cell while the underlying radial shell remains aligned.

Debug view:

```glsl
vec3 debugBrickIds(vec3 p) {
    vec2 id;
    vec3 q = radialRowCellSpace(p, 16.0, 0.22, 1.0, id);
    float mask = step(abs(q.x), 0.45) * step(abs(q.y), 0.42);
    return mask * vec3(fract(id.x*0.13), fract(id.y*0.19), fract(dot(id, vec2(0.07, 0.11))));
}
```

## 15. Modular Rectangular Cell SDF

Equation:

```text
d_cell = sdBox((lu, lv, lr), (W, H, D))
```

GLSL:

```glsl
float sdRadialCell(vec3 p, out vec2 id) {
    vec3 q = radialRowCellSpace(p, 16.0, 0.22, 1.0, id);
    q.x *= 0.75; // angular width correction
    return sdRoundBox(q, vec3(0.32, 0.085, 0.08), 0.018);
}
```

Visual result:

Individual rectangular modules on a cylindrical lattice. Without material variation, the structure should already read from silhouette and gaps.

Debug view:

```glsl
vec3 debugBrickSdf(vec3 p) {
    vec2 id;
    float d = sdRadialCell(p, id);
    return contourDebug(d);
}
```

## 16. Continuous Interstitial Radial Shell

Equation:

```text
d_shell = | ||p_xz|| - R | - thickness
d_interstitial = max(d_shell, vertical/radial trim)
```

GLSL:

```glsl
float sdInterstitialShell(vec3 p, float radius, float thickness) {
    float shell = abs(length(p.xz) - radius) - thickness;
    float heightTrim = abs(p.y - 0.8) - 1.1;
    return max(shell, heightTrim);
}
```

Visual result:

The interstitial shell reads as one continuous binding/support field behind and between modular cells, not as disconnected filler rectangles.

Debug view:

```glsl
vec3 debugMortar(vec3 p) {
    float d = sdInterstitialShell(p, 1.0, 0.025);
    return vec3(0.35, 0.34, 0.31) * smoothstep(0.03, -0.01, d);
}
```

## 17. Bilobed Organic Carrier

Equation:

```text
d = smin( ellipsoid_lobe_a, ellipsoid_lobe_b, k )
```

GLSL:

```glsl
float sdBilobedCarrier(vec3 p) {
    float upperLobe = sdEllipsoid(p - vec3(0.0, 0.18, 0.0), vec3(0.54, 0.68, 0.46));
    float lowerLobe = sdEllipsoid(p - vec3(0.0, -0.28, 0.02), vec3(0.42, 0.36, 0.38));
    return smin(upperLobe, lowerLobe, 0.22);
}
```

Visual result:

A two-lobe organic carrier: one dominant lobe and one secondary lobe blended into a continuous surface.

Debug view:

```glsl
vec3 debugHeadBase(vec3 p) {
    return contourDebug(sdBilobedCarrier(p));
}
```

## 18. Paired Concavity Carve

Equation:

```text
d = smoothSub(carrier, ellipsoid_concavity, k)
```

GLSL:

```glsl
float carvePairedConcavity(float carrierD, vec3 p, float side) {
    vec3 q = p - vec3(0.22*side, 0.12, -0.36);
    q.x *= 1.25;
    float concavity = sdEllipsoid(q, vec3(0.18, 0.11, 0.12));
    return smoothSub(carrierD, concavity, 0.08);
}
```

Visual result:

Soft paired concavities in a carrier surface. In lighting, each depression should read before any secondary inset element is added.

Debug view:

Render the carrier field as contour bands before and after applying the carve.

## 19. Parametric Arc Aperture Curve

Equation:

```text
r(x) = open (1 - x^2) (4x^3 - 9x^2 + 6x - 1)
```

GLSL:

```glsl
float apertureCurve(float x, float open) {
    float base = max(0.0, 1.0 - x*x);
    float expression = 4.0*x*x*x - 9.0*x*x + 6.0*x - 1.0;
    return open * base * expression;
}

float apertureMask(vec2 uv, float open) {
    float y = apertureCurve(uv.x, open);
    return smoothstep(0.025, 0.0, abs(uv.y - y));
}
```

Visual result:

Curved aperture bands with controllable openness and asymmetry. This is better than a circular line when the feature needs expressive deformation.

Debug view:

```glsl
vec3 debugEyelid(vec2 uv) {
    return vec3(apertureMask(uv, 0.18));
}
```

## 20. Soft Parametric Groove

Equation:

```text
d_line = |y - f(x)| - width(x)
mask = smoothstep(e, 0, d_line)
```

GLSL:

```glsl
float softGrooveCurve(vec2 uv) {
    float center = -0.04 + 0.06*uv.x*uv.x;
    float width = 0.018 * smoothstep(0.55, 0.05, abs(uv.x));
    return abs(uv.y - center) - width;
}

float softGrooveMask(vec2 uv) {
    float d = softGrooveCurve(uv);
    return smoothstep(0.02, -0.005, d);
}
```

Visual result:

A soft curved groove that can be shaded darker inside and tinted around its border.

Debug view:

Show `softGrooveMask` as grayscale on the local coordinate plane.

## 21. Anisotropic Strand Tube Perturbation

Equation:

```text
q' = q + A1 cos(w1 y + phi_id) + A2 sin(w2 y + psi_id)
```

GLSL:

```glsl
vec3 strandPerturbationSpace(vec3 q, float id) {
    float phase = 6.2831853 * hash12(vec2(id, 4.0));
    q.x += 0.025*cos(23.0*q.y + phase);
    q.z += 0.020*sin(23.0*q.y + phase);
    q.xz += 0.008*vec2(cos(91.0*q.y), sin(87.0*q.y));
    return q;
}

float sdPerturbedStrandSegment(vec3 p, vec3 a, vec3 b, float r, float id) {
    vec3 q = strandPerturbationSpace(p, id);
    return sdCapsule(q, a, b, r);
}
```

Visual result:

A strand-like tube that feels woven or fibrous rather than perfectly cylindrical. Use multiple nearby tubes with alternating phase.

Debug view:

Display strand IDs as color before adding the final anisotropic material.

## 22. Ellipsoidal Shell Clipped by Half-Space

Equation:

```text
d = max(sdEllipsoid(p, r), sdPlane(p, n, h))
```

GLSL:

```glsl
float sdPlane(vec3 p, vec3 n, float h) {
    return dot(p, normalize(n)) + h;
}

float sdClippedEllipsoidShell(vec3 p) {
    float shell = abs(sdEllipsoid(p, vec3(0.72, 0.86, 0.55))) - 0.045;
    float halfSpaceCut = sdPlane(p, vec3(0.0, 0.0, -1.0), -0.42);
    return max(shell, -halfSpaceCut);
}
```

Visual result:

A shell-like surface with a half-space opening or clipped front.

Debug view:

Use material IDs: one color for the carrier, one for the shell, one for the cut/inside.

## 23. Fabric Micro-Displacement

Equation:

```text
d' = d + A sin(wx x) sin(wy y)
```

GLSL:

```glsl
float fabricDisplace(float d, vec3 p) {
    float weave = sin(91.0*p.x) * sin(87.0*p.y);
    float broad = 0.5 + 0.5*sin(7.0*p.y + 2.0*sin(3.0*p.x));
    return d + 0.0035*weave + 0.01*broad;
}
```

Visual result:

Fine fabric or knitted texture that perturbs highlights. Keep amplitude tiny or the cloth silhouette becomes noisy.

Debug view:

Show the displacement field as `0.5 + 20.0*(dPrime - d)`.

## 24. Cone SDF for Trees and Props

Equation:

```text
cone approximates distance to a linearly shrinking radius over height
```

GLSL:

```glsl
float sdConeApprox(vec3 p, float h, float r) {
    float y = clamp(p.y, 0.0, h);
    float radiusAtY = r * (1.0 - y / h);
    vec2 q = vec2(length(p.xz) - radiusAtY, p.y - y);
    return length(max(q, 0.0)) + min(max(q.x, q.y), 0.0);
}
```

Visual result:

Simple evergreen silhouettes, roofs, spikes, stylized mountains, and cone props.

Debug view:

Use contour bands on one cone, then repeat with per-instance IDs.

## 25. Repeated Conical Field

Equation:

```text
cell = floor(xz / s)
local = fract(xz / s) - 1/2
height = h0 + random(cell) hvar
```

GLSL:

```glsl
float sdRepeatedConeField(vec3 p, float spacing) {
    vec2 cell = floor(p.xz / spacing);
    vec2 local = fract(p.xz / spacing) - 0.5;
    float rnd = hash12(cell);
    vec3 q = vec3(local.x*spacing, p.y, local.y*spacing);
    float h = mix(0.8, 1.6, rnd);
    float r = mix(0.22, 0.38, hash12(cell + 5.1));
    return sdConeApprox(q, h, r);
}
```

Visual result:

A field of repeated conical primitives. Variation should be visible in height and radius, but the set should still read as one coherent mass.

Debug view:

Render `hash12(cell)` as color to verify variation is stable in world space.

## 26. SDF Scene Return With Material ID

Equation:

```text
scene(p) = argmin_i d_i(p)
```

GLSL:

```glsl
struct Hit {
    float d;
    float mat;
};

Hit pickCloser(Hit a, Hit b) {
    return (b.d < a.d) ? b : a;
}

Hit mapScene(vec3 p) {
    Hit h = Hit(1e5, 0.0);
    h = pickCloser(h, Hit(sdBilobedCarrier(p), 1.0));
    h = pickCloser(h, Hit(sdClippedEllipsoidShell(p - vec3(0.0, 0.02, 0.0)), 2.0));
    h = pickCloser(h, Hit(sdRepeatedConeField(p - vec3(0.0, -1.0, 4.0), 1.1), 3.0));
    return h;
}
```

Visual result:

The closest surface wins and carries a material ID. This is the backbone for readable raymarched scenes.

Debug view:

```glsl
vec3 idColor(float id) {
    return vec3(fract(id*0.37), fract(id*0.61), fract(id*0.83));
}
```
