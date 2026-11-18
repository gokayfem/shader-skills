# Performance and temporal-history audit

Local audit of `765343c`, 13 November 2026. Windows, Rust 1.95.0, release profile,
AMD Ryzen 7 9800X3D, NVIDIA GeForce RTX 5080, Vulkan, driver 616.64. Measurements below compare the
original sources against the changes in this checkout. They are component
benchmarks, not claims about a game's overall FPS.

## Spreading black surfaces

The production diffuse-GI temporal shader reproduces the reported spreading
behaviour without a large scene. Injecting one NaN, positive infinity, or negative
infinity into its history, then supplying healthy inputs for twelve frames,
produces these counts of invalid pixels:

| Frame | 1 | 2 | 3 | 4 | 5 | 6 | 7 | 8 | 9 | 10 | 11 | 12 |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| Original | 4 | 9 | 16 | 25 | 36 | 49 | 64 | 81 | 100 | 121 | 144 | 169 |
| Fixed | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 |

Each frame's four-tap history gather was blending invalid values into neighbouring
pixels, then storing them for the next frame. Turning the camera away rejects
history, consistent with the user's observation that rotating clears the black
area. This proves a propagation mechanism in the library; it does not prove which
input first became invalid in the user's running street scene.

A second production-shader test identifies a concrete source of invalid values:
the visibility resolve divided each triangle vertex by its clip-space `w`. A
visible triangle can cross the eye plane with one vertex at `w = 0`. The old
function returned three NaNs for a case whose correct interpolation weights are
`(0.2, 0.3, 0.5)`. The replacement solves the same perspective interpolation in
homogeneous coordinates, avoiding those per-vertex divisions. Degenerate
projections have a finite fallback. Tests cover positive, zero, negative, and very
small vertex `w`, extrapolated samples used for derivatives, and zero-area input.

The fix also rejects invalid samples before the diffuse and reflection history
filters, probe projection, and TAA. Invalid screen-space lighting falls back to
volume tracing. A standalone TAA test showed one bad current pixel making a 3x3
black patch in the original shader. The fixed shader excludes that sample and
normalizes the remaining valid weights. It can also recover from damaged history.
Finite-value checks inspect exponent bits, rather than relying on `x == x` under
shader floating-point optimizations. TAA output is bounded to the representable
half-float maximum to avoid creating infinity at its storage boundary.

No GI, reflection, anti-aliasing, resolution, ray count, or LOD quality setting is
reduced. The running application must be rebuilt with these shaders; existing
scene bakes do not need changing for this fix.

## Avoidable per-frame CPU work

The movement table searched pending moves backwards every time a caller read an
instance transform. Reading and then moving every object therefore became
quadratic. Repeated moves also accumulated duplicate records that were sorted at
the end of the frame. A per-instance pending index now provides direct lookup and
replaces a previous update in place. Frame upload ranges still merge nearby
changes, with last-update-wins and previous-frame transforms preserved.

Streaming sorted all requested pages even when a frame would fetch only a small
number. A linear-time heap build and bounded extraction preserve the exact
priority and page-ID tie ordering. Immutable dependency and dependant lists use
contiguous offsets and entries, rather than a `Vec` header per page per direction.

`cpu_stress` performs one warm-up and reports the median of six measured rounds:

| Workload | Original median, ms | Fixed median, ms |
| --- | ---: | ---: |
| Read/move 1,024 objects | 0.2166 | 0.0048 |
| Read/move 8,192 objects | 17.1014 | 0.0394 |
| Read/move 32,768 objects | 308.5094 | 0.1608 |
| Finish frame, 32,768 moves | 1.5900 | 0.9312 |
| Eight updates/object, 32,768 objects | 6.1798 | 1.1508 |
| Plan requests, 4,096 pages | 0.0165 | 0.0063 |
| Plan requests, 65,536 pages | 0.3733 | 0.1177 |
| Plan requests, 1,048,576 pages | 8.9409 | 2.8114 |

All movement/upload checksums and fetch-order checksums match the original.
The compressed graph removes about 32 bytes of list headers per page on a 64-bit
host, or 16 bytes on wasm32, before allocator overhead. Those are layout savings,
not measured reductions in total application memory; the priority heap uses a
larger entry than the former page-ID queue. The graph is prepared once at scene
load, not rebuilt each frame.

A deterministic streaming soak runs 1,500 frames for each of a single pool and
three LOD pools, with 16,384 pages, dependencies, delayed arrivals, failed fetches,
changing views, protected pages, and eviction. The original and changed code
produce identical trace hashes:

| Pool arrangement | Fetches | Installs | Evictions | Trace hash |
| --- | ---: | ---: | ---: | ---: |
| Single pool | 955 | 866 | 618 | 14859531676831798266 |
| Three pools | 1639 | 1497 | 1249 | 12381328480948452750 |

## More objects and deep traversal

Traversal queues allow 4,194,304 entries, but the old 32-thread indirect dispatch
exceeded the guaranteed 65,535-workgroup X limit at 2,097,121 entries. The stress
test reproduced the invalid `[65536, 1, 1]` dispatch. Work now extends into Y and
the shader flattens both dimensions. Instance seeding follows the same rule.

The GPU regression exercises the actual control and traversal shaders at 0, 1,
31, 32, 33, 2,097,120, 2,097,121, and 4,194,304 entries, including a nonzero queue
start. It checks dispatch legality and complete processing. This removes a limit
below the allocated capacity; it does not make queues or visible-cluster buffers
unbounded. `FrameStats` already reports dropped entries for the main view and
shadows; a considered fallback is still needed before claiming arbitrary scene
sizes are supported.

## Smaller geometry pages without requantization

Page packing now shares byte-identical cluster vertex payloads and triangle runs
within the same page. It includes all vertex attributes and skinning records in
the comparison. Different normals or UVs cannot accidentally alias. Each incoming
sibling set remains intact, and page order remains spatial and coarse-to-fine.
A metadata-count cap prevents extremely repetitive payloads concentrating an
unbounded cluster table into one page.

The page capacity remains 128 KiB. This reduces payload and sometimes page count;
it does not reduce each GPU pool slot's fixed capacity. The binary schema and GPU
decode stay compatible. Existing baked files still load; re-bake assets to gain
the packing savings.

| Fixture | Metric | Original | Fixed |
| --- | --- | ---: | ---: |
| Connected terrain, 73,728 source triangles | Pages | 23 | 23 |
| | Raw geometry bytes | 2,191,488 | 2,190,976 |
| | Deflated geometry bytes | 1,015,543 | 1,015,535 |
| Disconnected foliage, 32,768 source triangles | Pages | 35 | 33 |
| | Raw geometry bytes | 3,402,100 | 3,159,988 |
| | Deflated geometry bytes | 761,959 | 759,240 |
| | All-pages geometry slot capacity, bytes | 4,587,520 | 4,325,376 |

For this foliage fixture the raw payload is 7.12% smaller and the geometry slot
capacity is 5.71% smaller. Deflated bytes improve only 0.36%, because compression
already handles repetition well. These totals exclude textures, cluster metadata,
and other GPU buffers. Savings depend strongly on the asset. One-shot packing
measurements increased from about 1.01 to 1.21 ms for terrain and 1.03 to 1.14 ms
for foliage; deduplication adds some bake work. The harness compares every packed
vertex and triangle against the original cluster data, byte for byte.

## What can move into the bake

The library already bakes cluster bounding spheres, parent LOD errors/bounds,
spatial hierarchies, UV density, and page dependencies. Runtime culling uses those
summaries; it does not rediscover them by scanning all triangles every frame.
It also uses depth-pyramid occlusion and retains occluded pages without demanding
that they be fetched just for that occluded view.

There are further options, but they require their own quality and performance
validation:

* Conservative normal cones can reject an entire single-sided static cluster when
  all its triangles face away. They need an extent-aware perspective test, and
  must be disabled or conservatively expanded for two-sided materials, animation,
  wind deformation, and unsuitable transforms. The current bake has no such cone.
* A static scene can bake potentially-visible sets for spatial cells. They must be
  conservative and account for movable occluders; their storage and bake cost can
  exceed their benefit in open scenes.
* Page-local vertex remapping and a compact three-byte triangle stream may reduce
  data further, but they change decode/upload work and need a format/version plan.
  Current vertices already occupy 16 bytes, triangle indices fit in three bytes
  of a four-byte word, and clusters allow 128 triangles and 255 vertices.
  (Since done in part, as file version 5: 12-byte vertices and the three-byte
  triangle stream. See "Tighter pages" in `docs/streaming.md`.)
* The planner still scans page-sized arrays each plan. A sparse changed-request
  set could reduce that cost further, provided retention, cached-view requests,
  and deterministic priority semantics are preserved.

The current frustum depends on the live camera and cannot simply be baked away.
A page outside that frustum may still cast a visible shadow or supply GI or a
reflection, and a nearby page may be deliberately prefetched before a turn.
An angle-only page blacklist would therefore discard valid contributions. The
existing per-view request logic must remain part of any baked visibility scheme.

There is also a separate correctness concern in `vg/vis_write.wgsl`: winning depth
and triangle ID are written with separate atomics. A farther writer can win depth,
pause, and store its ID after a nearer writer has won and stored its own ID. The
source already acknowledges this race, but its claim that TAA always hides it is
not a correctness guarantee. This audit did not reproduce or redesign that path.
An exact solution needs ordered phases or a suitable combined atomic scheme; a
per-pixel spinlock is not an appropriate shortcut. The history checks contain
invalid lighting, but do not fix mismatched finite depth/ID values.

## Additional reproduced failure

The original Windows disk-space guard failed its existing low-space bake test in
this environment because its PowerShell subprocess did not return a usable free
space value. The Windows path now uses `GetDiskFreeSpaceExW` on the nearest existing
output directory, avoiding subprocess startup and handling mount points and UNC
paths through the native API. Its existing regression passes.

## Validation completed

The local native suites passed 1,009 tests: 630 outside the renderer, 286 renderer
unit tests, and 93 full GPU integration tests. Three existing tests were ignored.
The final integration run used the completed fixes on the RTX 5080 and finished
in 354.39 seconds. The focused fault-injection and full-capacity traversal tests
also passed after the last shader changes.

Workspace/all-target native Clippy with warnings denied, formatting, and diff
whitespace checks passed. The WASM viewer built and passed Clippy for minimal,
default, and editor configurations; the browser baker also built and passed
Clippy. The JavaScript page tests and Python crate-selection tests passed.

## Reproduction

Run from the repository root with Rust 1.95.0. GPU checks require a supported
adapter and print a skip message if none is available.

```text
cargo run --release -p solstice-render --example cpu_stress
cargo run --release -p solstice-render --example page_stress
cargo run --release -p solstice-render --example streaming_stress
cargo test --release -p solstice-render --lib stress -- --nocapture
cargo test --workspace --release
cargo clippy --workspace --all-targets --release -- -D warnings
cargo fmt --all -- --check
```

The benchmark examples can also be compiled against the original library sources
to reproduce the baseline. Raw component measurements are in
[`performance-results`](performance-results/). Full local run logs, including the
intentional failing baseline regressions, are retained under `target/performance`.
The user's running street application has not been rebuilt or visually verified
with this checkout; that remains separate from the reproduced shader defects.
