# Zorah: NVIDIA's large scene

Solstice can bake [Zorah](https://github.com/nvpro-samples/vk_lod_clusters/blob/main/docs/scenes.md),
the glTF export of NVIDIA's RTX Kit sample shown at GDC 2025, which is far larger than anything
else this engine has read: 3.31 billion unique triangles, 18.9 billion with instancing, 2,812
meshes and 1,514 materials with 4,418 textures.

## Credit and licence

The Zorah scene is © 2025-2026 NVIDIA Corporation & Affiliates, released under the MIT licence,
whose text is in the archive's `LICENSE` file. Its environment map, `pizzo_pernice_4k.hdr`, is by
Andreas Mischok, from Poly Haven (CC0). The scene is not stored in this repository, and no bake of
it is published.

## Getting it

`python scripts/fetch-zorah.py DIR` downloads 75 GB and unpacks 83 GB, with the download resuming
if it is interrupted.

## Baking it

```
bake --large-gltf SCENE.gltf [--region X,Y,Z,RADIUS] [--no-textures] [--glow STRENGTH] [--texture-size N] --output FILE
```

`--large-gltf` works from disk, one mesh at a time, for scenes too large to hold in memory
(`crates/solstice-import/src/streamed.rs`). It builds meshes in parallel with a limit on the
triangles in flight, spills each mesh's vertices and triangles to a temporary file, cuts pages from
that, and writes the usual scene file: the format is unchanged, and the viewer reads it with
`--stream`. It stops cleanly if the output's drive falls below 50 GiB free, and removes its
temporary files however it ends.

It reads two extensions that `--gltf` does not need for ordinary files:
`EXT_meshopt_compression`, decoded in `meshopt.rs` to the specification's bitstream, and
`EXT_mesh_gpu_instancing`.

Textures are KTX2 in BC7 and BC5, zstd-compressed per level, which is what the renderer stores, so
`ktx2.rs` copies their blocks. A square level is an exact copy. Other shapes take the nearest
block. Normal maps have their green channel inverted in the blocks.

Emissive maps are kept at 512 texels, in the file's `GLOW` section, and materials that have one
glow by it. A material that blends and has an emissive map is left out of the clusters and written
to the `CARD` section as world-space triangles: in Zorah, the flames on the 30 standing lamps. Both
are optional sections, so the file version is unchanged and older files still load. glTF doesn't
say how bright an emissive factor of 1 is, so `--glow` multiplies them all. **Use `--glow 5` for
Zorah**: bright at night, still visible by day.

`--region` bakes only what lies within a radius of a point, which is how to try the scene quickly.
A radius of 5 m around NVIDIA's opening camera is 88 million unique triangles, baked in about 11
minutes.

## What it does not reproduce yet

- An alpha cut-off other than 0.5 is drawn at 0.5, since changing it means editing BC7 blocks.
  111 materials use 0.333, so foliage is a little thinner than it should be.
- A material with an emissive map gives the scene's GI light as if a tenth of it glowed, as voxels
  don't read the map.
- Zorah's flames are one still picture each. The shader sways and flickers them, which NVIDIA's
  scene doesn't describe.
- Non-square textures, 348 of the 4,418, are resampled by nearest block.

## Indoors

The throne room has no lamps that light it: what is seen there is sunlight that came in through the doors and bounced, about eleven stops below the ground outside. Three things went wrong there that the smaller scenes never showed.

- **It was drawn four stops too dark.** Exposure stopped following the picture seven stops below sunlit ground, the limit that keeps night dark and not grey. Under a sun that is well up it now goes five stops further (`Environment::day`, `DARKEST_BY_DAY` in `post.rs`), fading back to the old limit as the sun nears the horizon, so dusk, night and moonlight are as they were.
- **White patches lay at the foot of each step.** A reflection's ray counts as hitting only what it is closing in on, which its first sample can't tell. One starting at the foot of a riser, among the floor's voxels and below the middle of them, saw itself only leaving them, went through the floor and out under the scene, and brought back the sky. A ray that starts beside a surface other than its own has now hit it (`met_at_once` in `gi/reflections.wgsl`), a ray that leaves the volume pointing well down brings nothing where there is no sea, and one that runs out of steps among surfaces brings the light of the nearest (`miss_radiance`).
- **White dots lay on the columns and doors.** One ray in many finds a window, two thousand times brighter than the room. A reflection sample was allowed several times the light around it, and a pixel seen for the first time anything at all. It is now held to about the light arriving there plus what has built up, new pixels too, and what is truly brighter builds up over a few frames.

## Water

The public export has basins and no water: no mesh, material or node of it describes a water surface. So the viewer is told where the water lies (`ocean/pond.rs`): `--pond x0,z0,x1,z1,level` natively and `?ponds=` on the page, a rectangle and a height each, with `;` between ponds, up to eight. For the courtyard's long pool and the two garden ponds:

```
24.13,-5.51,59.39,5.47,1.9;-1.67,-37.3,3.55,-23.07,1.5;-1.67,23.05,3.55,37.28,1.5
```

A scene can also carry its own ponds, so that no viewer needs telling: a `pond` line in a scene's text file (see `apps/bake/src/scene_file.rs`), or `SceneAsset::still_water`, which says what the water is like too. Ponds given to the viewer take the place of the scene's own.

A pond is a level sheet of still water, clear unless its scene says otherwise, with what it reflects and what is seen through it. Its reflection is traced along several rays a pixel (four at High quality, two at Medium, one at Low), spread by the water's roughness, so wind-ruffled water blurs its reflection smoothly; what lies under the water is the bed straight behind each pixel, dimmed and tinted by the depth of water over it. Nothing is drawn or simulated for it: one pass over the lit scene finds where each pixel's view ray meets a pond. A basin's own walls hide the sheet's edges. A scene with a sea draws the sea and not its ponds, and a scene with neither makes none of this.

## Doors

The export's doors are shut, and nothing in it says they open. `scenes/zorah/` is a game for the scene that opens them (docs/gameplay.md, *A game from a folder*): the throne room's door from the courtyard, its great gate at the far end, the courtyard's gate, and the seven small gates in the garden walls. It is played without a bake, `--game scenes/zorah` natively and `&game=zorah` on the page, where *Play* starts it. Fly up to a doorway and press **F**, or click: its leaves swing open, and shut when used again.

- **Each leaf is an actor at its hinge** (`door.actor`, `door.star`), turning the instances the scene already draws for it, which its line in `scene.game` names. The numbers are those of the bake of `zorah_textured_public.v1`, found with `viewer --instances-near`, and are good only for that bake.
- **Near is measured from the camera,** which scripts read as `viewer()`. Zorah is baked without collision, so there is no floor to walk on and nobody to walk: the camera is flown, and E and Q stay its up and down.
- **A leaf is drawn as things that move are** from the moment the game starts: afresh each frame, with its own shadows, and out of the GI's voxels. The 20 leaves are 378 instances. Their shadows are drawn every frame whether or not a door is moving, which costs about 3 ms a frame natively beside the throne room's doors and half a millisecond in the courtyard and the garden, where the shadows kept from frame to frame cost 0.2.
- **The entrance door's stone panel sinks into the threshold.** It is one mesh across both leaves, two instances by its materials and not by its halves, so it can swing with neither. `door.actor` has a `slide` for such a part: how far it moves aside as the doorway opens. Baking the panel as two halves would let it swing.
- **Not done:** nothing stops the camera at a shut door, doors make no sound, and walking needs the scene's collision, which means baking it again.

## Cards

Cards are drawn after the lit scene and water, and before particles (`src/cards.rs`,
`shaders/cards/draw.wgsl`). Scenes with none pay nothing, and a scene with no emissive maps compiles
the same resolve shader and bindings as before.

- **Sorting:** cards are drawn farthest first against each other. Against what is solid and water,
  each pixel is tested against the scene's depth with the water in it. Particles are drawn over all
  cards, so a particle behind a card is wrongly in front of it.
- **Shadows and GI:** cards don't cast or receive shadows, aren't lit, and give the scene no light.
  In Zorah the lamps' fuel has an emissive map and does give GI light.
- **TAA:** cards don't mark the reactive mask or write motion. The flicker is slow and small enough
  that it survives the filter; a fast-moving camera may trail them slightly.
- **Without BC textures** there are no cards, and materials glow evenly by their factors.

## Viewing it

The whole map runs in the browser, served locally. A browser has 4 GiB of memory, and the scene's manifest unpacks to 5.65 GB, so it needs a second file holding the manifest without its 3.05 GB of cluster records and 2.07 GB of pinned pages, which follow it uncompressed. Make it from a baked scene, which takes about a minute and needs no re-bake:

```
bake --web-split zorah.solstice --output zorah.web
```

Put `zorah.solstice` and `zorah.web` in `dist/assets/`, serve `dist/` with `scripts/serve.py` and open `?scene=zorah&split`. A scene read in parts gets 10,000 MiB for geometry and 1,000 MiB for textures unless `geometry-mb` and `texture-mb` say otherwise: with the page's usual 128 and 64 it holds only the pages it can never drop, and looks low-poly under blurred textures. The file version doesn't change, and the native viewer refuses the `.web` file.

- **Cluster records come and go with their pages.** The `.web` file holds them in the order pages do, so a page's records are one run of it. The GPU has a block of records for each pool slot, 187 for Zorah, the most any of its pages has, and a leaf node is pointed at its set's place there when its page installs. That is 0.7 GB of GPU memory for a pool of 80,000 pages, where the whole table is 3.05 GB, and it leaves room for about 16,000 more pages. Such a scene can't be skinned, as bone lists go by a cluster's number.
- **Up to eight pools.** A pool address keeps its band in its top three bits, above 2^29 words, which is one binding. Three pools are by level of detail and the rest are spare, bound only as far as the budget needs: 10,000 MiB is five. The masked rasterizers then bind 13 storage buffers, of Chrome's 16.
- **Pages are fetched and unpacked in workers,** 64 to a request. A browser opens six connections to a host, so one page a request stops at about 1,000 pages a second; `scripts/serve.py` answers an `X-Solstice-Ranges` header with many parts of a file in one response.

- **Cached views ask for their pages apart.** The sun's shadow cascades and the GI's voxels are kept from frame to frame, and ask for pages only on the frame they are drawn. Requests gather on the GPU until they are read, so that frame isn't missed, and those views set a bit per page beside their requests, the cascades' apart from the voxels'. Such a page is held until views of its kind have been redrawn for four seconds without asking for it again, and only when one arrives is that kind of view drawn again. Before, any page arriving redrew them all, the redraw asked for pages that had been let go since, and their arrival redrew them again, without end.
- **Clusters under a texel across are specks.** The coarsest level of the whole map is 120 million triangles, in clusters with a median radius of 5 cm: the bake stops simplifying a mesh of many small pieces long before it is a few triangles. A far cascade or a coarse level of voxels is therefore a million or two clusters that each cover one texel or voxel, and the rasterizers give each a workgroup of 128 threads. `RendererSettings::speck_texels` (`viewer --specks 1`) sends such clusters to a list of their own, drawn a thread each: voxelizing a level went from 21 ms to 1.6 ms at most, and a cascade from 14 ms to 6 ms, natively. A speck marks every texel its bounds touch, so it never lets through light that its triangles would have stopped.
- **The far cascades lag the sun.** Each cascade keeps the light it was drawn with until the light has moved about a texel's worth for it, so a step of the sun redraws the nearest cascade and, most times, no other. The page still holds the sun still for this scene unless `?day-length=` says otherwise.
- **The streamer keeps time in sixtieths of a second, not frames,** so that a display at 200 Hz doesn't plan, read requests back and let pages go three times as often as one at 60 Hz. Pages install at up to 9,000 a second, shared out over the frames.

Tested by hand on one machine: Chrome 152 (Dawn), RTX 5080, Windows 11, at 1600×900 on a 210 Hz display, at budgets of 10,000 MiB geometry and 1,000 MiB textures. The first frame comes after about 14 seconds, and pages then arrive at about 5,000 a second. From the throne room (`&camera=-30,2.5,0,1.571,0&time=11:00`), which settles on 62,000 pages in about 13 seconds more:

| | frames a second | frames over 20 ms |
|---|---|---|
| Standing still | 210 | none |
| Standing still, the sun moving at a day in 20 minutes | 203 | none in 10 s |
| Walking | 198 | 1 in 8 s |
| Flying at full speed, the pool filling and pages leaving | about 105 | about 8 a second |

No GPU errors. What is left in the last row is the nearest cascades being redrawn as the camera moves (10 to 20 ms each) and the time the page spends installing pages (a few thousand small buffer writes a frame). CI doesn't run this path; GPU tests cover the split path, the specks and the held requests on small scenes natively.

The full scene's cluster table and the page pool it wants are each over wgpu's 2 GiB binding limit, so `viewer --stream --geometry-budget-mb N` splits them between bindings for such a scene (`crates/solstice-render/src/regions.rs`): cluster records in two, and the pool in three by how simplified a page is. Pages are stored coarsest first, so the far band fills with the pages every other page depends on. The whole of Zorah has more of those always-loaded pages (19,537) than one binding holds (16,383), so the rest of the far band's pages go in the middle band. A page goes in its own pool if there's room, and otherwise borrows from whichever has the most it isn't using. The path is for an RTX 5080 on x64, natively or in Chrome. It needs 11 storage buffers per stage, and Chrome 152 reports 16 on that card, with a 2,147,483,644-byte storage binding and a 2 GiB buffer limit. Integrated GPUs are out of scope for this scene. Scenes that fit in single bindings don't use it, and compile the same shaders with the same limits as before.

To run the renderer's GPU tests through this path on small scenes: `SOLSTICE_FORCE_REGIONS=1 cargo test -p solstice-render --test gpu`.
