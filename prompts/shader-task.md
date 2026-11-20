# Prompt for shader work

Copy the block below and replace the first line with your brief. Include any reference, existing source, output format or hardware budget that matters.

```text
My brief: [what to make, change, diagnose, optimize or explain].

Use the shader-art-coding skill. Read SKILL.md and references/example-selection.md inside the skill. Match each important part of my brief to the closest working example. Open its source and identify the function, pass or data flow you will adapt. Use references/subject-routing.md for subjects outside the examples and references/library-index.md to find tested building blocks.

Choose the representation that fits my target host. Consider the single-pass templates, multipass studies and, in a full checkout, the gallery and the two WebGPU worlds. For engine work follow worlds/README.md and worlds/catalog.md to the implementation. Call out any host feature the adaptation depends on. If a technique is new, explain why the closest example needs extending.

For a new image, establish framing, silhouette and scale before materials and lighting. For a change, capture the existing behavior and preserve the requested interface. For a diagnosis or explanation, trace the relevant coordinates, signals or passes through the source. For optimization, measure the same workload and compare image quality before and after.

Choose checks for the features in my brief. Examples include a flat-color blockout for layout, normals and IDs for shape, a mask for material placement, a light sweep for optics, consecutive frames for motion, and reset or camera movement for feedback and caches. Check interactions between features as well as each feature alone. Compare against my reference at the requested resolution or timestamps when supplied.

Deliver the requested artifact or answer. For runnable work, give the entry point, run the checks for its host and inspect the result. Summarize which examples informed it, what the checks actually showed, and any remaining mismatch or untested behavior.
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
