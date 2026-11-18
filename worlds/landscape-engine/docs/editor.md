# Editor: design

A design for Solstice's editor: a page on the site, beside the viewer, where people build their
own scenes and games in the browser, and which AI agents can drive as well as people can. This
document is the plan, written before any of it is built. It will be kept up to date as each
phase lands. Choices still open are marked **Decision** and listed together at the end.

## Goals

- **An editor people already know how to use.** Anyone who has used Unreal's level editor or
  Unity's scene editor should find the same layout, the same shortcuts and the same habits
  (select, move, frame, duplicate, undo, play) where they expect them. "Unreal/Unity quality"
  is made concrete below, as a checklist each phase is held to.
- **Edits show at once.** Moving, hiding, duplicating, lighting and weather never wait for a
  bake. Work that does need one (a new model, a new terrain) bakes in the browser, in the
  background, on the parts that changed.
- **Agents are first-class users.** Every action in the editor is a command in one registry.
  The panels call it, the keyboard calls it, and the page exposes it to agents (Claude in
  Chrome, a browser MCP, or WebMCP where the browser has it) with the same names, the same
  arguments and the same undo.
- **The viewer pays nothing.** The editor is its own page and its own wasm build. The viewer's
  page, build and scenes are unchanged by it (the gating rule).
- **The browser is the platform.** Everything here runs in Chrome on the RTX 5080 and degrades
  sensibly on phones. Nothing in the first phases needs the native bake.

## What there is to build on

A survey of the engine as it stands (13 November 2026, `main` at `9d57b5b`).

**The web viewer.** `apps/viewer/src/web.rs` is a `WebViewer` bound to one canvas, driven by
the page: `frame()`, `set_camera()`, `set_time()`, `set_weather()`, `set_ponds()`,
`spawn_effect()`, stats (`triangles()`, `clusters()`, `instances()`, `gpu_error_count()`,
`fallbacks()`), streaming (`stream_requests()`, `deliver()`), and, in the `gameplay` build,
play, input and script messages. `apps/viewer/web/index.html` is one page with one inline
script: scene choice by `?scene=`, the streaming loader (range requests, workers, the external
tables of large scenes), the HUD and the settings panel.

**Live changes the renderer already supports.** `Renderer` names instances by their index in
the scene file and can, from the next frame, `move_instance()` (as motion, which TAA and motion
blur follow), `place_instance()` (as a jump), `set_instance_hidden()`,
`set_instance_movable()` (a movable instance is drawn afresh each frame; a static one lives in
the cached shadow and GI views, which redraw once when it changes), `set_instance_stays_in_gi()`
and `set_instance_glow()`, and reports `instance_transform()` and `instance_bounds()`. Weather,
time of day, the sea and ponds are already live. This is most of what a level editor's viewport
needs, and it is why moving things needs no bake.

What it can't do yet: add instances after load (the instance table is sized at load), add
meshes, materials or textures after load, or change lights after load.

**The scene text format** (`apps/bake/src/scene_file.rs`, `scenes/*.txt`). One command per
line: `model`, `part`, `actor`, `blocker`, `light`, `camera`, `terrain`, `land`, `path`,
`yard`, `clear`, `meadow`, `wood`, `ocean`, `water`, `input`, `track`, `circuit`, and from PR
#75 ponds. It is already an editor's document in all but name: a list of placed things and
generators, with comments a person wrote. `append_model()` returns the instances each placement
became, so the bake knows which line made which instance; it just doesn't write that down. The
format served hand-written scenes well, but it has no ids, no hierarchy and no room for
component data, so the editor gets a project format of its own (below) and this one is
converted.

**The scene file** (`crates/solstice-asset/src/file.rs`). Version 4: a header, a deflated
manifest with optional tagged sections (`SEA `, `GAME`, `COLL`, `SKIN`, `CLIP`, `GLOW`, `CARD`,
`EXTP`) that older readers skip, then pages and texture levels, each fetchable alone. A new
section can be added without a version change.

**Procedural generation** (`crates/solstice-procgen`): terrain noise, surfaces, cracks, and
foliage species (trees, grass, weeds, bark, leaves) read from text (`foliage/text.rs`).
**Game layer** (`crates/solstice-game`, `solstice-physics`, `solstice-vehicle`,
`solstice-script`): actors with Starlark scripts, input maps, races and tracks, rapier physics,
all already running in the browser in the `gameplay` build. **Streamed virtual geometry**
(`docs/streaming.md`): pages fetched on demand by HTTP range, which is what lets the site's
scenes open in seconds.

### Can the bake run in the browser?

Yes, for the parts an editor needs. Measured today:

- `solstice-builder`, `solstice-import` and `solstice-procgen` **compile for
  `wasm32-unknown-unknown` unchanged** (`cargo check`, 13 s). rayon falls back to the calling
  thread where there are no threads (`rayon-core` 1.13, `registry.rs:213`), and
  `block_compression` is built CPU-only here. The gltf crate is pure Rust. Only the streamed
  large-scene importer calls `Instant::now()`, which panics on the web; the editor won't use it.
- **Time, one model, one thread, native** (`bake --gltf`, 1024² textures, RTX 5080 PC):

  | Model | Triangles | Whole bake | Without texture compression | 16 threads |
  |---|---|---|---|---|
  | `boulder_01` | 66 k | 10.9 s | 2.9 s | 1.5 s |
  | `gothic_statue` | 28 k | 9.5 s | — | 1.4 s |

  Most of the time is BC7 compression. Wasm runs such code at roughly 1.5–2× native, so an
  imported scan takes about 5 s to build its clusters in one Web Worker, and its textures can go
  to the GPU uncompressed at once (the renderer already has that path, `unpack_textures`), then
  be compressed by a pool of workers, one texture each, and swapped in.

- **What stays native**: baking a whole published scene from scratch (minutes, hundreds of
  megabytes), Zorah-sized imports, and publishing. The editor never rebakes a whole scene; it
  bakes the piece that changed.

So the rule is: **placing and tuning is live; new content bakes piece by piece, in the browser
or on the PC, with the same result; publishing stays on the PC.** See "One bake" below.

## What "Unreal/Unity quality" means

Measured against the editors themselves, not against other web editors. References:

- Unreal Engine 5 level editor: [Level Editor](https://dev.epicgames.com/documentation/en-us/unreal-engine/level-editor-in-unreal-engine),
  [Viewport controls](https://dev.epicgames.com/documentation/en-us/unreal-engine/viewport-controls-in-unreal-engine),
  [Transforming actors](https://dev.epicgames.com/documentation/en-us/unreal-engine/transforming-actors-in-unreal-engine),
  [Landscape](https://dev.epicgames.com/documentation/en-us/unreal-engine/landscape-outdoor-terrain-in-unreal-engine),
  [Foliage mode](https://dev.epicgames.com/documentation/en-us/unreal-engine/foliage-mode-in-unreal-engine),
  [Play in Editor](https://dev.epicgames.com/documentation/en-us/unreal-engine/play-in-editor-in-unreal-engine).
- Unity 6 editor: [Scene view navigation](https://docs.unity3d.com/Manual/SceneViewNavigation.html),
  [Positioning GameObjects](https://docs.unity3d.com/Manual/PositioningGameObjects.html),
  [Hierarchy](https://docs.unity3d.com/Manual/Hierarchy.html),
  [Inspector](https://docs.unity3d.com/Manual/UsingTheInspector.html),
  [Undo](https://docs.unity3d.com/ScriptReference/Undo.html).
- The best editors already in a browser, for what the web makes possible:
  [PlayCanvas Editor](https://developer.playcanvas.com/user-manual/editor/),
  [three.js editor](https://threejs.org/editor/), and
  [Babylon.js Inspector](https://doc.babylonjs.com/toolsAndResources/inspector).

Concretely, the editor reaches that bar when each of these holds:

| Area | Bar |
|---|---|
| Viewport navigation | Right-drag to look with WASD/QE to fly, wheel to change fly speed, Alt+left orbit, Alt+right/wheel dolly, middle-drag pan; **F** frames the selection; an orientation cube that snaps to the six views; perspective and top/front/side orthographic views. |
| Gizmos | Move, rotate, scale on **W/E/R** (Unreal's keys; Unity's Q/W/E/R as an option); planar handles; world/local toggle; snapping to grid, angle and scale steps, toggled and sized from the toolbar; Ctrl to snap on the fly; **End** drops the selection onto the ground; Alt-drag duplicates. Handles keep a constant size on screen and stay readable over any scene. |
| Selection | Click picks the exact object under the cursor (from the visibility buffer, not bounding boxes); Ctrl/Shift add and remove; box select; selection outlined in the viewport and mirrored in the outliner; Escape clears. |
| Outliner | Every object, in the scene's own order and groups, searchable, with eye (hide) and lock toggles, rename in place, drag to reorder and group, and a virtualised list that stays smooth with 100 000 rows (Zorah). |
| Details | The selection's properties, grouped and collapsible: transform (with reset arrows), mesh, material, collision, actor properties; numbers drag-scrub as in both editors; multi-selection edits shared fields and shows mixed values as "—". |
| Content browser | Thumbnails of every mesh, material and species in the scene and in imports; drag into the viewport to place under the cursor, on the surface; search and filter by kind. |
| Undo | Every change, including each gizmo drag as one step, a whole drag of a slider as one step, and agents' changes; Ctrl+Z / Ctrl+Y; a history panel to jump back several steps. |
| Play | Play in the viewport from the camera or the player start; the editor's state is kept and restored when play stops; the viewport tints while playing (as Unity does) so you know edits there are temporary; Simulate (physics without the player) as in Unreal. |
| Output | A console with the editor's, scripts' and the GPU's messages, filterable, clickable to the object or script line they name; frame stats on demand (Unreal's `stat unit`): frame, GPU time per pass, triangles, streaming. |
| Save | Autosave to the browser; open and save projects, including a folder on disk; bake in the browser or on the PC to the same file, so the PC can publish what the browser made. |
| Feel | 60 fps or better in the editor on the 5080 for the site's scenes; no hitch when selecting, dragging or opening panels; shortcuts listed in a cheat sheet; layout remembered. |

## Layout

Unreal's default level editor layout, which Unity's matches closely:

```text
┌──────────────────────────────────────────────────────────────────────────────┐
│ Menu · Save · Undo/Redo · Move/Rotate/Scale · World/Local · Snap ▾ · ▶ Play │  toolbar
├───────────┬────────────────────────────────────────────────────┬─────────────┤
│ Modes     │                                                    │ Outliner    │
│ Select    │                                                    │ ▸ Fort      │
│ Landscape │                    Viewport                        │   tower ×4  │
│ Foliage   │        (gizmo, selection outline, grid,            │ ▸ Courtyard │
│ Water     │         orientation cube, stats overlay)           │ ▸ Cliffs    │
│ Weather   │                                                    ├─────────────┤
│           │                                                    │ Details     │
│           │                                                    │ Transform   │
├───────────┴──────────────────────┬─────────────────────────────┤ Mesh        │
│ Content browser                  │ Output log · History        │ Collision   │
└──────────────────────────────────┴─────────────────────────────┴─────────────┘
```

Panels dock, resize and tab together; the layout is remembered per browser. The **mode**
picked on the left (select, landscape, foliage, water, weather) changes what the viewport's
mouse does and what the details panel shows, as Unreal's editor modes do.

On a phone (Galaxy S26 class) the same editor is laid out for touch: the viewport fills the
screen, the outliner, details and content browser are sheets that slide up, the gizmo's handles
are larger, and two-finger gestures orbit and pan. Phones get the whole command set; the
layout, not the feature list, is what changes.

## Features, by tool

- **Viewport.** The renderer as the viewer has it, with an editor overlay drawn on top:
  gizmos, selection outlines, the grid, light and actor icons, bounds and collision when asked.
  Debug views (clusters, GI, overdraw) from the viewer's menu.
- **Outliner.** The project's entity tree; a foliage entity is one row whose many instances
  are shown only on demand.
- **Details.** The selected entity's components, each drawn from the same schema the bake
  reads (position, rotation, scale, `collide`, ground seating, actor properties), so the panel
  and the bake can never disagree about what a field means.
- **Content browser.** The scene's meshes (from its manifest), the scans the scenes use, the
  built-in foliage species and grasses, and anything imported. Thumbnails render off screen in
  the same renderer, cached in the browser.
- **Asset import (glTF).** Drop a `.glb` or `.gltf` (with its files) on the page. A Web Worker
  runs `solstice-import` and `solstice-builder` built for wasm, and hands back a small scene
  piece (meshes, materials, textures) that the renderer appends. Textures show uncompressed at
  once and are compressed in the background. The source file goes into the project's
  `assets/` with a `.meta`, and the baked piece into its `.cache/`.
- **Terrain.** Sculpt (raise, lower, smooth, flatten, noise) and paint the land's bands (sand,
  dirt, grass), paths and yards. While a brush is down the touched tiles draw from a
  plain heightfield mesh; when it lifts, the worker rebuilds only those tiles' clusters and
  swaps them in. Sculpted height is the project's `terrain/*.height`, painted layers
  `terrain/*.paint`.
- **Foliage.** Paint and erase with a species and a density, as Unreal's foliage mode does,
  plus foliage areas as editable shapes in the viewport (today's `wood` circles, `meadow`
  radius, `clear` circles). Changing a shape regrows only the trees inside it.
- **Water.** The sea's level and kind (`ocean`), and ponds (PR #75's format) as rectangles
  with handles, all already live in the renderer.
- **Weather and time.** The viewer's controls, as a panel, plus saving a scene's starting
  weather and time.
- **Lights.** Point lights with handles for range and colour (needs the renderer to accept
  new lights after load).
- **Actors and scripts.** Place actors from `.actor` files, edit their properties in the
  details panel, edit their Starlark in a code panel with errors marked by line; scripts
  already hot reload.
- **Play-in-editor.** Starts the `gameplay` build's play mode on the edited scene: physics
  colliders are placed where the editor has moved things, not where the bake left them. Stop
  restores every transform. Play loads the gameplay wasm only when first pressed, as the viewer
  does today.
- **Undo and redo.** A command stack (below).
- **Save and load.** Projects autosave to OPFS, or straight to a folder on disk opened in
  Chrome. Download and upload a project as a zip. **Bake** writes `build/*.solstice`, the
  same file `bake --project` writes on the PC.

## The project

The editor's document is a **project**: a folder of source files that the editor and the
native tools both read and write, and from which either the browser or the PC bakes the scene
file the viewer plays. The old scene text (`scenes/*.txt`) is not the editor's format; it is
converted once, and the bake keeps reading it until every scene has moved.

### Layout on disk

```text
scenes/                            a project is a folder
  solstice.project                 its name, its format and its scenes
  cove.scene                       one scene: its settings and its entities (below)
  harbour.scene                    a project can hold many scenes, sharing what they use
  cove/gate.actor, gate.star       actors and scripts, as today
  ../target/scan/…/*.gltf          models, by any path relative to the folder
  terrain/cove.height              sculpted height, 16-bit (phase 4)
  terrain/cove.paint               painted land layers (phase 4)
  .cache/                          baked pieces, by content hash; safe to delete (phase 2)
  build/cove.solstice              the baked scene, what the viewer loads
```

Every path in a project is relative to its folder, the one holding `solstice.project`, and a
scene file can sit anywhere inside it. The repository's own `scenes/` folder becomes one project
holding all the site's scenes, which share their scans and actors, as an Unreal or Unity project
shares its content between levels. The `.meta` files beside imported assets (their ids and
import settings, as Unity's are) come with import, in phase 3; until then a mesh names its file
by path, and `skinned` sits on the mesh.

The same folder lives in three places, all the same bytes:

- **on disk**, for the native bake, git and a text editor; Chrome's editor can open such a folder
  directly ([File System Access](https://developer.chrome.com/docs/capabilities/web-apis/file-system-access)),
  so the browser and the PC work on one copy;
- **in the browser's private file system** (OPFS), where the editor keeps projects that live
  only in the browser, and where phones keep theirs;
- **as a zip**, to download, upload, or attach to a scene's repository.

### The files

Every project file is JSON, written with fields in a fixed order, one to a line, and short lists
of numbers on one line, so it reads and diffs well. JSON because the editor, agents and every
tool read it with no parser of their own; in Rust it is `serde_json`, which the workspace
already built (through Starlark), so it added no new code to any build. The types, reading and
writing are the `solstice-project` crate, which has no file system or GPU in it, so the editor's
wasm and the native bake share it.

`solstice.project`:

```json
{
  "format": 1,
  "name": "scenes",
  "scenes": [
    "cove.scene"
  ]
}
```

A `.scene` holds the settings a scene has one of, then a tree of **entities**. Each entity has
an id that never changes once made (eight letters and digits), a name for people, an optional
note, and **components**. Part of Cove in this format (its ids here are made up):

```json
{
  "format": 1,
  "note": "A fort guarding a sheltered cove, with a ship at anchor, built from Poly Haven scans (CC0).",
  "texture_size": 2048,
  "camera": {
    "position": [66.0, 1.5, -41.0],
    "yaw_radians": 2.31,
    "pitch_radians": 0.03
  },
  "terrain": {
    "size": 900.0,
    "cells": 1024,
    "height": 0.3,
    "shore_x": 52.0
  },
  "land": {
    "sand": 10.0,
    "dirt": 5.0
  },
  "meadow": 75.0,
  "sea": {
    "level": -0.6,
    "ocean": "coastal"
  },
  "entities": [
    {
      "id": "Xq81bT0c",
      "name": "Towers on the four corners",
      "note": "Towers on the four corners. Each is wider than the walls, which hides the joints.",
      "children": [
        {
          "id": "a91fXq2b",
          "name": "tower_round",
          "transform": {
            "position": [22.0, 0.0, -22.0],
            "rotation": [0.0, 90.0, 0.0]
          },
          "mesh": {
            "asset": "../target/scan/modular_fort_01/modular_fort_01.gltf",
            "part": "tower_round"
          }
        }
      ]
    },
    {
      "id": "pQ3m0Lw7",
      "name": "gate",
      "note": "The iron gate, filling the 3.4 m opening of the arch.",
      "transform": {
        "position": [20.7, 0.0, 0.0],
        "rotation": [0.0, 90.0, 0.0]
      },
      "actor": {
        "asset": "cove/gate.actor",
        "properties": {
          "locked": "true"
        }
      }
    }
  ]
}
```

The scene's settings are `texture_size`, `camera`, `terrain`, `land`, `meadow`, `water`, `sea`
and `game` (`input`, `track`, `circuit`). The components that say what an entity is, at most
one each, are `mesh` (a whole model or one `part`, with `collide` and `skinned`), `actor`,
`light`, `blocker`, `wood`, `clear`, `path`, `yard` and `pond`; an entity with none is a folder.
A child's transform is in the world, not relative to its parent: folders only group, as
Unreal's do. `transform` holds `position`, `rotation` (degrees about X, Y and Z, as yaw about Y,
then pitch about the turned X, then roll about the turned Z),
a uniform `scale`, and `ground`: the share of its height sunk into the terrain, for a model
that stands on it. Anything left at its default is left out of the file. Unknown fields, a
later format, a repeated id, or an entity that is two kinds of thing are refused with a reason.

### Baking a project, and converting the old scenes

`bake --project scenes/cove.scene` bakes a scene to the project's `build/cove.solstice`. It
lowers each setting and entity to the line of scene text that says the same thing and reads it
with the text's own parser, so a project and a text can never mean different things and every
check the text has, the project has. (Only a model turned about X or Z says more than a line
can; that is set on the parsed line.)

`bake --convert scenes/cove.txt` writes `scenes/cove.scene` and lists it in
`scenes/solstice.project`. Each line that places something becomes an entity named from what
it places; the comments above it become its note; a run of banner comments
(`# ---- Towers on the four corners. …`) becomes a folder named by its first sentence. Ids come
from where each line was, so converting twice gives the same ids. Two tests hold the converter
to the text: every scene in `scenes/` converts, is written and read back, and lowers to exactly
the commands its text has; and, given the scans, baking the converted project gives a
byte-identical scene file to baking the text, apart from the new map of entities (shown for
Cove; Harbour's and the walker's files weren't on the PC it was run on). A project lowers its
settings before its entities, so a text that names its `input` or `track` after an actor would
list its gameplay files in another order; none of the site's scenes does. The scenes
move over one at a time, each with your go-ahead, and `publish-scenes.py` then publishes from
the project.

## One bake, in the browser or on the PC

The bake becomes a library, `solstice-bake`, taken out of `apps/bake`, that turns a project into
a scene file. The native `bake` command and the editor's Web Worker both call it; neither has a
bake of its own.

**It bakes in pieces, by content.** Each source asset, each foliage area and each terrain tile
is baked on its own into a **piece** (its meshes, materials, textures and collision), stored in
`.cache/` under a hash of everything it depends on: the source bytes, its import settings, and
the bake's own version. Placing entities costs no baking at all: the final step only gathers
the pieces the scene uses and writes their instances, which takes well under a second. So:

- moving, adding or deleting a placed model **rebakes nothing**;
- importing one model bakes **that model**, a few seconds in the browser (measured above);
- sculpting a corner of the terrain rebakes **those tiles**; regrowing one grove, **that grove**.

**The browser and the PC give the same bytes.** The bake is already deterministic: the same
model baked on one thread and on sixteen gives byte-identical files (checked, 13 November). The
library keeps that a rule, tested in CI, so a piece baked in the browser and one baked on the PC
are interchangeable. A project opened from disk shares one `.cache/` between them: bake the big
scans natively once, and the editor never bakes them again.

**Where each bake runs best:**

| Work | Browser (editor) | PC (`bake --project`) |
|---|---|---|
| Placing, moving, environment | live, no bake | — |
| One imported model, a few foliage areas, terrain tiles | seconds, in a worker | seconds |
| A whole small scene from an empty cache | a minute or two, workers | under a minute, all cores |
| Photoscanned fort scenes, Zorah | possible, slow; better natively | minutes |
| Publishing to a scene's site | — | `publish-scenes.py` |

Texture compression is most of a bake's time. The browser shows a new texture uncompressed at
once and compresses it in the background on a pool of workers, one texture each.

### The editor and baked scenes

The editor opens a **project**, and shows it from its baked scene: pieces already in `.cache/`
stream as the viewer streams, and anything missing is baked and appended as it is ready. To
open one of the site's scenes, the editor fetches its project (published beside its baked file)
and its baked file; the baked file's pieces fill the cache, so nothing is baked to start.

**The baked scene records which entity made which instances.** A new optional section in the
scene file, `SRCE`, lists each entity's id and the range of instances it became; all that the
terrain grows (the ground, its meadow and woods, the water) is listed as `terrain`, and a
circuit's land as `circuit` (so no entity may have either id). It is a few bytes an entity,
written by every project bake that places anything, and
skipped by every reader that doesn't know it. The editor needs it to turn a click on an
instance into an entity.

**Edits flow into the renderer without a rebake:**

```text
 panel / gizmo / keyboard / agent
            │  command (name, args)
            ▼
     command registry ──► undo stack (inverse recorded)
            │
            ▼
     project (entities, components) ── changed files ──► autosave / save to folder / zip
            │  what each change means for the running scene
            ▼
   ┌───────────────────────────────────────────────────────────┐
   │ transform / hide / glow      → Renderer instance calls     │ live, this frame
   │ weather / time / sea / ponds → existing viewer calls       │ live, this frame
   │ duplicate / place a model    → append instance (its piece) │ live, needs reserve
   │ new model, species, terrain  → bake worker → append piece  │ seconds, in the background
   └───────────────────────────────────────────────────────────┘
```

While a gizmo drags an instance, the editor marks it movable so the cached shadow and GI views
aren't redrawn every frame; when the drag ends it is made static again, which redraws them once.
That is exactly what `set_instance_movable()` was built for.

**Undo** records, for each command, the command that reverses it (`set_transform` with the old
value, `delete` with the entity as it was). A gizmo drag or a slider scrub is merged into one
step when it ends. Undo works on the project, so redoing re-applies to the renderer through the same
path as the first time. Each step records who made it (`user`, `agent`), and agents' steps are
marked in the history panel.

## The command API for agents

One registry of commands, exposed three ways from the editor page:

1. **`window.solstice`** — always present. For an agent driving the page through Claude in
   Chrome's or a browser MCP's JavaScript tool:

   ```js
   await solstice.call("object.move", { id: "gothic_statue", by: [0, 0, 2] })
   // → { ok: true, result: { id: "gothic_statue", position: [-7, 0, 2], ... } }
   await solstice.help()            // every command, its arguments as JSON Schema, and an example
   await solstice.help("object.move")
   solstice.version                 // "1": the API's version, raised when a command changes
   ```

   Results are plain JSON; errors are `{ ok: false, error: { code, message } }` and never
   throw, so a single JavaScript call always returns something the agent can read.
2. **WebMCP**, where the browser offers `navigator.modelContext`
   ([WebMCP proposal](https://github.com/webmachinelearning/webmcp)): the page registers each
   command as a tool with the same name, description and schema, so an agent in such a browser
   sees the editor's tools directly, with no JavaScript in between.
3. **`postMessage`**, for the editor embedded in another page: `{ solstice: 1, id, call, args }`
   in, `{ solstice: 1, id, ok, result | error }` out, only from origins the page was opened with
   (`?embed=ORIGIN`), off otherwise.

The commands (first slice in **bold**):

| Command | Does |
|---|---|
| **`scene.info`** | Scene name, counts, bounds, what can be saved, API version. |
| **`scene.objects`** `{ filter?, kind?, offset?, limit? }` | Objects: id, kind, mesh, position, rotation, scale, hidden, parent. Paged. |
| **`object.get`** `{ id }` | One object, every field the details panel shows. |
| **`object.set`** `{ id, position?, rotation?, scale?, hidden?, … }` | Sets fields; one undo step. |
| **`object.move`** `{ id, by?, to? }`, **`object.rotate`**, **`object.scale`** | Relative and absolute transforms. |
| `object.duplicate`, `object.delete`, `object.rename`, `object.place` `{ asset, at }` | Phase 2. |
| **`selection.get`**, **`selection.set`** `{ ids }` | What is selected, as the panels show. |
| **`camera.get`**, **`camera.set`**, **`camera.frame`** `{ ids? }` | The viewport's camera. |
| **`pick`** `{ x, y }` | The object under a point of the viewport, as a click would select. |
| **`history.undo`**, **`history.redo`**, **`history.list`** | Undo, with who did each step. |
| **`env.set`** `{ time?, weather?, ponds? }` | Time of day, weather, ponds. |
| **`stats.frame`** | Frame time, fps, triangles, clusters, instances, streaming, GPU errors, fallbacks. |
| **`log.read`** `{ since?, level? }` | The output log: editor, script and GPU messages, with ids to read on from. |
| **`wait`** `{ frames?, settled? }` | Resolves after N frames, or when streaming has settled, so an agent reads stats and screenshots of a finished frame. |
| **`project.get`** `{ path? }` | The project's files, or one of them, as they would be saved. |
| `project.save`, `bake.run` `{ scene? }`, `bake.status` | Save, bake in the browser, and follow a bake. Phase 2. |
| `view.capture` `{ width? }` | A PNG of the viewport, for agents without screenshots. Phase 2. |
| `play.start`, `play.stop`, `play.input` `{ action, value }` | Play in the editor and drive it. Phase 2. |
| `asset.import` `{ url }`, `terrain.*`, `foliage.*`, `light.*` | Later phases. |

Everything an agent does goes through the undo stack and shows in the panels as it happens, so
a person watching sees the agent work and can undo it. The API reads and changes only the
scene open in that tab: it has no network access, no file access beyond the project, and
nothing it is given is run as code.

**MCP tools.** A browser MCP gives the agent a page and a way to run JavaScript in it, and that
is all this needs: `javascript_tool` (Claude in Chrome) or `evaluate` (Playwright's and
Chrome DevTools' MCP servers) running `solstice.call(...)`, with their screenshot and console
tools alongside. `docs/editor-api.md`, generated from the registry by `solstice.help()`, is the
reference an agent is pointed at. With WebMCP the same tools appear by name (`object.move`
etc.) and no glue is needed at all. A standalone MCP server is not needed and not planned.

## Performance

- **The viewer pays nothing.** The editor is `editor.html` with its own wasm build (feature
  `editor`); none of its code or panels reach the viewer's page or build. Scenes gain at most
  the `SRCE` section, a few kilobytes, which the viewer skips. The bake library is only in the
  editor's worker, never in the viewer's wasm.
- **On the 5080**, the editor's own cost per frame is the overlay (a few hundred lines and
  handles, drawn as one pass on top) and, when the mouse moves, a one-texel read of the
  visibility buffer for hover. Target: within 0.3 ms of the viewer's frame on Cove and Forest,
  measured with `stats.frame`. Panels are plain DOM; the outliner is virtualised (only visible
  rows exist), so Zorah's instance count costs nothing until scrolled.
- **Dragging** keeps the moved instances movable for the drag only, so the static caches redraw
  once at the end, not every frame.
- **Bakes in the browser** run in a Web Worker and never stall the frame; the viewport shows
  the piece uncompressed until its textures are compressed.
- **On phones**, the editor uses the viewer's existing quality fallbacks and budgets, draws
  its overlay at the render scale, and keeps workers to two. Terrain sculpting rebuilds smaller
  tiles. Target: 30 fps editing Cove on a Galaxy S26.

## Phases

The editor is for making scenes and games from nothing, as Unreal's and Unity's are: a blank
terrain to begin, then land, water, trees, models, lights, weather, actors and their scripts,
all made and changed in the browser. Editing the site's own scenes comes with it, not instead.
Every tool is a command as well, so an agent can build a scene as a person does: sculpt,
plant, place, script and play.

Each phase is one or more reviewable PRs, each shown working in Chrome on the 5080 and checked
on a phone.

1. **Project format and first slice.**
   1. The project format, `bake --convert` and `bake --project`, with `SRCE` (PR #76).
   2. `editor.html` and its build: open a scene and its project, a viewport with fly and orbit
      cameras, GPU picking, move/rotate/scale gizmo with snapping, outliner, details, undo and
      redo, frame, autosave in the browser, save to a file, and the command API on
      `window.solstice` and WebMCP (`docs/editor-api.md`).

   **Milestone**: Claude, through Claude in Chrome, opens the editor on Cove, lists the
   courtyard's entities, moves the statue, reads frame stats and undoes, while you watch.
2. **A new scene from blank terrain.** New makes a scene from a template: terrain, land bands
   and a coastal sea, seen from offshore. With nothing selected, the details panel shows the
   scene's own settings (terrain, land, sea), and a change bakes the scene again in the
   browser and shows it, keeping the camera and the selection. The scene is kept in the browser
   as it changes (`editor.html?draft=NAME`), and saved to a file as any other. Commands:
   `project.new`, `scene.settings`, `scene.set`, `bake.run`, `bake.status`.

   How the bake runs in the browser: the scene building moved out of `apps/bake` into the
   `solstice-bake` library, built for wasm as `apps/bake-web` (7.7 MB, fetched only when the
   editor first bakes) and run in a Web Worker. What it measured, one thread:

   | | Native | Browser |
   |---|---|---|
   | The three land surfaces at 1024², generated | 2.5 min | far longer |
   | A 400 m terrain of 512 cells (524 k triangles), clusters built | 45 s | — |
   | A 256 m terrain of 128 cells, with land and sea, from ready-made surfaces | — | 1.7–2.5 s |
   | The same, 256 cells | — | 15 s |

   So the surfaces, which are the same in every scene, are made once by `bake --surfaces` (in
   the site's build) and fetched ready-made, 4.2 MB each, as ordinary scene files of one
   compressed layer (`solstice_bake::textures`). A new scene's terrain is 2 m cells, which bakes
   in about two seconds; finer terrain waits on a faster cluster builder or on baking the
   terrain in tiles across workers. A scene baked in the browser is byte for byte the one
   `bake --project` makes of the same project natively (checked: the same SHA-256).

   **Milestone**: from an empty page, make an island with sand, grass and sea, change it, reload
   and find it kept; an agent does the same through the API.
3. **Trees, water and weather.** Woods, meadows and clearings as shapes with handles, planted
   and regrown in the worker, area by area; tree species and grass from the built-in ones and
   from species files, edited in a panel; ponds with handles; the scene's weather and time of
   day saved with it. Commands: `foliage.*`, `species.*`, `pond.*`.
   **Milestone**: plant a wood with a clearing and a pond on the island.
4. **Placing things.** A content browser of the built-in models, the scans the site's scenes use,
   and glTF files dropped on the page and baked in the worker; drag into the viewport onto the
   ground; duplicate, delete, box select, End to the ground; lights with handles. The renderer
   takes new meshes, materials, textures and instances after load. Commands: `asset.import`,
   `object.place`, `object.duplicate`, `object.delete`, `light.*`.
   **Milestone**: furnish the island with a few Poly Haven scans and a lamp-lit hut.
5. **Games and scripting.** Actors placed from `.actor` files and new ones made in the editor;
   their properties in the details panel; a script panel for their Starlark with errors marked
   where they are, reloaded as you type; input maps; a player start; Play and Simulate in the
   viewport, with what was edited kept, and the scripts' messages in the output log. Commands:
   `actor.*`, `script.read`, `script.write`, `play.start`, `play.stop`, `play.input`.
   **Milestone**: make the gate puzzle from scratch on the island, and play it; an agent writes
   and fixes a script by reading its errors.
6. **Sculpting and painting.** Terrain brushes (raise, lower, smooth, flatten, noise) and
   painting sand, dirt and grass, rebuilt tile by tile; paths and yards with handles.
   **Milestone**: carve a valley and a road through the island.
7. **Polish to the bar.** Orthographic views, docking layouts, the phone layout's gestures, the
   shortcut sheet, selection outlines in the renderer, and a pass against the table above.

## Coordination

- PR #75 (ponds, willows, oaks) adds to the scene text; the converter covers ponds and the new
  species once it merges.
- The maintenance session is reworking engine code. The first slice moves the bake's scene
  building out of `apps/bake`, touches `index.html` (to move the loader into a module) and adds
  to `web.rs`; those are the places to agree on first.

## Decisions

Approved by the user on 13 November 2026, as recommended:

1. **A project format**: a folder with `solstice.project`, `*.scene` files, the assets they use,
   terrain maps, `.cache/` and `build/`, as above.
2. **JSON for project files**, through `serde` and `serde_json` as direct dependencies, both
   already built through Starlark.
3. **`SRCE` section in the scene file**, written by every project bake.
4. **Publishing each scene's project** beside its baked file, so the site's scenes open in the
   editor.
5. **Scene text retires**: converted to projects one scene at a time, the bake reading both
   until the last has moved.
6. **No new dependencies for the UI**: panels, docking and the gizmo hand-written in plain
   JavaScript modules and WGSL, as the viewer is.
7. **Shortcuts**: Unreal's (W/E/R, F, End) by default, Unity's (Q/W/E/R) as an option.
