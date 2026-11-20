# Prompt for shader work

Copy the block below and replace the first line with your brief. Include any reference, existing source, output format or hardware budget that matters.

```text
My brief: [what to make, change, diagnose, optimize or explain].

Use the shader-art-coding skill. Keep my requested host, interface, inputs and deliverable. If I supplied a reference, identify which visible features define the result and which dimensions or timings you must infer. For a still, use recognizable landmarks, silhouette and relative size. For a clip, use timestamps and describe what moves relative to what. For existing code, reproduce the relevant behavior before editing it.

Find the working examples. Start at SKILL.md and references/example-selection.md. Open the closest source and follow its entry point to the function or pass responsible for the effect. Use subject-routing.md and library-index.md in references/ to find forms and helpers beyond the templates. For a complex brief, map each major feature to a source symbol, the input it needs and an observation that would show it is working. Say when you introduce a new technique. Keep the research proportional to the request.

Use targeted search first. From the skill directory, run `python3 scripts/glsl_tools.py search <terms>` to find a helper, then `python3 scripts/glsl_tools.py examples <exactName>` for direct uses and available debug views. Open the matching source at the reported line. For host-side wiring, run `rg -n -w <symbol>` inside the relevant world’s source directory and follow its definition and consumers. For a visual brief without a known symbol, start with the example guide.

Choose an implementation the host can run. A poster may only need a 2D field. Solid silhouettes may need intersections or bounded SDF tracing. Terrain needs a height query that agrees with its tracer. A cloud needs density integrated along a ray. History, cached fields and iterated transport need explicit buffer ownership. In the full checkout, use worlds/README.md and worlds/catalog.md to find host-side code when a technique depends on generated geometry, animation, streaming, a compute pass or an editor. Trace that dependency before promising it in a fragment shader.

When combining examples, identify what each one owns. Use one camera and time convention for the shot. Agree on coordinate space, scale, hit distance, normals and material IDs where geometry meets. For a background seen through water or air, establish which surface ends the ray and how much light reaches it. For multipass work, record what each channel stores, who writes it, who reads it, the resolution, filtering and reset conditions. Only carry over contracts that apply to this task.

Build and inspect the earliest version that can reveal the main failure. For a scene, render the large shapes and nearest-hit visibility before adding material detail. For a material, inspect its field and masks before lighting it. For simulation, inspect the first state and one update before tuning its palette. For a bug, find the first intermediate value that diverges from the working example. For an explanation, trace a representative pixel, object or frame through the source.

Choose a decisive comparison. Hold the other variables fixed. Change light angle to test an optical response, camera position to test grounding and parallax, output size to test filtering, time to test motion, or input state to test invalidation. Read each template’s own DEBUG_MODE branches. Include the expected observation, what you actually saw, and the correction if they differ. Match reference crops in framing, timestamp, resolution and exposure where those affect the comparison.

For performance work, capture the baseline at the requested quality, device, resolution and camera path. Change one cost driver at a time. Report the timing source, warm-up and before/after frame time alongside a comparable image. Distinguish first visit from steady state for streaming. If only software rendering is available, identify it and leave target-device timing open.

Deliver the requested output. For Shadertoy, bundle the source with scripts/glsl_tools.py and check the bundle. For multipass, include ordered passes and bindings. For WebGPU work, exercise the actual host path. State separately what compiled, what ran, and what you visually inspected. Give the relevant source anchors, useful art controls, and any visible mismatch or unverified condition.
```

## Example briefs

Replace the first line with a brief like one of these. The details tell the agent which behavior you want to see.

- **2D print.** Make a two-ink engraving of flow past a cylinder. The ink should follow the field around the obstacle. Show the field direction and the finished print at phone resolution.
- **Material.** Build a stamped copper panel. Keep the stamp legible as the surface pattern repeats. Show the wear mask, a grazing-light view and the finished material.
- **Landscape.** Recreate my valley reference with a cabin and a stream. Match the roof and shoreline positions in the reference frame. Move sideways to check foundation contact, occlusion and water depth.
- **Optics.** Make a butterfly wing that flashes blue as the light turns. Keep its outline and veins readable. Show the same wing under three light angles at fixed exposure.
- **Animation and state.** Grow coral-like chemical patterns in a two-pass viewer. Give the channel bindings and show the same seed at reset, after early growth and after the pattern fills the frame.
- **Debugging.** Diagnose the holes in my twisted SDF when the camera grazes its surface. Trace the distance estimate through the deformation and show the failing view before and after the fix.
- **Engine performance.** Investigate the slowdown on the first flight into a forest. Trace visibility, residency and foliage detail through the landscape source. Compare first visit and revisit on the same camera path and report the measurement conditions.

## When the brief targets a full WebGPU world

Append the paragraph that fits the work. The [world subsystem map](../worlds/README.md) links to the host code, shaders and measurements for both worlds.

**Dive, wildlife and underwater light**

> Follow the subject from its generator in `worlds/dive/source/src/world.js` through the material or shaft pass in `shaders.js` and the draw loop in `main.js`. Choose two reproducible dive viewpoints, including one where the subject crosses open water. Hold geometry and camera fixed while testing attenuation or shafts. For silhouette or swimming-space changes, inspect both the shape and its approach. If adjusting quality, identify the allocation or sample count selected in `quality.js` and record the actual render size.

**Landscape, atmosphere and streaming**

> Start with the scene and identify where the relevant data is authored, baked, streamed and consumed by a render pass. Use the source map to follow only the stages the change touches. For missing detail during travel, compare the first visit, a stationary hold and the return along the same path. For storm lighting or wetness, record the weather state and test the transition after rain stops. For foliage, hold wind phase fixed while checking projected coverage, then restore motion to inspect popping. Deliver a reproducible camera path and the evidence that distinguishes the suspected subsystem from its neighbors.

**Actors, scene editing or interchange**

> Trace one representative object through the source scene or import, its transform hierarchy, the editor/runtime operation and the resulting frame. Check axis convention, units, pivot, parent transform and animation time at the stage where the mismatch first appears. For a contact problem, inspect the rendered pose alongside collision geometry at contact and one frame on either side. For a save/load issue, repeat the operation after reopening the same scene. Report which boundary loses the expected value and how you verified the correction.
