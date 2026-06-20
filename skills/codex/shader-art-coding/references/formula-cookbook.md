# Formula Cookbook

Use this when a prompt needs concrete mathematical construction strategy, not only generic shader structure.

These recipes describe visually verified shader-building patterns: define a simple field, expose IDs and local coordinates, then use those controls for geometry, material, lighting, and variation. The examples are original abstractions, not copied source.

## Visual Formula Audit

When deriving or checking formulas from a video, screenshot, or sketch:

1. Identify the visible stage: primitive, repetition, material, lighting, or polish.
2. Name the coordinate space before naming the formula.
3. Find the local ID: cell, sector, row, branch, feature, material, or layer.
4. Separate silhouette math from material math.
5. Display the controlling scalar as grayscale before using it in color.
6. Ask what the formula is making controllable: position, thickness, softness, slope, frequency, density, or age.

Good shader formulas usually create knobs. If a formula only makes a picture more complicated and exposes no art control, simplify it.

## Cylindrical Modular Cell Lattice

Use this for radial cell lattices, tiled columns, pipes, shingles, woven bands, modular facades, and radial ornaments.

### Field Stack

1. Convert horizontal position to polar form:
   - `radius = length(p.xz)`
   - `angle = atan(p.z, p.x)`
2. Divide angle into sectors for cell columns.
3. Divide height into rows.
4. Stagger alternate rows by half a sector.
5. Keep IDs:
   - `sectorId` for left/right cell variation.
   - `rowId` for vertical variation.
   - `brickId` for color, displacement, damage, and missing pieces.
6. Convert back to a local cell coordinate:
   - angular local coordinate controls cell width.
   - vertical local coordinate controls cell height.
   - radial coordinate controls wall thickness.
7. Model the interstitial material as its own continuous field, not just leftover space between cells.

### Visual Checks

- With only local cell coordinates visible, every cell should be a centered rectangle.
- With IDs visible, adjacent bricks should not share identical colors too often.
- The interstitial field should remain continuous around the cylinder when rows are staggered.
- Displacement should be strongest inside cell faces, weaker near interstitial boundaries, and not destroy the silhouette unless erosion is intended.

### Generic Formula Sketch

```glsl
vec3 radialCellSpace(vec3 p, float sectors, float rowHeight, out vec2 id) {
    float a = atan(p.z, p.x);
    float r = length(p.xz);
    float row = floor(p.y / rowHeight);
    float stagger = mod(row, 2.0) * 0.5;
    float u = a / 6.2831853 * sectors + stagger;
    float sector = floor(u);
    id = vec2(sector, row);
    float lu = fract(u) - 0.5;
    float lv = fract(p.y / rowHeight) - 0.5;
    return vec3(lu, lv, r);
}
```

Use a box-like SDF in `(lu, lv, radialOffset)` for each cell face. Use a cylinder or radial shell field for an interstitial shell so the binding material feels continuous rather than fragmented.

### Verified Build Order

For cylindrical modular-cell systems, use this order:

1. Start with an existing lit raymarch renderer and a single primitive.
2. Replace the primitive with a box SDF and verify the box in isolation.
3. Introduce polar angle only; render a single angularly controlled band.
4. Add `sectorId = round(angle / sectorAngle)` and rotate the point back into the sector's local axis.
5. Add `rowId = round(y / rowHeight)` and shift the local y coordinate by row height.
6. Add row parity or hash-based stagger before material variation.
7. Add modular-cell SDF.
8. Add interstitial/tube SDF as a separate object/material.
9. Use IDs for color variation and local face coordinates for roughness.
10. Add displacement last; keep it local to cell faces.

### Cell Material Flow

Use IDs for coarse variation and local coordinates for fine variation:

```glsl
float cellRand = hash12(id);
float faceGrain = fbm(local.xy*18.0 + 7.0*cellRand);
float interstitialMask = smoothstep(0.02, 0.08, distanceFromInterstitial);
vec3 cellColor = mix(vec3(0.45, 0.12, 0.07), vec3(0.9, 0.32, 0.18), cellRand);
cellColor *= 0.82 + 0.28*faceGrain;
vec3 color = mix(vec3(0.42, 0.39, 0.34), cellColor, interstitialMask);
```

Do not use the same noise for cell ID, cell grain, interstitial stain, and displacement. Those are different art controls.

## Polynomial Heightfield Synthesis

Use this for heightfields, rolling scalar surfaces, basin/ridge systems, and heightfield-like abstract surfaces.

### Field Stack

1. Start with a height function `h(xz)`, not a mesh.
2. Use low-frequency shape first: ridge, basin, slope, plateau, horizon silhouette.
3. Add multi-scale detail only after the large forms compose well.
4. Estimate normals from finite differences of the exact height function.
5. Derive material masks from interpretable fields:
   - altitude,
   - slope,
   - curvature or ridge strength,
   - moisture/noise,
   - distance from camera.
6. Add atmosphere before vegetation. Fog and sky color establish scale.
7. Add density or material channels as a density field over heightfield, then light them as volumes or clumps.

### Smooth Cell Terrain

For tiled procedural heightfield, avoid visible cell seams by forcing continuity at cell boundaries. A useful pattern is: choose corner heights or coefficients per cell, interpolate with a cubic/quintic curve, then layer octaves. A single high-degree polynomial over the whole scene is too global; small patches give local control.

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
```

### Patch Thinking

For a cell `(i, j)`, treat the local coordinate `(x, z)` as living in `[0,1]^2`. Build the height from values at the four corners plus smooth interpolation:

```glsl
float patchHeight(vec2 p) {
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

Use quintic interpolation when normal smoothness matters. Use derivative-aware or analytic derivatives when available; otherwise use finite differences.

### Octave Matrix

Rotate and scale each octave to avoid grid-aligned mountains:

```glsl
float heightfieldBase(vec2 p) {
    mat2 m = mat2(0.8, -0.6, 0.6, 0.8) * 2.0;
    float h = 0.0;
    float a = 1.0;
    for (int i = 0; i < 7; i++) {
        h += a * patchHeight(p);
        p = m * p + 13.7;
        a *= 0.5;
    }
    return h;
}
```

Keep a separate low-frequency base shape so the composition does not become an even carpet of fractal detail.

### Terrain Normal

```glsl
vec3 heightfieldNormal(vec2 xz) {
    float e = 0.02;
    float h = heightfieldHeight(xz);
    float hx = heightfieldHeight(xz + vec2(e, 0.0)) - h;
    float hz = heightfieldHeight(xz + vec2(0.0, e)) - h;
    return normalize(vec3(-hx, e, -hz));
}
```

### Visual Checks

- Show height as grayscale; large slopes should be legible without lighting.
- Show normals as RGB; seams reveal discontinuity immediately.
- Show slope masks before assigning accumulation/channel A/channel B material.
- Show fog alone over depth; atmosphere should create scale before details.

### Heightfield Self-Shadowing

Terrain shadows can be approximated by marching along the light direction in the heightfield. The visual idea is to compare the heightfield height under the ray with the ray's expected height.

```glsl
float heightfieldShadow(vec2 xz, float h, vec3 ldir) {
    float shadow = 1.0;
    float t = 0.05;
    for (int i = 0; i < 48; i++) {
        vec2 q = xz + ldir.xz * t;
        float rayH = h + ldir.y * t;
        float heightfieldH = heightfieldHeight(q);
        float clearance = rayH - heightfieldH;
        shadow = min(shadow, smoothstep(0.0, 0.25*t, clearance));
        t += 0.12 + 0.08*t;
    }
    return clamp(shadow, 0.0, 1.0);
}
```

This is an art approximation: reduce steps for distant heightfield and soften aggressively to avoid crawling.

## Bilateral Organic SDF Composition

Use this for bilateral organic carriers, articulated forms, stylized ovoids, appendages, shell layers, and props.

### Field Stack

1. Block the primary carrier volume with ellipsoids and capsules.
2. Pose by transforming the local space before evaluating each body part.
3. Blend organic masses with smooth union.
4. Carve concavities, aperture bands, curved grooves, shell seams, and small recesses with subtraction or smooth subtraction.
5. Use material gradients to do some work that geometry should not do: localized warmth, subsurface tint, groove color, aperture shadow, woven-material tint.
6. Add anisotropic strands as repeated curves, capsules, shells, or clumps, not as individually simulated strands unless the shot is close.
7. Add environment only after dominant carrier proportions read.

### Smooth-Min Sculpting

Use smooth minimum to turn primitive intersections into anatomy. The polynomial smooth-min radius should be smaller than the feature being blended:

```glsl
float smin(float a, float b, float k) {
    float h = clamp(0.5 + 0.5*(b - a)/k, 0.0, 1.0);
    return mix(b, a, h) - k*h*(1.0 - h);
}
```

Use contour-band debug to inspect whether the field remains smooth:

```glsl
vec3 contourDebug(float d) {
    float bands = 0.5 + 0.5*cos(80.0*d);
    float surface = smoothstep(0.02, 0.0, abs(d));
    return mix(vec3(bands), vec3(1.0, 0.35, 0.1), surface);
}
```

Contours that kink or bunch reveal bad blends, wrong scale, or accidental non-SDF operations.

### Ellipsoid Approximation

```glsl
float sdEllipsoid(vec3 p, vec3 r) {
    float k0 = length(p / r);
    float k1 = length(p / (r*r));
    return k0 * (k0 - 1.0) / k1;
}
```

Use ellipsoids for ovoid lobes, convex protrusions, insets, appendages, stones, leaves, and soft props. Use capsules for necks, limbs, strand bundles, straps, stems, and rails.

### Concavity and Aperture Pattern

Create the inset sphere as a sphere/ellipsoid, then shape aperture bands as overlapping soft masks or SDF cuts. The concavity is usually a subtractive or darkened concavity, not merely a flat disk.

Visual controls:

- socket depth,
- lid thickness,
- iris radius,
- highlight radius,
- brow shadow,
- asymmetry/pose.

Eyeaperture bands can be controlled by a curve radius along local x:

```glsl
float lidCurve(float x, float open) {
    float base = 1.0 - x*x;
    float expression = x*x*(2.0*x - 3.0) + 1.0;
    return open * base * expression;
}
```

This style of formula is valuable because it exposes expression controls: openness, tilt, thickness, and asymmetry.

### Subsurface and Soft Materials

For subsurface-like or waxy materials, use:

- warm diffuse base,
- cool soft shadow,
- warm localized curvature/chroma masks,
- high roughness broad specular,
- subtle noise in color, not noisy geometry.

Use geometric fields to place color:

```glsl
float freckleField(vec2 uv) {
    vec2 cell = floor(uv*25.0);
    vec2 q = fract(uv*25.0) - 0.5;
    float r = 0.08 + 0.12*hash12(cell);
    float spot = exp(-24.0*dot(q, q) / r);
    return spot * step(0.78, hash12(cell + 19.1));
}

float softBlush(vec3 n, vec3 viewDir, float cheekMask) {
    float facing = pow(clamp(dot(n, -viewDir), 0.0, 1.0), 2.0);
    return cheekMask * facing;
}
```

Subsurface detail should be sparse and masked. Uniform procedural stochastic spots across every surface look synthetic.

### Anisotropic Strand Clumps

Use a curve or tube coordinate `q`, a strand ID, and sinusoidal offsets:

```glsl
vec3 braidOffset(vec3 q, float id) {
    float phase = 6.28318*hash12(vec2(id, 4.0));
    q.x += 0.025*cos(23.0*q.y + phase);
    q.z += 0.020*sin(23.0*q.y + phase);
    q.xz += 0.008*vec2(cos(91.0*q.y), sin(87.0*q.y));
    return q;
}
```

For visible strand bundles, repeat three or more lobes around the strand-bundle cross-section, then shade with alternating highlights using the strand ID and local tangent direction.

### Visual Checks

- In normal debug, facial planes should flow smoothly across blended forms.
- In material-ID debug, insets, subsurface material, strands, shell layers, and props should be cleanly separable.
- In grayscale, the focal carrier should read without relying on hue.

## Vegetation as Volumetric Clumps

Use this for instanced clump fields, low vegetation density fields, moss-like coverage, and stylized volumetric density.

Do not start with individual leaves. Start with a density envelope:

```glsl
float foliageDensity(vec3 p, vec2 cellId) {
    float crown = 1.0 - smoothstep(0.6, 1.0, length(p.xz));
    float vertical = smoothstep(-0.4, 0.2, p.y) * smoothstep(1.2, 0.5, p.y);
    float breakup = 0.65 + 0.35*fbm(p.xz*4.0 + cellId);
    return crown * vertical * breakup;
}
```

Then shade the clump with directional light, fake self-shadowing, and distance fade. Variation should come from cell IDs: height, hue, lean, crown radius, and density.

For heightfield forests, place clumps using a grid over heightfield, but derive density from slope, altitude, light, and distance:

```glsl
float treeSuitability(float height, float slope, float moisture, float dist) {
    float altitude = smoothstep(0.05, 0.35, height) * smoothstep(1.0, 0.65, height);
    float flatness = smoothstep(0.85, 0.35, slope);
    float distanceFade = smoothstep(140.0, 20.0, dist);
    return altitude * flatness * moisture * distanceFade;
}
```

Instance fields should align to the heightfield normal for close views and become texture/density fields in the distance.

## Clouds and Atmospheric Volume

Use this for skies, fog banks, mist, smoke, and volumetric-looking 2D effects.

Atmospheric coverage recipe:

1. Build a low-frequency coverage field.
2. Warp coordinates gently with fbm.
3. Use height or screen-y to shape coverage layer thickness.
4. Light one side with the sun direction.
5. Fade with distance/horizon.

```glsl
float cloudMask(vec2 p, float coverage) {
    vec2 warp = vec2(fbm(p*0.45 + 4.1), fbm(p*0.45 - 2.8));
    float d = fbm(p + 0.55*warp);
    return smoothstep(coverage, coverage + 0.18, d);
}
```

Visual checks:

- Clouds should have large masses before wispy detail.
- The sun-facing side should be warmer/brighter.
- Coverage contrast should fall near the horizon if atmosphere is present.

## Environment Props and Snowy Detail

Use environment props to support the subject, not compete with it.

Common formulas:

- Cones for conical instance silhouettes.
- Boxes/capsules for rails, planks, fences, bridges, and posts.
- Sine-stacked offsets for layered branches, accumulation ridges, or stylized bark.
- Height masks and upward normals for upward-facing accumulation.
- Low-frequency fbm for uneven ground and wind-drifted accumulation.

```glsl
float snowAccumulation(vec3 n, float height, float noiseValue) {
    float upward = smoothstep(0.35, 0.9, n.y);
    float altitude = smoothstep(-0.2, 0.6, height);
    return clamp(upward * altitude + 0.18*noiseValue, 0.0, 1.0);
}

float layeredBranch(vec2 p) {
    float wave = 0.10*sin(11.0*p.y) + 0.05*sin(23.0*p.y + 1.7);
    return abs(p.x - wave) - (0.08 - 0.03*p.y);
}
```

For accumulation-material scenes, separate white material from bright lighting: the accumulation material has blue/cool shadow, warm highlights, rough sparkle, and soft occlusion in creases.

## Temporal Polish

Use tiny motion fields at the end:

- camera breathing,
- carrier/aperture drift,
- cloth/fog movement,
- branch sway,
- coverage-field drift.

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

Motion should reveal the mathematical form without making the shader harder to inspect.

## Material Breakup Without Noise Soup

Use material breakup in named layers:

- edge wear: function of SDF distance to bevel/edge,
- stains: low-frequency world-space noise with gravity bias,
- grain: tangent or object-local stretched noise,
- cracks: thresholded cellular/noise ridges,
- color variation: ID hash plus low-frequency noise,
- roughness variation: independent from albedo.

Noise should be masked by material logic:

```glsl
float cellInteriorMask = smoothstep(0.08, 0.14, distanceFromInterstitial);
float grain = fbm(localFaceUv*18.0);
albedo *= mix(1.0, 0.75 + 0.35*grain, cellInteriorMask);
```

This keeps detail inside the visual object it belongs to.

## Composition Math

Mathematical images still need composition math:

- Put the focal mass near thirds or golden-ratio intersections, but break exact symmetry.
- Use a broad value ramp from foreground to background.
- Reduce high-frequency detail with distance.
- Use directional light to draw attention toward the subject.
- Use masks to reserve saturated color for the focal area.

Composition overlays can be temporary debug views:

```glsl
float ruleLine(vec2 uv, float x) {
    return smoothstep(0.006, 0.0, abs(uv.x - x));
}
vec3 compositionOverlay(vec2 uv, vec3 col) {
    float g = 0.618;
    float lines = max(ruleLine(uv, -g), ruleLine(uv, g));
    lines = max(lines, max(ruleLine(uv.yx, -g), ruleLine(uv.yx, g)));
    return mix(col, vec3(1.0, 0.85, 0.15), 0.45*lines);
}
```

## Formula Selection Heuristics

- Need perfect controllable silhouette: use SDF or analytic masks.
- Need broad natural variation: use smooth noise/fbm.
- Need repeated objects: use cell IDs and local coordinates.
- Need organic blends: use smooth union/subtraction.
- Need manufactured seams: use hard booleans, bevels, and separate material IDs.
- Need scale: use fog, value compression, frequency reduction, and desaturation with distance.
- Need bilateral organic appeal: use proportions, asymmetry, warm/cool material gradients, and clean inset/aperture geometry before micro-detail.
