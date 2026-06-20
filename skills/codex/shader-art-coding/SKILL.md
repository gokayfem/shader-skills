---
name: shader-art-coding
description: Expert procedural shader art and GLSL/Shadertoy coding guidance. Use when an agent needs to write, improve, debug, explain, port, critique, or art-direct fragment shaders, raymarching shaders, signed-distance-field scenes, procedural heightfields, mathematical organic/object drawing, noise/fbm/domain-warp effects, lighting/material shaders, or compact real-time visual code.
license: MIT
metadata:
  hermes:
    tags: [shader, glsl, shadertoy, procedural-art, raymarching, sdf]
    category: development
---

# Shader Art Coding

## Operating Mode

Treat shader work as visual engineering. Start from the intended image, choose the smallest mathematical representation that can produce it, then grow the shader through inspectable stages.

For every substantial shader task:

1. State the visual target, camera/framing, dominant forms, material families, and motion.
2. Choose the rendering approach:
   - 2D analytic drawing for icons, posters, stylized flat scenes, UI effects, and compact masks.
   - Heightfield marching for scalar heightfields, sky radiance fields, fluids, density layers, and atmosphere.
   - SDF raymarching for sculpted 3D forms, modular structures, organic carriers, tunnels, and abstract scenes.
   - Fullscreen post/effect shader for transitions, glitches, feedback, color, and image processing.
3. Build the first visible image quickly: coordinates, background, one primitive, one light or color ramp.
4. Add debug views before adding complexity: distance field, IDs, normals, layer masks, step count, depth, material index, and noise octave contribution.
5. Refine in this order: proportions, composition, material separation, lighting, atmosphere, color grading, antialiasing, performance.
6. Prefer original compact helpers over copied shader code. Explain non-obvious formulas with the visual control they create.

## Reference Routing

Read only what the current task needs:

- `references/shader-workflow.md`: end-to-end process, task decomposition, build order, and review checklist.
- `references/math-patterns.md`: coordinates, shaping functions, SDFs, repetition, noise, fbm, domain warping, and palettes.
- `references/rendering-patterns.md`: raymarching, normals, AO, soft shadows, materials, fog, tonemapping, and antialiasing.
- `references/composition-and-debugging.md`: visual hierarchy, debug modes, readable iterations, and shader critique.
- `references/formula-cookbook.md`: visually checked formula stacks for radial modular repetition, heightfield synthesis, implicit organic composition, volumetric density, atmospheric fields, and material breakup.
- `references/formula-code-couples-sdf.md`: equation-to-GLSL pairs for implicit primitives, booleans, smoothing, local transforms, contour debugging, radial-cell lattices, bilateral feature fields, anisotropic strand fields, and shell intersections.
- `references/formula-code-couples-terrain.md`: equation-to-GLSL pairs for polynomial height patches, octave synthesis, finite-difference normals, horizon visibility, density fields, atmospheric coverage, accumulation masks, and distance LOD.
- `references/formula-code-couples-materials.md`: equation-to-GLSL pairs for palettes, scalar material masks, lighting terms, sparse stochastic spots, ID-based albedo variation, composition overlays, antialiasing, tonemapping, and temporal polish.
- `references/glsl-snippets.md`: original Shadertoy-compatible helper snippets and a compact complete template.

## Output Expectations

When writing shader code:

- Produce Shadertoy-compatible GLSL unless the user asks for another target.
- Include `mainImage(out vec4 fragColor, in vec2 fragCoord)` for complete Shadertoy shaders.
- Use stable coordinate normalization: center coordinates and preserve aspect ratio.
- Keep loops bounded by compile-time constants.
- Include 1-3 debug switches or clearly described places to inspect intermediate fields.
- Avoid texture dependencies unless the user provides textures or explicitly wants them.
- Finish with a short visual tuning guide: which constants control scale, softness, color, lighting, and motion.

When improving an existing shader:

- Identify the current rendering model and the likely bottlenecks.
- Preserve the user's intent and public interface.
- Improve one layer at a time: correctness, readability, visual quality, then speed.
- Call out risky changes such as altered coordinate scale, changed material IDs, or loop count increases.
