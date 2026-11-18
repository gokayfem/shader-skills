# Keeping shader worlds within a frame budget

A 60 Hz display has 16.67 ms between refreshes. A 30 Hz target has 33.33 ms. Measure a repeatable camera path through the richest area, then turn toward it from a cold start. Separate time spent generating or streaming the world from the cost of drawing it once resident. Record median and p95 frame intervals, per-pass GPU time where available, actual output size, and quality settings. A single stationary FPS reading hides both camera-dependent cost and hitches.

## Choose the first experiment

| Symptom | Change one thing | Interpret the result |
|---|---|---|
| Looking into cloud or dense shafts slows the view | Reduce width and height while keeping the shot, steps and simulation fixed | A large time saving points toward work per pixel. Isolate the volume from geometry before lowering the entire image. |
| Spinning toward dense foliage slows the frame even at low resolution | Compare visible cells, draw calls, selected clusters and queue drops on the same turn | More draws suggest submission pressure. Many tiny triangles can still cost raster work. Any dropped required work invalidates a speed comparison. |
| First visits hitch, repeat visits are smooth | Record requested bytes, upload bytes per frame and page residency while replaying the same route | Bound installs per frame, prioritize parents and retain coarse coverage during refinement. A steady-state shader change cannot remove a network stall. |
| Frame time jumps only when an effect starts | Capture allocation and pipeline creation on the triggering frame | Keep idle memory small, then prewarm shortly before a predictable burst when first-use latency matters. |
| GPU timings fit but frames arrive late | Inspect CPU update, draw submission and browser scheduling independently | Check repeated instance lookups and synchronous GPU readback before cutting visual detail. Frame intervals include waiting for display refresh. |

## Underwater scene

**Cull ranges, then batch what survives.** The world builder stores members of each mesh or plant family in cell order. Each cell has a start offset and its occupied height interval. The draw loop rejects cells by squared distance and a conservative view test, then merges adjacent visible cells in a row into one indexed or instanced draw. The final sentinel flushes the last run. Expand bounds for the full mesh and vertex animation. To test a port, freeze the same frame with culling disabled, then pan across a cell boundary with strong wind. Compare coverage, draw count and submitted instances. [Cell traversal and draws](https://github.com/gokayfem/shader-skills/blob/707e7cd/worlds/dive/source/src/main.js#L395-L480)

**Spend geometry on blades near the camera.** At full detail, the large kelp uses 9 rows out to distance 24, 5 rows from 24 to 58, and a thinned set of 3-row blades out to 150. The shared instance data lets the host change the vertex pipeline without generating a new blade mesh per plant each frame. Some props also swap to a coarser mesh. Keep the root and large bending motion consistent across tiers. Review a slow forward swim for density pops. [Leaf draws](https://github.com/gokayfem/shader-skills/blob/707e7cd/worlds/dive/source/src/main.js#L475-L480)

**Render shafts at half width and height.** This gives one march per four full-resolution pixels. The dive stores a depth cue in alpha and weights a 3 × 3 neighborhood by relative depth difference during blur. That preserves boundaries between dark wreck ribs and bright open water. Low detail also lowers the shaft loop from 34 samples to 12. These two changes reduce different dimensions of work. Inspect thin silhouettes with a light behind them after each change. [Blur shader](https://github.com/gokayfem/shader-skills/blob/707e7cd/worlds/dive/source/src/shaders.js#L725-L739) · [Quality constants](https://github.com/gokayfem/shader-skills/blob/707e7cd/worlds/dive/source/src/quality.js)

**Schedule passes by what actually changes.** Detail textures and the static shadow map are baked once. Animated water and its four lower mips are updated during rendering. Bloom uses six successively smaller targets and adds the result back up the chain. Target allocation and bind-group creation sit behind a size-change guard. A port with moving shadow casters needs a dynamic shadow path or explicit invalidation. Check that a moving actor's shadow follows it while stationary geometry reuses its bake. [Bakes and target allocation](https://github.com/gokayfem/shader-skills/blob/707e7cd/worlds/dive/source/src/main.js#L302-L371)

**Use GPU headroom to choose pixel count.** The dive reads timestamps asynchronously from six slots and skips sampling when all are busy. Outside a 9.8–13.6 ms GPU band it requests a pixel-count multiplier of 11.7 divided by measured GPU milliseconds, clamped to 0.5–1.7. Width scales with the square root of the pixel ratio. Under load it first reduces pixels to an intermediate budget, then draw distance, then pixels again. Terrain keeps its own range. Without timestamps it waits for sustained slow frame intervals and backs off after an unsuccessful recovery. These thresholds belong to the dive's 60 fps workload. [Timer ring](https://github.com/gokayfem/shader-skills/blob/707e7cd/worlds/dive/source/src/main.js#L372-L389) · [Controller](https://github.com/gokayfem/shader-skills/blob/707e7cd/worlds/dive/source/src/main.js#L535-L551)

## Landscape renderer

**Reject children before reserving GPU queue space.** Apply the existing projected-error test to the child before appending it. Use the same transformed bounds, animation margins and threshold as the traversal. This removes atomic work and prevents a full queue from silently dropping visible nodes. In the supplied million-patch overview, the old queue dropped over ten million entries in one measured frame. The new run dropped zero. Include shadow-list overflow and selected geometry in the acceptance check. [Traversal and regression measurements](https://github.com/gokayfem/shader-skills/blob/707e7cd/worlds/landscape-engine/docs/vegetation-performance.md#reject-unnecessary-hierarchy-work-before-queueing-it)

**Bound rasterization at pixel centers.** For tiny triangles the scan rectangle can cost more than the pixels shaded. Build inclusive pixel-center bounds from the same quantized, 8-bit-subpixel coordinates as the edge tests. Verify ties, shared edges, viewport clipping, masked surfaces and depth-only paths. The renderer compared 3,430,912 sample positions with an exhaustive rasterizer. Its paired 1080p native Vulkan forest measurements on an RTX 5080 showed the combined changes taking ground-view GPU time from 5.938 to 5.357 ms and a large overview from 4.271 to 3.547 ms. Those are resident-scene GPU medians from the bundled audit. [Raster details and results](https://github.com/gokayfem/shader-skills/blob/707e7cd/worlds/landscape-engine/docs/vegetation-performance.md)

**Use the right depth pyramid for each query.** Occlusion culling summarizes farthest depth for conservative rejection. GI screen traces use nearest depth and align mip coordinates with screen pixels. Reusing one reduction for both operations can reject visible geometry or miss a ray hit. Test an archway with empty space beside a thin foreground object, then compare against culling disabled or a full-resolution trace. [GI pyramid implementation](https://github.com/gokayfem/shader-skills/blob/707e7cd/worlds/landscape-engine/crates/solstice-render/src/gi/depth_pyramid.rs)

**Keep a complete coarse surface during streaming.** Fetch parent dependencies first. Install refinement once the required pages are available together. Bound installation work per frame so a burst of completed downloads leaves time for rendering. The engine describes a roughly 1 MB/frame install allowance. Test a cold turn, constrained bandwidth and eviction while returning to the original viewpoint. Track time to first usable frame and time to full detail separately. [Residency, priorities and budgets](https://github.com/gokayfem/shader-skills/blob/707e7cd/worlds/landscape-engine/docs/streaming.md)

**Cache by actual shader inputs.** The sky-view table depends on sun direction and effective viewer radius. The renderer compares parameter bytes with the previous build. Camera rotation can reuse the table, while a change in sun or effective height invalidates it. Compare against forced reconstruction under all three motions. The same principle applies to the gallery's room transport buffer when moving its lamp. [Sky cache](https://github.com/gokayfem/shader-skills/blob/707e7cd/worlds/landscape-engine/docs/vegetation-performance.md#reuse-an-unchanged-atmosphere-table)

**Separate configured capacity from allocated storage.** The particle pool delays simulation-buffer allocation until an effect first needs it, then retains storage across restarts. The documented idle saving is 27.58 MiB for the default 262,144-particle capacity. Measure the first burst and restart as well as idle memory. [Particle storage](https://github.com/gokayfem/shader-skills/blob/707e7cd/worlds/landscape-engine/docs/vegetation-performance.md#allocate-particle-storage-when-an-effect-needs-it)

**Remove CPU work from repeated updates.** Index the pending transform record per instance and overwrite it on subsequent moves in the same frame. Preserve previous-frame transforms for motion vectors. For streaming, heapify candidates and extract only the pages the current budget permits. Preserve tie ordering so identical camera paths yield identical request traces. [CPU workloads and trace validation](https://github.com/gokayfem/shader-skills/blob/707e7cd/worlds/landscape-engine/docs/performance-audit.md#avoidable-per-frame-cpu-work)

**Keep startup out of automatic quality decisions.** The landscape browser viewer ignores loading and four initial non-loading reports. It steps down after three reports below 28 fps, or two below 14. Its last high-DPI step caps pixels per CSS pixel at one. This controller intentionally holds the chosen lower tier. The underwater controller instead probes for recovery. Choose between these behaviors based on whether settings transitions or sustained loss of detail are more visible in your scene. [Browser preset controller](https://github.com/gokayfem/shader-skills/blob/707e7cd/worlds/landscape-engine/apps/viewer/web/auto-quality.js)

## Reproduce the gallery resolution experiment

From a full checkout with Playwright and desktop Chrome installed, run:

```sh
python3 tests/profile_resolution.py --chrome ribbed-wreck kelp-passage volumetric-clouds
```

The probe pauses animation at the scene's initial time, warms up each size, and takes measurements in small/large/large/small order. Each measurement draws the image pass and reads one pixel to wait for completion. The times include CPU and readback overhead. They exclude simulation stepping, startup and display presentation. Omit `--chrome` for a headless Chromium run and inspect the reported renderer.

| Fixed shot, Apple M4 / Metal, Chrome 157 | 800 × 450 median / p95 | 480 × 270 median / p95 |
|---|---:|---:|
| Ribbed wreck | 1.70 / 2.40 ms | 1.00 / 1.90 ms |
| Kelp passage | 1.00 / 1.60 ms | 0.60 / 1.20 ms |
| Volumetric clouds | 15.00 / 17.10 ms | 5.55 / 7.30 ms |

Each cell summarizes 60 draws. The larger image has 2.78 times as many pixels. The cloud median grew by 2.70 times, while the wreck median grew by 1.70 times. The cloud result makes reducing the volume pass resolution worth testing before simplifying silhouettes. Compare moving cloud edges and sun shafts at the target display size before adopting the lower resolution.

In the offline viewer, select **Adaptive · target 60 fps** or **Adaptive · target 30 fps** to exercise dynamic resolution. The viewer takes the median of 45 displayed-frame intervals and waits longer to increase resolution after a slowdown. These modes are frame-time targets. At the lowest resolution, an expensive shader can still exceed its budget.
