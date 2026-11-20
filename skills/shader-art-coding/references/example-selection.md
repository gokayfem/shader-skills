# Choosing examples and checking the result

Start with the row closest to the requested effect. Open the example source to inspect its inputs, coordinate system and pass structure. For a mixed scene, use one example for the main representation and borrow individual techniques from the others. Each row names source symbols and suggests a visible test to adapt to the brief.


## Contents

- [Graphic silhouettes and pattern](#graphic-silhouettes-and-pattern)
- [Fields and surfaces](#fields-and-surfaces)
- [Geometry and built structures](#geometry-and-built-structures)
- [Landscape and plants](#landscape-and-plants)
- [Ocean and air](#ocean-and-air)
- [Underwater subjects and optics](#underwater-subjects-and-optics)
- [Illumination](#illumination)
- [Effects with persistent state](#effects-with-persistent-state)
- [Finding the implementation behind a study](#finding-the-implementation-behind-a-study)
- [Choosing library modules inside an example](#choosing-library-modules-inside-an-example)
- [Joining examples in one deliverable](#joining-examples-in-one-deliverable)

For an exact helper, run `python3 scripts/glsl_tools.py examples fresnelDielectric` from the skill directory. It prints the library signature, the first direct call in each matching source, and any debug legend or modes declared in that file. A multipass fragment can inherit additional views from the assembled display pass. For a helper used indirectly, search its name in `glsl/`, then look up the calling helper.

### Graphic silhouettes and pattern

| Working source | Follow in the code | Review the adaptation |
|---|---|---|
| [sdf-2d-poster](../templates/sdf-2d-poster.glsl) | `ridge`, `plant` | Use the foliage distance slice to inspect the edge at the final pixel size. Compare the layer masks if a correct silhouette disappears in the composite. |
| [pattern-swatches](../templates/pattern-swatches.glsl) | `marble`, `crackedEarth`, `truchet` | Choose the construction that matches the motif. Follow a line across tile boundaries and reduce resolution to expose broken joins or flicker. |
| [streamline-press](../templates/streamline-press.glsl) | `flow`, `direction`, `streaks` | Compare the velocity view with the ink direction beside the cylinder. Check both sides of the obstacle and the low-speed region before changing paper grain. |

### Fields and surfaces

| Working source | Follow in the code | Review the adaptation |
|---|---|---|
| [noise-progression](../templates/noise-progression.glsl) | `noiseStudy` | Compare candidate fields at the same frequency. The unclipped scalar view reveals saturation that a grayscale swatch can hide. |
| [noise-terrain-map](../templates/noise-terrain-map.glsl) | `terrainMapFields` | Inspect height and river mask independently. Trace the river through the final biome map to see which features the mask actually controls. |
| [banded-agate](../templates/banded-agate.glsl) | `field`, `phase`, `filteredStripes` | Follow a band around the crystal pocket. Mode 3 isolates core, band region and rind. Resize to see whether the fine bands average smoothly. |
| [patinated-brass](../templates/patinated-brass.glsl) | `rolledGrain`, `sheetGrain` | Change the grain scale and watch the engraved edges. Surface breakup can change frequency while the object-local stamp geometry stays registered and deposits remain inside the stamped recesses. |

### Geometry and built structures

| Working source | Follow in the code | Review the adaptation |
|---|---|---|
| [raymarch-sdf](../templates/raymarch-sdf.glsl) | `map`, `material`, `render` | Use the distance slice for shape, IDs for ownership and steps for tracing. A hole visible in the distance slice needs a different fix from a hole that appears only on grazing rays. |
| [procedural-city](../templates/procedural-city.glsl) | `cityProfile`, `map`, `renderCity` | Track a parcel across cell boundaries in mode 3. Compare its silhouette in mode 4 with its windows in mode 5 before changing either the march bound or facade filtering. |
| [ribbed-wreck](../templates/ribbed-wreck.glsl) | `sandHeight`, `wreckMap` | Orbit until hull ribs overlap and inspect openings, sand contact and mode 4 at the same angle. A side silhouette can reveal a fused opening hidden in the front view. |

### Landscape and plants

| Working source | Follow in the code | Review the adaptation |
|---|---|---|
| [terrain](../templates/terrain.glsl) | `terrainHeight`, `map` | Compare the height field, grazing ridge normals and step count along the horizon. Inspect the shadow view before grading a dark slope. |
| [alpine-valley](../templates/alpine-valley.glsl) | `alpineValleyMask`, `alpineCeiling`, `traceAlpine` | Trace a skyline ray after altering ridge height. Check that the early-out ceiling still encloses the new terrain and that the material masks follow the edited slope. |
| [woodland-edge](../templates/woodland-edge.glsl) | `bankDistance`, `dampGround`, `stand` | Follow a cross-section from stream to clearing. Check which species the moisture field favors and whether understory changes beneath the same crowns. |
| [cloudline-forest](../templates/cloudline-forest.glsl) | `fir`, `projectedErrorPx` | Resize at a fixed time. Mode 4 shows which boughs switch to a crown proxy. Watch adjacent silhouettes through the transition with wind enabled. |

### Ocean and air

| Working source | Follow in the code | Review the adaptation |
|---|---|---|
| [ocean](../templates/ocean.glsl) | `traceOcean`, `oceanDetail`, `waveKeep` | Compare the nearest crest and the horizon in the normal and foam views. Move the camera to expose distant glint shimmer and missed grazing intersections. |
| [coastal-water](../templates/coastal-water.glsl) | `coastBed`, `coastWaves`, `traceCoast`, `renderCoast` | Follow a ray from water entry to bed. Compare mode 1 optical path, mode 5 Fresnel and mode 6 transmission at shallow and deep points before tuning water color. |
| [volumetric-clouds](../templates/volumetric-clouds.glsl) | `cloudDensity`, `render` | Use mode 5 for density/transmittance and mode 6 for illumination. Compare a sunlit edge and a thick core with the same exposure. |
| [tide-observatory](../templates/tide-observatory.glsl) | `landHeight`, `mainImage` | At fixed framing, vary tidal height and lunar phase separately. Mode 3 separates cloud coverage, moon illumination and land silhouette. |

### Underwater subjects and optics

| Working source | Follow in the code | Review the adaptation |
|---|---|---|
| [kelp-passage](../templates/kelp-passage.glsl) | `kelpCurtain`, `mantaShape` | Pause on a frond, then advance time. Compare tip and base motion and inspect the wildlife mask where fronds overlap the manta. |
| [sponge-shelf](../templates/sponge-shelf.glsl) | `seabed`, `shelfField` | Inspect the bore at an oblique angle in mode 2. Inner and outer normals should show a wall with thickness around an open mouth. |
| [comb-jelly](../templates/comb-jelly.glsl) | `waterField`, `combSurface` | Use mode 3 to track front rows, rear rows and thickness. Inspect the silhouette where the two interfaces approach one another. |
| [morpho](../templates/morpho.glsl) | `wingOutline`, `wingColor` | Sweep the light at fixed framing and exposure. The scale return should vary while veins, pigment margin and wing outline remain registered. |
| [lensed-disk](../templates/lensed-disk.glsl) | `diskTransfer`, `diskLight` | Use mode 3 to distinguish direct and higher-order disk images and the capture region. Check where the far side appears as the viewpoint changes. |

### Illumination

| Working source | Follow in the code | Review the adaptation |
|---|---|---|
| [pigment-chamber](../templates/pigment-chamber.glsl) | `clearSegment`, `panelLight`, `wallBounce` | Move the emitter at fixed exposure. Mode 6 separates direct, left-wall and right-wall contributions. Inspect a receiver beside the sphere where visibility changes. |

## Effects with persistent state

| Need | Follow the producer and consumer | State to preserve | Decisive check |
|---|---|---|---|
| Reaction diffusion | [Solver](../multipass/reaction-diffusion/buffer-a.glsl) → [display](../multipass/reaction-diffusion/image.glsl) | Buffer A reads the prior floating-point RG concentrations. Nearest texel reads and clamped neighbor coordinates define the boundary. The display samples the current buffer. | Restart from frame zero and compare at the same iteration. Inject a seed near an edge and inspect both concentrations to distinguish a solver fault from a palette change. |
| Cached mountain heights | [Atlas writer](../multipass/alpine-cache/buffer-a.glsl) → [height lookup](../multipass/alpine-cache/lookup.glsl) | R stores height. GB carry buffer dimensions. The lookup maps bounds to texel centers, samples linearly and evaluates the exact function outside the rectangle. | Compare against the uncached valley on either side of the atlas edge. Resize the buffer and inspect the first frame. If you make the terrain editable, add invalidation for the changed parameters. |
| Room radiosity | [Iteration](../multipass/room-radiosity/buffer-a.glsl) and [patch transport](../multipass/room-radiosity/transport.glsl) | A 20 × 4 texture stores diffuse patch radiance. Each iteration gathers from the previous one. The moving lamp changes the lighting solution. | Inspect direct-only, first bounce and converged views at fixed exposure. Move the lamp while paused and check when the old solution is replaced. |

**Diagnosing an atlas seam.** The [pass assembler](../scripts/build_alpine_cache.py) combines the writer and lookup fragments with [shared bounds](../multipass/alpine-cache/domain.glsl) and the exact height function from the alpine template. From the skill directory, run `python3 scripts/build_alpine_cache.py <output-directory>` for standalone Buffer A and Image passes. Plot signed cached-minus-exact height along all four edges, sampling boundary texel centers and points halfway between them. Expect agreement at the centers within storage precision. Between centers, linear interpolation can leave a residual even with the correct coordinates. Compare normals at fixed lighting if heights agree but the seam remains visible. The [offline viewer](../viewer/viewer.js) uses a fixed 1536 × 1536 atlas. To exercise invalidation, change the atlas allocation itself, run the writer before display, and inspect the stored dimensions on the first new frame.

## Finding the implementation behind a study

Use [subject routing](subject-routing.md) for subjects without a dedicated template and the [library index](library-index.md) for exact helper signatures. For a combination, write down what crosses between examples (world units, hit depth, material ID, velocity, transmittance, or previous-frame state). Test that interface before adjusting surface detail.

In a full checkout, open `gallery/studies.html` for interactive comparisons, `worlds/README.md` for subsystem entry points and `worlds/catalog.md` for the source inventory. Follow the relevant implementation into the dive or landscape engine for mesh generation, animation, wildlife, materials, weather, particles, ocean, lighting, streaming, baking, editor controls or profiling. In the installed skill, [world rendering](world-rendering.md) describes the portable studies and [frame budgets](frame-budgets.md) covers timing and quality settings. Identify which engine inputs or passes the target host supplies before adapting a technique.

## Choosing library modules inside an example

Read the [function index](library-index.md) for signatures. Use the groups below when a template has the right representation but one stage needs replacing. The module source documents units, assumptions and dependencies.

| Operation | Module sources | What to verify at the call site |
|---|---|---|
| Place or trace a feature | [common](../glsl/common.glsl), [camera](../glsl/camera.glsl), [intersect](../glsl/intersect.glsl), [raymarch](../glsl/raymarch.glsl) | Check world versus view space, whether ray direction has unit length, and whether returned distances use the same ray origin. Inspect hit depth or step count. |
| Construct a silhouette | [sdf2d](../glsl/sdf2d.glsl), [sdf3d](../glsl/sdf3d.glsl), [domain](../glsl/domain.glsl), [operators](../glsl/operators.glsl) | Evaluate the primitive in local space. Check sign, bounds after deformation and neighboring copies. Inspect a slice through the join. |
| Distribute and filter detail | [hash](../glsl/hash.glsl), [noise](../glsl/noise.glsl), [pattern](../glsl/pattern.glsl), [architecture](../glsl/architecture.glsl) | Choose stable IDs for variation. Set the frequency in the intended coordinate space and filter against the pixel footprint. Track one parcel or motif during a camera move. |
| Transport light | [shading](../glsl/shading.glsl), [atmosphere](../glsl/atmosphere.glsl), [volume](../glsl/volume.glsl), [water](../glsl/water.glsl), [relativity](../glsl/relativity.glsl) | Follow the path that carries light to the camera. Check which functions expect a direction toward the light, a path length, an extinction coefficient or a dimensionless orbital radius. Isolate each contribution before grading. |
| Finish and inspect the image | [color](../glsl/color.glsl), [tonemap](../glsl/tonemap.glsl), [post](../glsl/post.glsl), [debug](../glsl/debug.glsl) | Trace linear and encoded values through exposure, tone mapping, antialiasing and display. Compare an intermediate field with the final image at the same pixel. |

## Joining examples in one deliverable

**A rocky inlet with a flooded street.** Start with `renderCoast` for the water interface and `cityProfile` for parcel massing. Compare the nearest positive water, building and exposed-land hits along the same primary ray. For transmitted light, evaluate submerged geometry along the refracted ray from the water entry point. Use the traveled underwater distance for attenuation. Trace the building on the reflected path too, so the wall above the water can appear in its reflection. Borrow the building mass without importing the city’s repeated parcels and ground plane. Test with one striped box partly underwater. Freeze time and translate the camera to check the dry silhouette, bent submerged stripes and reflected wall. Then hold the camera still and advance the waves before adding facades and foam.

**An engraved brass emblem for a moving camera.** Use the 2D field from the chosen motif to define the groove and the stamped-sheet study to turn groove depth into a normal. Keep the design in the object's coordinates. Compute its pixel footprint at the projected size. First hold the camera still and move the light to inspect relief. Then hold the light still and move the camera away. If thin strokes sparkle, inspect the filtered coverage before increasing roughness.

**A fly-through with cached terrain and trees.** Use the alpine lookup for heights and the woodland study for founder positions and wetness. Query terrain at each founder's world coordinates. The layered fir study demonstrates projected-size detail selection in a composed view. A free-moving camera needs world-space trees projected by its own camera. Fly the same path with exact and cached heights, across the atlas edge and back. Compare root positions, silhouette changes and cold versus warm frame times separately.

**A glowing translucent swimmer.** The comb jelly gives front and back optical events. The kelp study gives occluding silhouettes and suspended particles. To let the camera swim around the subject, derive positions along each camera ray, order the water segments and surfaces by depth, and apply attenuation per segment. Start with a bright stripe behind the swimmer and one frond crossing in front. Test which stripe segments transmit, which frond segments hide the animal, and how thickness changes at the outline before grading the water.
