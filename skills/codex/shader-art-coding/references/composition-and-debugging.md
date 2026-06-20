# Composition and Debugging

## Composition Rules for Procedural Shaders

Make the image readable before making it detailed:

- Use one dominant silhouette or value mass.
- Reserve high contrast for the focal area.
- Separate foreground, midground, and background by value and saturation.
- Use atmosphere, scale, and texture frequency to show depth.
- Let repeated elements vary in size, color, angle, or phase.
- Keep large forms stable; animate small details unless the scene is about motion.

For bilateral organic carrier scenes:

- Establish dominant/secondary lobe proportions first.
- Use symmetry only as a starting point; break it with pose, anisotropic strands, shell layers, or expression parameters.
- Detail focal bilateral features more than distant or peripheral shapes.
- Use material contrast to separate subsurface, woven, filament, metallic, accumulation, or prop materials.

For heightfield scenes:

- Build the heightfield silhouette first.
- Place the sun and horizon early.
- Use distance fog before adding tiny density-field details.
- Use instance/material IDs for variation but keep clump structure coherent.

For constructed objects:

- Define the base primitive and repetition system.
- Keep local coordinates and cell IDs available for color variation.
- Add seams, interstitial fields, bevels, or edge wear after the major tiling reads correctly.

## Debugging Ritual

When a shader fails visually, inspect fields instead of guessing:

1. Show coordinates as color.
2. Show the raw mask or distance.
3. Show object/material IDs.
4. Show normals.
5. Show depth or raymarch step count.
6. Show lighting terms separately.
7. Re-enable composition in layers.

Do not debug a full shaded image when the broken term can be displayed directly.

## Formula Verification From Visual Sources

When learning from a video, screenshot, or visual derivation, sample more than the final frame:

- Early frames reveal the primitive and coordinate system.
- Middle frames reveal IDs, local coordinates, repetition, and masking.
- Late frames reveal material, lighting, and detail breakup.
- Final frames often reveal temporal polish: subtle camera drift, procedural background blur, atmospheric motion, or tiny pose changes.
- Top-ranked code/formula frames often overrepresent final code, so also inspect chronological sheets.

Record the verified formula families, not copied source. For each family, capture:

- visual stage,
- coordinate space,
- scalar/vector field,
- IDs retained,
- visible debug output,
- final material/rendering use.

This prevents the common mistake of remembering the final image but missing the controllable formulas that built it.

## Common Failure Modes

- Aspect distortion: coordinates divide by `iResolution.x` and `iResolution.y` inconsistently.
- Dead noise: high-frequency noise is added before the large form works.
- Mushy SDFs: smooth unions use a radius larger than the feature scale.
- Lost material IDs: geometry distance and material assignment are separated incorrectly.
- Shadow acne: epsilon is too small or not scaled by hit distance.
- Flat composition: all objects have similar value and saturation.
- Expensive loops: nested fbm inside raymarching inside shadow calls.
- Temporal shimmer: noise or thresholds change abruptly with time or distance.

## Quality Passes

Run these passes in order:

- Shape pass: show only silhouettes and major masks.
- Value pass: convert mentally to grayscale; focal contrast should survive.
- Material pass: each material should be identifiable under simple light.
- Light pass: key direction, fill, shadow softness, and rim are deliberate.
- Atmosphere pass: distance, scale, and depth are clear.
- Motion pass: animation has stable phase and loops if requested.
- Performance pass: reduce octaves, step counts, shadow calls, and repeated map evaluations.

## Response Pattern for Codex

When presenting a shader, include:

- What the shader renders.
- The complete code or the precise patch.
- Debug controls or intermediate fields.
- Tuning knobs.
- Performance notes if raymarching, fbm, shadows, or volumetrics are involved.
