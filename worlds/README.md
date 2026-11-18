# Exploring the two WebGPU worlds

Open the [dive](dive/index.html) locally in a WebGPU desktop browser. The page generates its own geometry, materials and sound. After loading, press **Dive in**, swim with WASD, ascend or descend with E/Q, and use 1–6 to jump between viewpoints. Press H to inspect performance. Choose Low in the menu to try a smaller shadow map and fewer shaft samples.

For the landscape, install Rust with rustup, then run these commands from the checkout. The pinned toolchain includes the WebAssembly target. The first build downloads Rust dependencies, bakes a scene and surfaces, and compiles the viewer, gameplay and editor modules.

```sh
cd worlds/landscape-engine
cargo run --release -p bake -- --quality web --output assets/web.solstice
bash scripts/build-web.sh
python3 scripts/serve.py
```

Open http://localhost:8765/?scene=web to load the baked temple. See the [engine instructions](landscape-engine/README.md) for scene selection and native controls. Additional high-resolution scene packs stream separately. The [editor guide](landscape-engine/docs/editor.md) covers building a project in the browser. The [offline WebGL2 studies](../gallery/studies.html) include smaller editable implementations of seven related scenes and lighting experiments.

## From a pixel to its implementation

| What to investigate | Where to look and what to change |
|---|---|
| Canyon silhouettes and swimmable arches | [World generation](dive/source/src/world.js), beginning at MESAS, ARCHES, HOLES and CLEARS. Widen a tunnel and its clearance together. Check both the opening and its approach from a swimming viewpoint. |
| Ground, wreck and wildlife geometry | In [world.js](dive/source/src/world.js), follow buildTerrain, meshShip, meshFluke, meshManta and meshFish. The lathe and tube builders serve different silhouettes. Change a profile before changing surface noise. |
| Kelp, grotto and sponges | Trace makeWorld placement through meshLathe and meshCoral into the W_LEAF and W_SOLID paths in [shaders.js](dive/source/src/shaders.js). Inspect silhouettes against open water and the overlap against rock. |
| Light shafts and surface detail | In [shaders.js](dive/source/src/shaders.js), follow W_WATER, W_SURF, W_RAYS, W_RBLUR and W_POST. Inspect the separate shaft image before increasing gain. Compare silhouettes at the waterline and in shadow. |
| Dive GPU and memory budget | [quality.js](dive/source/src/quality.js) selects geometry thinning, a 2048/4096 shadow map, 256/512 water resolution, 12/34 shaft steps and 1/4 samples before construction. [main.js](dive/source/src/main.js) connects these budgets to allocations and dispatches. |
| A navigable, audible dive | [main.js](dive/source/src/main.js) owns input, views and frame submission. [audio.js](dive/source/src/audio.js) synthesizes the underwater ambience. Use the same viewpoint with sound switched on and off to judge timing independently of the image. |
| Virtual geometry | Start with [culling, raster and resolve](landscape-engine/crates/solstice-render/src/vg), then inspect the matching [GPU passes](landscape-engine/crates/solstice-render/shaders/vg). Compare cluster debug coloration with the final shaded silhouette. |
| Texture streaming | Follow [GPU feedback and CPU residency](landscape-engine/crates/solstice-render/src/streaming) through a cold camera move and revisit. The [streaming guide](landscape-engine/docs/streaming.md) describes range requests and which data can arrive independently. |
| Indirect light | Compare [depth hierarchy, screen trace and volume](landscape-engine/crates/solstice-render/src/gi) with the actual [GI shader passes](landscape-engine/crates/solstice-render/shaders/gi). Test a receiver with the light source moving off screen. |
| Ocean | Follow [spectrum and waves through surface, seabed and waterline](landscape-engine/crates/solstice-render/src/ocean). Compare distant wave filtering with a near-surface camera. Read the [ocean notes](landscape-engine/docs/ocean.md) for the pass sequence. |
| Sky and storms | [Time of day](landscape-engine/crates/solstice-render/src/time_of_day.rs), [cloud shaders](landscape-engine/crates/solstice-render/shaders/clouds) and [weather effects](landscape-engine/docs/weather-effects.md) cover changing illumination, precipitation and lightning. Test wetness after rain stops. |
| Vegetation and motion | Inspect [foliage baking](landscape-engine/crates/solstice-bake/src/foliage.rs), [cards](landscape-engine/crates/solstice-render/src/cards.rs), [wind shaders](landscape-engine/crates/solstice-render/shaders/wind) and the [vegetation measurements](landscape-engine/docs/vegetation-performance.md). Move slowly through the detail transition and watch canopy coverage. |
| Persistent terrain marks | The [ruts guide](landscape-engine/docs/ruts.md) and [wetness guide](landscape-engine/docs/wetness.md) describe how local surface memory affects shading. Test a wheel path during rain and again as the ground dries. |
| Actors and scene interaction | Walk from the [scene definitions](landscape-engine/scenes) into [animation](landscape-engine/crates/solstice-anim), [physics](landscape-engine/crates/solstice-physics), [vehicles](landscape-engine/crates/solstice-vehicle) and the [gameplay guide](landscape-engine/docs/gameplay.md). |
| Editing, interchange and performance | See [browser editor API](landscape-engine/docs/editor-api.md), [Blender tools](landscape-engine/tools/blender), [gameplay benchmarks](landscape-engine/tools/gameplay-bench) and the [GPU performance audit](landscape-engine/docs/performance-audit.md). Use the captured workload and quality settings when comparing changes. |

## Complete inventory

[Browse every tracked file by directory](catalog.md), including the screenshots, editor sources, build scripts, tests, scene descriptions, licenses and shader passes. The two source directories retain the exact Git trees listed in [snapshots.json](snapshots.json). Run `python3 tests/check_worlds.py` from the repo root to verify them.
