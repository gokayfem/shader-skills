# Choosing examples and checking the result

Start with the row closest to the requested effect. Open the example source to inspect its inputs, coordinate system and pass structure. For a mixed scene, use one example for the main representation and borrow individual techniques from the others. The final column suggests a visible test to adapt to the brief.

| Need | Starting examples | What to inspect |
|---|---|---|
| Flat artwork, patterns and fields | [sdf-2d-poster](../templates/sdf-2d-poster.glsl), [pattern-swatches](../templates/pattern-swatches.glsl), [noise-progression](../templates/noise-progression.glsl), [noise-terrain-map](../templates/noise-terrain-map.glsl), [streamline-press](../templates/streamline-press.glsl) | Inspect silhouette and coverage at delivery size. For flow engraving, follow the ink along a streamline and around the obstacle. |
| Solid form, architecture and surfaces | [raymarch-sdf](../templates/raymarch-sdf.glsl), [procedural-city](../templates/procedural-city.glsl), [ribbed-wreck](../templates/ribbed-wreck.glsl), [banded-agate](../templates/banded-agate.glsl), [patinated-brass](../templates/patinated-brass.glsl) | Orbit a solid or inspect its normals. Check repeated facades at oblique angles and distant scales. On materials, isolate growth bands, relief or wear from illumination. |
| Terrain and vegetation | [terrain](../templates/terrain.glsl), [alpine-valley](../templates/alpine-valley.glsl), [woodland-edge](../templates/woodland-edge.glsl), [cloudline-forest](../templates/cloudline-forest.glsl) | Inspect the height or habitat mask alongside the image. Move across a ridge or detail boundary and check the silhouette, grounding and transition. |
| Water, weather and sky | [ocean](../templates/ocean.glsl), [coastal-water](../templates/coastal-water.glsl), [tide-observatory](../templates/tide-observatory.glsl), [volumetric-clouds](../templates/volumetric-clouds.glsl) | Compare water near shore and at grazing angles. Isolate density and transmittance for clouds. For a time-of-day shot, hold the camera still and scrub time. |
| Life, underwater structures and optics | [kelp-passage](../templates/kelp-passage.glsl), [sponge-shelf](../templates/sponge-shelf.glsl), [morpho](../templates/morpho.glsl), [comb-jelly](../templates/comb-jelly.glsl), [lensed-disk](../templates/lensed-disk.glsl) | Pick the mechanism that identifies the subject. Move the light across a wing, look into the sponge bore or follow the far side of the disk as the view changes. |
| Direct and indirect illumination | [pigment-chamber](../templates/pigment-chamber.glsl) | Move the emitter, isolate direct and bounced light, then inspect the receiver and its shadow. |

## Effects with persistent state

| Need | Source | What to inspect |
|---|---|---|
| Evolving chemical patterns | [Reaction diffusion](../multipass/reaction-diffusion/) | Reset, step, and compare later frames. Verify which pass reads the previous state. |
| Reusing terrain across frames | [Alpine cache](../multipass/alpine-cache/) | Cross the cache boundary and compare against the procedural fallback. |
| Iterated diffuse transport | [Room radiosity](../multipass/room-radiosity/) | Move the lamp, check reset and convergence, then isolate direct illumination. |

## Finding the implementation behind a study

Use [subject routing](subject-routing.md) for subjects without a dedicated template and the [library index](library-index.md) for exact helper signatures. For a combination, write down what crosses between examples (world units, hit depth, material ID, velocity, transmittance, or previous-frame state). Test that interface before adjusting surface detail.

In a full checkout, open `gallery/studies.html` for interactive comparisons, `worlds/README.md` for subsystem entry points and `worlds/catalog.md` for the source inventory. Follow the relevant implementation into the dive or landscape engine for mesh generation, animation, wildlife, materials, weather, particles, ocean, lighting, streaming, baking, editor controls or profiling. In the installed skill, [world rendering](world-rendering.md) describes the portable studies and [frame budgets](frame-budgets.md) covers timing and quality settings. Identify which engine inputs or passes the target host supplies before adapting a technique.
