# On-demand streaming

This is the design as built. [Results](#results) are at the end.

## What the data said

Measured from the current browser cove (`cove_web.solstice`, file v3, textures at 1024):

| Part | Raw | Compressed |
|---|---|---|
| Cluster records (125,354 at 48 bytes) | 6.0 MB | 3.3 MB |
| Hierarchy nodes (11,562) | 0.6 MB | 0.3 MB |
| Vertices and triangles | 192 MB | about 127 MB (ratio 0.66, sampled pages) |
| Root clusters only (50,309 triangles) | 1.1 MB | about 0.7 MB |
| Textures, all mips | 96 MB | about 92 MB (BC barely compresses) |
| Textures, 128 and smaller | 1.5 MB | 1.4 MB |

- **The whole file** gzips to 209 MB today. (163 MB was the older 512 uncompressed bake.)
- **Sibling sets** (clusters with the same parent group): 8,622, averaging 14.5 clusters and 22 KB, largest 66 KB.
- **The hole-free rule has what it needs.** A scan of the file found 71,223 parent-to-child links, none
  missing and no duplicates, so every coarser cluster matches exactly one sibling set below it.
- **One view needs little.** From the cove's opening camera at 1 px error, 1600x900, with no occlusion
  culling and parent pages included, a simulated cut needs 230 pages, or 29 MiB of a 212 MiB total. A
  cut for every direction at once needs 62 MiB.
- **GitHub Pages serves Range requests.** A `GET` with `Range` on the live site returned
  `206 Partial Content` with a stable `ETag`.

## The hole-free rule

Call a sibling set `S(g)` and the clusters simplified from it `P(g)`. They cover the same surface with the
same locked border, so either may be drawn. The renderer draws a valid cut as long as it swaps whole sets, and
only swaps `P(g)` for `S(g)` once everything `P(g)` depends on has itself been swapped.

Today a cluster is drawn when its parent is too coarse and it is itself fine enough. This changes to:

```
draw C  =  parent_too_coarse(C) and (fine_enough(C) or children_not_installed(C))
```

That requires one invariant, enforced on the CPU: **a page installs only after every page holding the
parents of its clusters, and leaves only after every page that depends on it.** A cluster's parent set is
then always resident when it is, so a parent keeps drawing until its whole child set has arrived. Cracks can't
appear because a set's border is locked and all clusters share a mesh-wide position grid.

The GPU needs two extra bits, both in existing fields, so no struct grows:

- **Leaf node `is_leaf`:** leaf flag, installed flag and page id. A leaf whose set is not installed
  requests its page and stops.
- **Cluster `counts`:** a "children not installed" flag in a spare bit.

## Pages

- **Fixed 128 KiB of vertices and triangles.** The largest sibling set fits with room to spare, and it's
  a sensible size for one HTTP request.
- **A sibling set is never split**, so installing a page is all-or-nothing per set.
- **Each page holds sets from one build round.** A set's parents always come from a later round, so page
  dependencies cannot form a cycle.
- **Sets are ordered spatially within a round**, so one page tends to serve one area.
- **Small meshes share pages.** A page isn't tied to a mesh, since offsets are absolute. All root sets go
  together in pinned pages rather than one 128 KiB slot per mesh, which would waste about 10 MB.
- **Cluster records and hierarchy nodes stay resident**, about 7 MB here. This is much simpler than Nanite's
  scheme of keeping them in pages with fix-ups, because activating a page only rewrites records already in
  place. The cost is 48 bytes per cluster, or roughly 0.7 GB for a billion-triangle scene. This repo's
  scenes are far below that, and moving the records into pages later stays possible.

## GPU page pool and eviction

- **One storage buffer of fixed slots**, sized up front, so the budget cannot be exceeded. It's bound to
  the existing `vertices` and `triangles` bindings, which share it. Each page lays out vertices then triangles.
- **No extra indirection in shaders.** Installing rewrites `vertex_offset` and `triangle_offset` in the
  cluster records, so raster, resolve and voxelize shaders don't change.
- **Atomic with respect to frames.** All writes use `queue.write_buffer` between frames, and WebGPU orders them
  against submissions, so a frame never sees half a page.

Requests are one array of `u32` per page, cleared each frame:

- Traversal does `atomicMax(requests[page], priority)` at each leaf that passes the LOD test, where priority
  is projected error. Occluded leaves write the minimum value, which means "keep, but don't load".
- All views write to it, with shadow and GI views weighted lower.
- It can't overflow and needs no deduplication. It's about 8 KB to read back, using the same non-blocking
  pattern as `CullStats`.

Each frame the CPU, with no GPU code involved so it can be unit tested:

1. Adds the parents of requested pages, and sorts by priority with coarser pages first.
2. Issues up to a set number of fetches.
3. Installs arrived pages whose parents are in, up to about 1 MB per frame to avoid hitches.
4. When out of slots, evicts the least recently used page that has no installed dependants, is not pinned, and
   was not touched in the last 30 frames.
5. When that frees too little, a page left without a slot takes the place of the least wanted page still in,
   while it is wanted twice as much: most wanted first, against least wanted first, and of those the
   finest. So a full pool comes to hold what is wanted most, whatever came first. What a view only fetched
   ahead of need, or kept though hidden, makes way for what is seen, and never outbids anything itself. The
   factor of two keeps two pages wanted about as much from changing places for ever. Views ask by turns,
   the shadow maps and the voxels only when they are redrawn, so a page counts as wanted as much as it was
   asked for at most over the last 120 to 240 frames, and as much as a shadow map or the voxels asked for
   it over their last 3600 frames of redrawing: by its last asking alone it would lose its slot at each
   turn and take another's at the next. A page a shadow map or the voxels asked for also stands until
   views of its kind have been redrawn for 240 frames without asking for it again, however long that
   takes. Counted by the clock it left while its view stood still, the page that took its slot redrew the
   view, and the view asked for it back: thousands of pages went round like that for as long as one
   looked. A kind of view that asks for nothing for 3600 frames counts as gone, and its pages as anyone's.
6. If nothing qualifies, the load waits. Quality drops but nothing breaks, and a counter reports it.

**Shadows and GI.** Both are cached today, so they would keep the root-level shape forever. Page installs
will mark them stale, rate-limited and using the existing cascade turn-taking. GI levels re-voxelize one at a time.

## Textures

WebGPU has no sparse textures, and all layers of an array must share a size and mip count. So one array with
missing top mips is not possible, and creating it costs the full memory anyway. Instead, a material's layer
maps to a tier and slot:

| Tier | Edge | Layers | Lifetime |
|---|---|---|---|
| Base | 128 | All | In the initial download, never evicted |
| Middle | 512 | All | Fetched by demand, never evicted |
| Top | Largest bake size that gives at least 6 slots | As many as the budget allows | Least recently used |

- **Memory is fixed at startup**, as all arrays are allocated up front. At 1024 and 64 MiB that's 1.5 MB +
  24 MB + 8 slots of 4.2 MB. A 2048 bake gets 1024 in the browser on that budget, and full 2048 natively.
- **Promotion** copies lower mips between arrays with `copy_texture_to_texture`, fetches only the missing top
  mips, then rewrites the material. **Demotion** only rewrites the material.
- **Bindings:** resolve samples nine textures rather than three, which is under the limit of 16. Voxelize stays
  on the base tier, since 20 cm voxels gain nothing from more.
- **Demand comes from the cull pass, not resolve.** Resolve is already at the guaranteed limit of 8 storage
  buffers. Each cluster stores a quantised UV density in `counts`, each instance its layer and UV scale, and
  `try_emit` works out the mip wanted. Results go in the request array after the page entries. Occlusion
  is included, with no new binding.
- **GPUs without BC** keep the existing path: unpack on arrival, tiers capped at 512.

## Fetching: HTTP Range, in one file

| | One file with Range | One file per page |
|---|---|---|
| Works on GitHub Pages | Yes, tested | Yes |
| Files to deploy | 1 | About 1,700 pages plus texture mips |
| Merging adjacent pages into one request | Yes | No |
| Atomic update of a scene | Check `ETag` on each response | Files can mix mid-deploy |
| Works with `python -m http.server` | No, it ignores Range | Yes |

**Recommendation: Range.** If a server answers `200` to a Range request, the page downloads the whole file once
and serves ranges from memory, so nothing breaks. `scripts/serve.py` replaces the plain server in
`launch.json`, adding Range support and a `--throttle-mbps` option for measuring.

File v4 (version 5 packs its pages tighter, see [Tighter pages](#tighter-pages-file-version-5)):

```
header    magic, version, sizes and offsets
manifest  deflate: meshes, nodes, clusters, page table, set links, materials, instances,
          lights, viewpoint, root pages, texture mips of 128 and below
pages     each deflated on its own (filtered first, from version 5)
textures  raw, by layer and mip
```

The whole-file `.gz` goes away, as pages and manifest are already compressed. Rust never fetches in the
browser: JS calls `viewer.page_requests()`, fetches, unpacks with `DecompressionStream`, and calls
`viewer.deliver_page(id, bytes)`. Natively, a background thread reads and inflates. Failed fetches retry with
back-off while rendering continues.

## Stages

Each stage kept rendering correct and passed fmt, clippy and tests. Textures came before the
browser in the end, as the page needed them to look right.

0. **Baseline.** Measure download, time to first image and GPU memory on current `main`, with a throttled server.
1. **Format.** Sets, rounds, page assignment and file v4. Loading still installs everything. Rebake; renders
   should match today's.
2. **Residency-aware culling.** Pool, flags and the new draw rule, still fully resident. Tests that random
   valid residency sets, from random viewpoints, are watertight. A GPU test with only roots installed
   and `temporal_accumulation: false` checks for holes.
3. **Streaming, native.** Requests, residency manager, eviction, budget, shadow and GI invalidation. Tests
   for page selection, parents-first install, children-first eviction, pinning, the budget and a randomised
   stress run checking the cut after every step.
4. **Browser.** Range fetching, whole-file fallback, HUD counters, `serve.py`, `build-web.sh`.
5. **Textures.** Tiers, demand and promotion.
6. **Report.** Before and after numbers, README, Status section.

## Results

The browser cove at 1024, on an RTX 5080, in a 1024x768 canvas, with `scripts/serve.py` limiting
all traffic to 50 Mbit/s with 30 ms latency. Whole-viewer memory is `nvidia-smi` total used minus
idle, with both versions measured back to back. Idle drifted by about 70 MiB, so treat those two
as approximate.

| | Before | After |
|---|---|---|
| Download before the first image | 209 MB | **6.0 MB** |
| First image | 36.5 s | **1.4 s** |
| The opening view at full detail | At the first image | About 17 s, 61 MB in all |
| Scene data on the GPU | 280 MiB | **195 MiB, fixed**: 128 geometry, 60 textures, 6 records |
| Whole viewer on the GPU, browser | 855 MiB | **668 MiB** |
| On the server | 209 MB | 226 MB |

Scene data accounts for about 85 MiB of the 187 MiB fall in whole-viewer memory. The rest wasn't
traced. From the same camera, a settled streamed image differs from a whole load by 44 to 48 dB,
no more than two whole loads differ from each other (45 dB), using 396 of 1,640 pages.

The 2048 bake is 515 MB on the server, and starts the same way: 6.0 MB and 1.4 s to the first image.
With the budget cut to 24 MiB of geometry and 16 MiB of textures, flying between three viewpoints
evicted 46 pages with no GPU errors and stayed within the pool. Parts of the view were coarser
and textures softer, as intended.

A server without Range support, such as `python -m http.server`, also works: the page downloads
the file once, in that test 226 MB, and streams from memory.

**Not measured:** flyover frame times, real networks, and GitHub Pages serving a `.solstice` file.
Range was tested on Pages with a `.gz` file only. The on-screen frame rate looked the same as before
in the browser, at 102 to 110 fps, but with no controlled comparison.

## Tighter pages (file version 5)

A pool of fixed slots holds as many clusters as fit in a slot, so this made what a cluster takes
smaller, and the slots fuller. Measured on a castle in its gardens, a scene that is not in this
repository (31.2 million unique triangles in 960 meshes, most of it leaf-by-leaf planting,
541,000 clusters, 4.1 billion triangles placed), with
`cargo run --release -p solstice-asset --example page_stats -- FILE`, which measures any bake.

**What the pages held.** Vertices were 82% of them, at 129 to a cluster of 118 triangles: leaves
and petals share few corners. Of a vertex's 16 bytes two were padding, and its normal had 32 bits.
Of a triangle's 4 bytes one was unused. Slots were 91.6% full, as a page closed at the first
sibling set that would not fit.

**What changed.**

- **A vertex is 12 bytes**, `x y z n u v` at sixteen bits each. The normal is octahedral with a
  signed byte for each coordinate, as Nanite's default is, and the four codes around the exact
  point are tried for the nearest (`gpu::encode_normal`): 0.31 degrees out on average, under one
  at worst. Positions and UVs keep all their bits, so nothing moves and no seam can open.
- **A triangle is 3 bytes**, end to end within its cluster (`gpu::pack_triangle_words`). A mesh
  being built still has a word for each; pages are packed when they are made.
- **Offsets are words.** A cluster's `vertex_offset` counts words of the pool, as its
  `triangle_offset` always did, so a slot need not be a whole number of vertices and a banded
  pool needs no special arithmetic for them. Shaders read both through `common/pool.wgsl`.
- **Skins lie end to end** behind their cluster's vertices, two words each: a skinned vertex is
  20 bytes where it was 24.
- **A page is filtered before it is deflated** (`file::filter_page`): each field of a vertex as
  its difference from the vertex before, one byte of it to a run. The host undoes it on arrival
  (`Renderer::deliver_fetched`). Triangles are left alone, as they packed no better for it.
- **Room left in a page is filled** from the next twelve sets of the same mesh and round, where
  any fit.

| | Version 4 | Version 5 |
|---|---|---|
| Vertex, triangle | 16 and 4 bytes | **12 and 3** |
| Pages, unpacked | 1,294.6 MiB | **970.8 MiB** |
| Pages, as stored | 787.3 MiB | **499.9 MiB** |
| The scene file | 1,062.7 MB | **761.2 MB** |
| Pages | 11,309 | **8,033** |
| Slots full to | 91.6% | **96.7%** |
| Clusters to a slot | 47.9 | **67.4** |
| Pool bytes a cluster | 2,736 | **1,945** |

So a pool of the same size holds 41% more clusters, and a page is 36% less to download for
what it holds.

**What views want**, at 1600x900, after 400 frames with the default 128 MiB pool and 600 with a
pool that holds everything:

| View | Short of room in 128 MiB, v4 | v5 | Installed with room for all, v4 | v5 |
|---|---|---|---|---|
| South gate, up the avenue | 1,153 | **643** | 2,759 (345 MiB) | **2,080 (260 MiB)** |
| From the air | 204 | **0** | 1,329 | **1,057** |
| In the maze | 658 | **272** | 2,368 | **1,788** |
| The great hall | 0 | 0 | 2,401 | **1,874** |
| By the lake | | | 2,643 | **1,982** |
| The rose garden | | | 2,584 | **1,967** |

A scene this dense still wants more than the default pool: at 128 MiB its opening view is
short, and its own page asks for 320 with `?geometry-mb=320`.

**It costs nothing to draw.** The same three views, whole scene loaded, 400 frames: 5.56 ms a
frame before and 5.53 after from the gate (raster 2.03 and 2.00, resolve 0.30 and 0.30), 3.52 and
3.46 in the maze, 2.94 and 2.92 in the hall. A triangle that straddles two words is a second
read, and it does not show.

**The picture changes a little.** A version 5 render differs from the version 4 one by 35 to
37 dB, where two renders of one file differ by 44 to 47 dB. The difference is soft and wide,
bounced light a few levels brighter or darker in places, not edges: normals a fraction of a
degree apart now weld into one vertex, so meshes simplify slightly differently and the voxels
light is traced through differ with them. Side by side they look the same. All of
`cargo test` passes, the GPU tests among them.

**Two things tried and left out**, both for the same reason: what matters is how many pages a
view wants, and that is set by how well a page keeps to one place.

- *Filling pages from the next 256 sets of any mesh* made slots 99.0% full and views want a
  quarter more pages: from the gate, 1,058 short of room where 643 had been.
- *Ordering meshes by where their instances stand*, so that small meshes sharing a page would be
  neighbours, made every view want 3 to 4% more pages. The order meshes are made in already puts
  like beside like.

**A version 4 file does not open**: the reader says so, and the scene must be baked again. The
surfaces in `assets/content` are scene files too, and `scripts/build-web.sh` makes them again
when they are missing.

**Not measured:** a browser on a real network, scenes with skinned meshes or regions (their
tests pass, but no such scene was baked for sizes), and whether deflate at a lower level would
now do as well.

## Known limits

- **Most of the viewer's memory isn't the scene.** The rest is render targets, cull lists and the GI
  volume, which this work doesn't touch.
- **A pool that's too small is coarse somewhere.** When one view wants more than the pool holds, the pages
  wanted least stay out, and pages wanted within twice of each other stay as they came. `geometry-mb`
  on the page, and `--geometry-budget-mb` natively, give the pool more room.
- **Detail pops in with no cross-fade**, softened only by temporal accumulation.
- **A fast flight outruns the network.** The result is coarser geometry until pages arrive, not holes.
- **The live demo stays procedural for now.** CI bakes without scans, so putting the cove on Pages is separate work.
