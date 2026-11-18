# Vegetation performance audit

This follows the correctness, page packing and streaming work in [PR #85](https://github.com/SamG-Coder/solstice/pull/85). The baseline is `a07a4b48487cf8c1f907e7e6935a7b634793a8df`, including those fixes.

## Workload and measurement

The new `solstice-render` example `vegetation_stress` builds actual procedural meadow geometry and oak, birch and fir trees. Each tree has wood and leaf instances. Grass patches are two metres across; transforms and species selection use a fixed seed. The ground, textures, materials, geometry, lighting and camera paths are identical between versions.

All runs use 1920 × 1080, release builds and the default high-quality settings: render scale 1, LOD error 1, temporal accumulation, clouds, eight probe rays, animated vegetation and no speck approximation. The renderer warms up for 48 frames and advances time by exactly 1/60 second per frame. Comparisons measure the next 240 frames. Baseline/candidate order reverses in the second repetition.

Hardware: NVIDIA RTX 5080, Ryzen 7 9800X3D, Windows, native Vulkan through wgpu. These are native measurements of the shared renderer; they are not browser FPS measurements.

* GPU time is the sum of the renderer's per-pass timestamp measurements.
* CPU time covers `Renderer::render`, including command encoding and submission, but excludes the explicit GPU wait. Wall time includes that wait. Neither number includes procedural generation or shader/pipeline startup.
* `scene_gpu_bytes` counts the existing scene memory categories, not all GPU allocations. Process working set and private commit are sampled during measured frames; they are not VRAM measurements or whole-process lifetime peaks.
* Geometry is fully resident. These runs test instance count, culling, rasterization and steady-state memory. They do not model network latency or a forest of unique assets. Three tree variants and one meadow mesh share geometry across all instances.
* Source triangle counts include every instance before LOD/visibility selection. Selected triangle counts describe what reaches rasterization, not necessarily how many triangles cover a pixel.
* Each run writes per-frame CSV data and a final PNG. Any nonzero main-view or shadow dropped-cluster count fails the run, so overflow cannot silently count as a performance gain.

## Results

Medians across 480 measured frames per version (two 240-frame runs). Lower times are better.

| Scene and view | GPU before | GPU after | Reduction | CPU before | CPU after |
| --- | ---: | ---: | ---: | ---: | ---: |
| 16,384 grass patches + 1,024 trees, ground | 5.938 ms | 5.357 ms | 9.8% | 0.378 ms | 0.372 ms |
| Same forest, walking/turning | 5.976 ms | 5.426 ms | 9.2% | 0.398 ms | 0.378 ms |
| 262,144 grass patches + 16,384 trees, overview | 4.271 ms | 3.547 ms | 17.0% | 0.378 ms | 0.371 ms |

The first raster pass fell from 3.094 to 2.432 ms in the ground view, and from 2.212 to 1.744 ms in the large overview. The overview's first cull fell from 0.622 to 0.390 ms. CPU medians improved slightly in this pair of repetitions, but exploratory runs varied more than that difference; a reliable CPU speedup has not been established. Process working set/private commit also did not demonstrate a consistent decrease. The measured allocation reduction is the unused GPU particle buffers described below, not a claim of lower whole-process RAM.

Additional runs of the final implementation:

| Grass patches | Trees | View | Render instances | Selected triangles at final frame | GPU median | Dropped work |
| ---: | ---: | --- | ---: | ---: | ---: | ---: |
| 1,048,576 | 65,536 | Overview | 1,179,649 | 103,911,475 | 6.043 ms | 0 |
| 1,048,576 | 65,536 | Ground | 1,179,649 | 66,527,832 | 6.620 ms | 0 |
| 262,144 | 0 | Ground | 262,145 | 89,517,544 | 5.764 ms | 0 |
| 16,384 | 16,384 | Dense forest, ground | 49,153 | 12,942,710 | 4.158 ms | 0 |
| 16,384 | 1,024 | Four full rotations, 2,400 frames | 18,433 | 55,258,314 | 5.292 ms | 0 |

Both main-view and shadow dropped counters remained zero throughout the measured frames of these final runs. The million-patch overview before early rejection failed with a maximum of 10,179,858 dropped entries in a measured frame; its 2.55 ms result is invalid for a quality-preserving comparison. Denser trees can occlude more geometry, so increasing tree count need not increase selected triangles or GPU time monotonically.

All 1,440 paired frame positions had identical selected-cluster and triangle counts between versions. Isolated raster coverage/depth and sky-cache tests are bit-exact. Full shaded PNGs are not bit-identical: mean absolute RGB differences were 0.115–0.319 on a 0–255 scale, compared with 0.021–0.165 between baseline repetitions. These differences are recorded rather than treated as exact image equivalence. Inspection of the ground views and rotation checkpoints found no large spreading black region. At the four rotation checkpoints, near-black pixels (all channels below 4) occupied 0.15–0.24% of the image. This is a 40-second simulated-time soak, not a claim of indefinite stability or validation of the user's separate street application.

Data: [individual runs](performance-results/vegetation-runs.csv), [aggregate pass timings](performance-results/vegetation-aggregate.csv), [image differences](performance-results/vegetation-image-checks.json), [rotation checkpoints](performance-results/vegetation-orbit-checks.json), and [the failed pre-rejection million-patch run](performance-results/vegetation-overflow-before.csv). The overflow reference uses the raster/sky/particle changes but the original queueing behavior, isolating the capacity issue.

## Changes

### Reject unnecessary hierarchy work before queueing it

The existing traversal checked a child's parent-error bound only after adding the child to the queue. At very high instance counts, many nodes below the required LOD cut could exhaust the 4,194,304-entry queue before being immediately rejected on the next dispatch.

The child now goes through that same existing bound and comparison before claiming a queue entry. It still goes through the normal traversal checks when processed. This uses the bounds already stored by the baker, with the same transforms, pose margin and camera threshold. No new visibility approximation or lower detail setting is introduced.

In the 1,048,576-patch / 65,536-tree overview, the earlier build dropped more than ten million queue entries in a measured frame. The early rejection build completed with zero drops and selected about 104 million triangles. The failed build's shorter frame time is not a valid speed baseline.

A GPU regression checks values below, exactly at and above the threshold, alongside the existing multi-million-entry traversal/dispatch tests. This does not remove every capacity limit: a genuinely wider live frontier or visible list can still overflow and must still be reported.

### Scan only potentially covered pixel centres

The software rasterizer previously scanned from the floor of each triangle's minimum coordinate to the ceiling of its maximum coordinate, inclusive. Tiny grass and leaf triangles consequently visited extra rows and columns, often even when they could not cover a pixel centre.

The new bounds come from the same rounded 8-bit subpixel coordinates used by the edge tests. They include every potentially covered centre. Triangle selection, positions, fill rules, depth interpolation, alpha coverage and LOD remain unchanged. Both opaque and masked software rasterizers, including their depth-only variants, use this shared implementation.

A GPU regression compares the production code with an exhaustive scan of a 16 × 16 viewport for 13,402 triangles: random tiny triangles, quantization ties, exact centres, shared-edge orientations and viewport clipping. All 3,430,912 sample positions must match bit-for-bit in coverage and depth.

### Allocate particle storage when an effect needs it

A forest with no particle effects previously allocated the complete default 262,144-particle pool. The configured capacity and reservation semantics are preserved, while GPU storage is created on the first active `prepare`. Once allocated, storage is retained so stopping and restarting effects does not repeatedly allocate it. Resizing an unused pool remains allocation-free.

This avoids 28,916,848 bytes (27.58 MiB) of simulation buffers at the default capacity: particle records, free/alive lists, lighting, events, emitter records, curves and indirect/uniform data. Pipelines, statistics readback and screen-sized particle targets still exist. Starting the first effect pays the allocation cost at that point.

`Particles::buffer_bytes()` reports these allocated buffers separately from configured capacity. The new lifecycle regression checks idle frames, resizing, cancellation before preparation, a late full-budget burst and restart with retained storage. Existing particle simulation, collision, sorting, lighting and rendering tests remain relevant.

### Reuse an unchanged atmosphere table

The sky-view table is reused when the exact bytes of its shader parameters match the last build. Its only variable inputs are sun direction and viewer radius after the existing height conversion. Camera rotation and movement at the same height do not require rebuilding it. Changing sun or effective height rebuilds immediately, without thresholds or update-rate reduction.

A GPU regression compares cached pixels with a forced rebuild and verifies that changing sun or height changes the output. This saves a small compute dispatch and uniform upload on unchanged frames; it is not the main vegetation speedup.

## Remaining constraints

The renderer stores a 208-byte GPU record per instance and a CPU copy for updates. The million-patch test therefore needs 245,366,992 bytes for the instance GPU buffer alone. A browser/device exposing a smaller single-storage-buffer binding limit cannot run that case unchanged. Splitting static and moving instance data, or paging instance records, is a separate opportunity; it is not implemented here.

The default atmosphere is kept in all runs. The very large `overview` cameras rise into cloud/haze, so their screenshots are not useful for judging close-up leaf detail even though the selected geometry is still rasterized. Ground-level tests exercise visible nearby grass and leaves. No claim is made that every one of the scene's source triangles is simultaneously visible.

The new early LOD rejection avoids proven unnecessary work. Back-face cones for one-sided static surfaces could eliminate other work, but two-sided leaves cannot use that rejection indiscriminately. Off-screen geometry may also remain necessary for shadows, indirect lighting and reflections. No angle-based page blacklist is introduced.

## Reproduction

Build the benchmark at the baseline and at the candidate, copying each executable to a distinct path. The baseline needs the benchmark source and its two dev dependencies from this change; its production renderer must remain unchanged. Do not compile or run another GPU workload during timing.

```sh
cargo build --release -p solstice-render --example vegetation_stress
# Single run: grid side, trees, measured frames, camera, output directory
path/to/vegetation_stress 128 1024 240 ground target/vegetation/run

# Matched runs with reversed ordering on alternate repetitions
python scripts/vegetation_stress.py --baseline path/to/before --candidate path/to/after \
    --output target/vegetation-comparison --repeats 2 --frames 240 --stress

cargo test --release -p solstice-render --lib
cargo test --release -p solstice-render --test gpu -- --test-threads=1
```

The comparison runner preserves failed-run logs and refuses to overwrite an existing run directory. `overview` scales camera height with the forest extent; `ground` and `walk` put the camera among the vegetation. The moving camera includes translation and turning.

For a temporal-history soak, `orbit` turns through 360 degrees every 600 measured frames and writes an additional PNG after each turn. For example, `vegetation_stress 128 1024 2400 orbit target/vegetation/orbit` renders four full rotations with wind, clouds and temporal lighting enabled.

## Validation status

* 289 renderer unit tests passed, with one existing ignored test. The expanded early-LOD traversal regression also passed after that change.
* All 11 shader tests and 13 region tests passed, including validation of rewritten shaders.
* Native workspace/all-target Clippy with warnings denied passed. Minimal, default and editor WASM viewer builds and Clippy passed.
* The final GPU integration sweep and separate startup-overflow checks are running; their completed results will be recorded before the pull request is marked ready.
