# Editor API

The editor's page (`editor.html`, design in `docs/editor.md`) exposes every command it has to
agents. The panels, the keyboard and the gizmo act through the same commands, so an agent can do
whatever a person can, and a person sees it happen and can undo it.

## Calling it

From an agent driving the page with a browser MCP, such as Claude in Chrome's `javascript_tool`
or the `evaluate` tool of Playwright's or Chrome DevTools' MCP servers:

```js
await solstice.call("scene.objects", { filter: "statue" })
// → { ok: true, result: { total: 1, objects: [{ id: "nohUSzZ9", name: "gothic_statue", ... }] } }
await solstice.call("object.move", { ids: ["nohUSzZ9"], by: [0, 0, 3] })
await solstice.call("history.undo")
solstice.help()                  // every command, its arguments as JSON Schema, and an example
solstice.help("object.move")     // one
solstice.list()                  // the names
solstice.version                 // 1, raised when a command changes
```

Every call returns `{ ok: true, result }` or `{ ok: false, error: { code, message } }`, and never
throws. Error codes are `unknown_command`, `bad_arguments` (a missing, misspelt or mistyped
argument, named in the message), `unknown_id`, `no_transform`, `nothing_selected`,
`nothing_to_frame`, `not_a_project`, `bake_failed`, `not_drawing` (a `pick` or `wait` while the tab is hidden, which draws no
frames: bring it to the front) and `failed`.

Changes an agent makes are steps in the same history as a person's, marked `agent` in
`history.list`, and undone the same way.

**WebMCP.** Where the browser offers `navigator.modelContext`
([WebMCP](https://github.com/webmachinelearning/webmcp)), the page also registers each command
as a tool with the same description and schema, its dots made underscores (`object_move`).

**What it can reach.** Only the scene open in that tab: no network, no files beyond the scene's
own project, and nothing given to it runs as code.

## Conventions

- Positions are metres, in the world: +Y is up, the sea lies towards +X in Cove.
- Rotations are degrees about X, Y and Z, applied as the bake applies them: yaw about Y, then
  pitch about the turned X, then roll about the turned Z. The camera's yaw and pitch are radians,
  as the viewer's are; yaw 0 looks along -Z.
- Entities are named by their id, which never changes. Names are for people and may repeat.
- A model seated on the ground (`on_ground` in `scene.objects`) shows the height the bake gave
  it. Moving it keeps that height, and drops its seat: the project then says the height.

## A session

```js
const { result: { objects: [statue] } } = await solstice.call("scene.objects", { filter: "gothic" });
await solstice.call("selection.set", { ids: [statue.id] });
await solstice.call("camera.frame", {});
await solstice.call("object.move", { ids: [statue.id], by: [0, 0, 3] });
await solstice.call("object.rotate", { ids: [statue.id], by: [0, 45, 0] });
await solstice.call("wait", { settled: true });
await solstice.call("stats.frame");          // fps, frame time, triangles, GPU errors...
await solstice.call("log.read", { level: "error" });
await solstice.call("history.undo");
```

Building a scene from blank terrain:

```js
await solstice.call("project.new", { name: "island" });   // baked in the browser in a few seconds
await solstice.call("scene.set", { terrain: { size: 256, cells: 128, height: 12, shore_x: 30 } });
await solstice.call("scene.set", { land: { sand: 18, dirt: 8 } });
await solstice.call("bake.run");                          // waits for the bake, and shows it
await solstice.call("project.get");                       // the scene's file, to save or read
```

A new scene is kept in the browser as it changes (`editor.html?draft=NAME` opens it again), and
its bake in the browser gives the same file `bake --project` gives for it on a PC.

## Commands

### `scene.info`

The scene's name, how many entities and instances it has, whether it can be saved, and the API's version.

No arguments.

### `scene.objects`

The scene's entities, parents before children: id, name, kind, parent, position, rotation (degrees), scale, and how many instances draw it. Paged.

| Argument | Type | Needed | Meaning |
|---|---|---|---|
| `filter` | string |  | Only those whose name or id holds this text (any case). |
| `kind` | string |  | Only this kind: mesh, actor, light, blocker, wood, clear, path, yard, pond or folder. |
| `offset` | number |  | Skip this many (default 0). |
| `limit` | number |  | At most this many (default 100, at most 1000). |

```js
await solstice.call("scene.objects", {"filter":"statue"})
```

### `object.get`

One entity: everything scene.objects gives, its note, and its components as the project holds them.

| Argument | Type | Needed | Meaning |
|---|---|---|---|
| `id` | string | yes | The entity's id, as scene.objects gives it. |

### `object.set`

Sets an entity's fields: position, rotation (degrees about X, Y, Z), scale, name, note, or hidden (in the editor only). One undo step.

| Argument | Type | Needed | Meaning |
|---|---|---|---|
| `id` | string | yes | The entity's id, as scene.objects gives it. |
| `position` | [x, y, z] |  | Where it is, in metres. |
| `rotation` | [x, y, z] |  | Degrees about X, Y and Z: yaw about Y, then pitch, then roll. |
| `scale` | number |  | Uniform scale, above 0. |
| `name` | string |  | Its name in the outliner. |
| `note` | string |  | What it is for. |
| `hidden` | boolean |  | Hidden in the editor's viewport. Not saved. |

```js
await solstice.call("object.set", {"id":"a91fXq2b","position":[-7,0,2]})
```

### `object.move`

Moves entities by an offset, or to a position, in metres. One undo step.

| Argument | Type | Needed | Meaning |
|---|---|---|---|
| `ids` | string[] |  | The entities to change; the selection if left out. |
| `by` | [x, y, z] |  | How far to move, in metres along X, Y and Z. |
| `to` | [x, y, z] |  | Where to put them. |

```js
await solstice.call("object.move", {"ids":["a91fXq2b"],"by":[0,0,2]})
```

### `object.rotate`

Turns entities: `by` degrees about the world's X, Y and Z axes (applied in that order), or `to` a rotation. One undo step.

| Argument | Type | Needed | Meaning |
|---|---|---|---|
| `ids` | string[] |  | The entities to change; the selection if left out. |
| `by` | [x, y, z] |  | Degrees to turn about the world's X, then Y, then Z axis. |
| `to` | [x, y, z] |  | The rotation to set: degrees about X, Y and Z, as object.set takes it. |

```js
await solstice.call("object.rotate", {"by":[0,90,0]})
```

### `object.scale`

Scales entities evenly: `by` a factor, or `to` a scale. One undo step.

| Argument | Type | Needed | Meaning |
|---|---|---|---|
| `ids` | string[] |  | The entities to change; the selection if left out. |
| `by` | number |  | A factor, above 0. |
| `to` | number |  | The scale to set, above 0. |

```js
await solstice.call("object.scale", {"by":1.5})
```

### `selection.get`

What is selected, as scene.objects describes it.

No arguments.

### `selection.set`

Selects entities by id (an empty list clears the selection), as clicking them does.

| Argument | Type | Needed | Meaning |
|---|---|---|---|
| `ids` | string[] | yes | The entities to select. |

```js
await solstice.call("selection.set", {"ids":["a91fXq2b"]})
```

### `camera.get`

The viewport's camera: position, yaw and pitch (radians; yaw 0 looks along -Z), field of view (degrees) and fly speed (m/s).

No arguments.

### `camera.set`

Moves the viewport's camera: a position, a yaw and pitch (radians), or a point to look at.

| Argument | Type | Needed | Meaning |
|---|---|---|---|
| `position` | [x, y, z] |  | Where the camera is. |
| `yaw` | number |  | Radians about Y; 0 looks along -Z. |
| `pitch` | number |  | Radians above the horizon. |
| `look_at` | [x, y, z] |  | A point to face, instead of yaw and pitch. |
| `speed` | number |  | Fly speed, metres a second. |

```js
await solstice.call("camera.set", {"position":[40,6,20],"look_at":[0,2,0]})
```

### `camera.frame`

Moves the camera to see entities whole (the selection if no ids), as F does.

| Argument | Type | Needed | Meaning |
|---|---|---|---|
| `ids` | string[] |  | The entities to frame; the selection if left out. |

### `pick`

What is under a point of the viewport, given as fractions of its width and height from the top left: the entity (null for the land or sky), the instance and the point hit. Selects nothing.

| Argument | Type | Needed | Meaning |
|---|---|---|---|
| `x` | number | yes | 0 at the left edge, 1 at the right. |
| `y` | number | yes | 0 at the top, 1 at the bottom. |

```js
await solstice.call("pick", {"x":0.5,"y":0.5})
```

### `history.undo`

Undoes the last change, by anyone.

No arguments.

### `history.redo`

Redoes the last change undone.

No arguments.

### `history.list`

The changes that can be undone (oldest first) and redone (next first), each with who made it: user or agent.

No arguments.

### `env.set`

The time of day (hours, 0 to 24), the weather (by name, or `changing`), and the ponds (x0,z0,x1,z1,level; with ; between). Not saved yet.

| Argument | Type | Needed | Meaning |
|---|---|---|---|
| `time` | number |  | Hours, 0 to 24. |
| `weather` | string |  | changing, or a weather's name: env.weathers lists them. |
| `ponds` | string |  | Ponds as x0,z0,x1,z1,level, with ; between them. |

```js
await solstice.call("env.set", {"time":18.5,"weather":"fair"})
```

### `env.weathers`

The names env.set takes for the weather.

No arguments.

### `stats.frame`

How the viewport is drawing: frames a second, milliseconds a frame, triangles, clusters, instances, geometry pages, megabytes fetched, GPU errors and any fallbacks for this GPU.

No arguments.

### `log.read`

The output log: the editor's messages, warnings and errors, each with a number; `since` gives only those after one.

| Argument | Type | Needed | Meaning |
|---|---|---|---|
| `since` | number |  | Only entries numbered above this. |
| `level` | string |  | Only info, warn or error. |

### `wait`

Resolves after some frames have been drawn, or once streaming has settled, so stats and screenshots show a finished frame.

| Argument | Type | Needed | Meaning |
|---|---|---|---|
| `frames` | number |  | Frames to wait (default 2, at most 600). |
| `settled` | boolean |  | Wait until no geometry has arrived for half a second (at most 30 s). |

### `project.get`

The scene's project file as it would be saved, and whether it has changes not yet saved.

No arguments.

### `project.new`

Starts a new scene from blank terrain: an island of sand, dirt and grass in a coastal sea, baked in the browser and kept there as it changes. The scene shown now is left; save it first if it has changes.

| Argument | Type | Needed | Meaning |
|---|---|---|---|
| `name` | string |  | Its name: letters, digits, - and _ (default untitled). |

```js
await solstice.call("project.new", {"name":"island"})
```

### `scene.settings`

The settings a scene has one of: its terrain, land bands, sea, texture size and starting camera, as the project holds them, with each field's range. Each is null if the scene has none.

No arguments.

### `scene.set`

Changes the scene's settings, each given whole, or null to take it away. One undo step. The scene bakes again in the browser; `bake.run` waits for it.

| Argument | Type | Needed | Meaning |
|---|---|---|---|
| `terrain` | object or null |  | `{ size, cells, height, shore_x }`: metres across (16 to 4096), cells along a side (8 to 1024; more takes longer), hill height in metres, and the x where the shore begins (the sea lies towards +X). Null for no ground. |
| `land` | object or null |  | `{ sand, dirt }`: metres of sand from the water, then of dirt; grass beyond. Null for plain ground. |
| `sea` | object or null |  | `{ level, ocean }`: the calm sea's height, and a built-in ocean (`coastal`). Null for no sea. |

```js
await solstice.call("scene.set", {"terrain":{"size":256,"cells":128,"height":6,"shore_x":40}})
```

### `bake.run`

Bakes the scene in the browser now, or waits for the bake already asked for, and shows it. Gives how long it took; a bake that fails is a `bake_failed` error with the bake's message.

No arguments.

### `bake.status`

Whether a bake is running or waiting to, and how the last one went.

No arguments.

### `gizmo.set`

The gizmo: its mode (move, rotate, scale), its space (world, local), and snapping (on, and its steps).

| Argument | Type | Needed | Meaning |
|---|---|---|---|
| `mode` | string |  | move, rotate or scale. |
| `space` | string |  | world or local. |
| `snap` | boolean |  | Whether drags snap. |
| `move_step` | number |  | Metres a move snaps to. |
| `angle_step` | number |  | Degrees a turn snaps to. |
