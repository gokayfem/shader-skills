# Gameplay: design

A design for making Solstice's worlds playable: things that move and collide, characters that
are skinned and animated, and game logic written in a Python-like language. The same game runs
in the browser and natively. This document is the plan, written before any of it is built, and
it will be kept up to date as each step lands.

## Goals

- **Built like Unreal.** A world of actors made of components, ticked in a fixed order of tick
  groups, talking through events. Engine systems are written in Rust where speed matters, and
  game logic is written in scripts. Unreal splits its work the same way: C++ beneath, with
  Blueprint above (and Verse, so far only in Unreal Editor for Fortnite).
- **Python-like scripting**, the same language natively and on the web, with:
  - hot reload while the game runs, keeping each actor's state;
  - a sandbox: a script can reach only what the engine hands it, and a runaway script is
    stopped rather than freezing the page;
  - errors that name the file and line.
- **Modular.** Each part is its own crate and can be tested without a GPU. The renderer knows
  nothing about gameplay: it gains a few small, general APIs (things that move, skinned meshes),
  and nothing more. A scene without gameplay loads and costs exactly what it does today.
- **Fast on the web, integrated GPUs included.** The web build has one thread. Wasm threads
  need the page to be cross-origin isolated, and GitHub Pages cannot send the headers for that.
  The usual workaround, a service worker that adds them, reloads the page on the first visit and
  blocks cross-origin fetches, and the scenes are fetched from their own Pages sites. The budget
  below assumes one thread, and every step measures against it.
- **Deterministic simulation.** Physics, script ticks and gameplay events advance in fixed
  steps, from input recorded per step. So a test can run a scene for N steps and check where
  everything ended up, a replay gives the same result, and the native and web builds agree.
  Only what is drawn (animation poses, the camera) follows the frame rate.
- **Gameplay drives the world.** Scripts can set the time of day, the weather and the sea, and
  react to them. The renderer already eases the weather and the ocean; gameplay sets where they
  should go, and eases the rest itself.

Not in the first version, with room left for each:
- networking and multiplayer;
- an editor beyond text files and hot reload;
- audio;
- navigation meshes and AI;
- cloth, soft bodies and destruction;
- ragdolls, which come after the IK step.

## What there is today

- **The scene is static.** `.solstice` files hold meshes as cluster DAGs, materials, instances
  and lights. Instances are uploaded once (`scene.rs`, `GpuInstance`) and never change. Only the
  ocean's surface writes its own instance buffer each frame.
- **Motion vectors are half ready.** `GpuInstance` already has `prev_world` next to `world`, and
  resolve uses it for each pixel's motion (`vg/resolve.wgsl`). Today the two are always equal.
- **Several passes assume nothing moves:**
  - sun shadow cascades are cached and only redrawn when their view moves (`shadow.rs`);
  - GI voxel levels are rebuilt only when the camera has moved far enough (`gi/volume.rs`);
  - the seabed map is redrawn only when the camera moves 128 m or the sea level changes;
  - wetness (`docs/wetness.md`, `ocean/waterline.rs`) remembers how high the water has stood
    at each spot, in a map fixed in the world. It draws no geometry, but the wetness it gives
    belongs to a place, not to an object: a hull that sails away leaves its wet band behind.

  Each of these has to be told when something moves, or ignore things that move.
- **Two-phase occlusion culling** (`vg/culling.rs`) already copes with motion: anything the old
  depth hid wrongly is retried against the new depth in the same frame.
- **Vertices** are 12 bytes on one 16-bit grid per mesh: position, a packed normal and UV
  (`x y z n u v`, `solstice-asset/src/gpu.rs`; 16 bytes with a spare value until file version 5,
  see `docs/streaming.md`). The grid has one step
  for all three axes, a 65,535th of the mesh's longest side (`Mesh::grid_step`), so a mesh
  that is very long keeps its points coarsely however thin it is: a kilometre of road as one
  mesh, to 15 mm. There are no bone
  weights. The importer warns that it ignores skins and animations
  (`solstice-import/src/lib.rs`), and imports models in their rest pose.
- **Dynamic attributes:**
  - `set_weather(Weather, seconds)`, eased by the renderer over that many seconds, a weather
    cycle (`set_weather_changing`), and reports from real observations (`Weather::observed`);
  - `set_ocean(Option<Ocean>, seconds)`, eased the same way;
  - `set_sea_level(level)`, which takes effect at once;
  - `TimeOfDay`: `advance(hours)`, which just adds hours, and from it the sun, moon, stars and
    `Environment`;
  - the weather and the ocean have text forms (`.weather`, `.ocean` files) with `parse` and
    `to_text`. The time of day is read from the viewers' flags (`parse_time` in
    `apps/viewer/src/lib.rs`).
- **The viewers** drive these directly. The native viewer has command-line flags, and the web
  page calls `set_time`, `set_weather` and the rest from JavaScript (`apps/viewer/src/web.rs`,
  `apps/viewer/web/index.html`). The camera is a free-flying viewer camera. There is no input
  system beyond it.
- **The ocean exists only on the GPU.** `spectrum.rs` has a CPU reference: the spectrum, the
  seeded starting amplitudes (`pcg`, `gaussian_pair`, `initial_amplitude`) and `direct_sum`.
  Tests use it to check that the GPU draws exactly the waves the reference does (`waves.rs`).
  Waves shrink over the seabed in `shoal()`, which exists only in WGSL
  (`shaders/ocean/seabed.wgsl`). Nothing reads wave heights back during play.
- **Scene files** take optional tagged sections, such as `SEA `, which older readers skip. So
  actors, collision and scripts can be added without a new file version.

## References

- **Unreal Engine.** The gameplay framework: `UWorld`, `AActor`, `UActorComponent` and
  `USceneComponent`, tick groups (`TG_PrePhysics`, `TG_DuringPhysics`, `TG_PostPhysics`,
  `TG_PostUpdateWork`), tick intervals, timers and delegates, GameMode, PlayerController and
  Pawn, and Enhanced Input. Also Mass, Unreal's ECS for crowds; Blueprints, a visual language
  run by a VM and slower than C++, which is why Epic advises keeping heavy loops out of
  Blueprint ticks; Verse, a language Epic designed itself, so far only in Unreal Editor for
  Fortnite; and the Animation Blueprint, blend spaces, Control Rig, IK Rig, IK Retargeter and
  the Animation Budget Allocator.
- **Nanite skinning.** Unreal 5.5 added experimental support for drawing skeletal meshes
  through Nanite. Epic's documentation on Nanite foliage, which is animated by the same
  skinning, says that bounds can be computed from the skinning matrices, so the hardware
  rasterizer can be used. Epic has published little more, so the skinning design here is our
  own, and step 7 proves it out before anything depends on it.
- **Godot.** A tree of nodes, signals for events, and GDScript: a Python-like language written
  for the engine, with optional static types and hot reload. It is the clearest example of an
  engine building its own language, and of what that costs: years of work on the language, its
  tools and its editor support.
- **Starlark** (Bazel's configuration language) and its Rust implementation `starlark-rust`,
  from Meta. **Monty**, Pydantic's sandboxed Python subset in Rust. **RustPython**. **Pyodide**,
  CPython compiled to wasm. **Rhai**. **Luau**, Roblox's sandboxed Lua.
- **Rapier**, from Dimforge: rigid bodies, joints, character controllers and scene queries in
  pure Rust. It compiles to `wasm32-unknown-unknown`, and Dimforge also publishes it to npm
  for JavaScript games.
- **Animation.** Kavan et al., *Skinning with Dual Quaternions* (2007). Frechette's Animation
  Compression Library, for quantising clips. ozz-animation's sampling and blending. Aristidou and
  Lasenby, *FABRIK* (2011). Clavet, *Motion Matching* (GDC 2016), for later.
- **Water.** *The Technical Art of Sea of Thieves* (GDC 2018): ships float by sampling the same
  waves that are drawn.

## How the engine fits together

```text
                 scripts (.star)        actor files (.actor)       scene (.solstice)
                       │                        │                         │
                       ▼                        ▼                         ▼
 input ──▶ ┌──────────────────────────────────────────────────────────────────┐
           │ solstice-world   entities, components, transform tree,           │
           │                  tick groups, events, timers, fixed clock        │
           ├──────────────┬───────────────┬────────────────┬──────────────────┤
           │ solstice-    │ solstice-     │ solstice-anim  │ environment:     │
           │ script       │ physics       │ clips, blends, │ time of day,     │
           │ (Starlark)   │ (Rapier)      │ state machines,│ weather, sea     │
           │              │               │ IK             │                  │
           └──────────────┴───────────────┴────────────────┴──────────────────┘
                                         │ extract, once per frame
                                         ▼
           solstice-render: moving instances, bone palettes, lights, environment
```

- **`solstice-world`**: the framework. It has no GPU, no physics and no scripting, so every
  other crate can depend on it, and it tests headless.
- **`solstice-script`**, **`solstice-physics`** and **`solstice-anim`** each add their
  components and systems to a world. Each is optional, and each tests on its own.
- **`solstice-game`**: the glue. It runs the frame, owns input, and extracts what the renderer
  needs at the end of each frame. The viewers use it only when a scene has gameplay.
- **The renderer gains general APIs only:** move an instance, hide it or change its glow, upload
  a skeleton's bone matrices. It never sees an actor.
- **Two web builds**, chosen on 8 November 2026. The viewer stays as it is (319 KB gzipped
  today). A second build, with gameplay, is loaded only for scenes that have it, when *Play* is
  pressed.

## The gameplay framework

### Actors and components

Actors are entities, and components are plain Rust structs stored by an ECS. This is how
Unreal's actors and components behave, with Mass's speed when there are thousands of them.

| Unreal | Solstice |
|---|---|
| `UWorld` | `World` |
| `AActor` | an entity with a `Transform`, a `Name`, and optionally a `Parent` |
| `UActorComponent`, `USceneComponent` | components: `Mesh`, `RigidBody`, `Collider`, `Skeleton`, `Animator`, `Script`, `Light`, `Camera`, `Trigger`… |
| Blueprint class | an actor file (`.actor`): its components, their settings, and a script |
| Blueprint graph, Verse | a Starlark script |
| C++ gameplay code | Rust systems, in game crates |
| Tick groups, tick interval | the same, below |
| Delegates, events | events, delivered between tick groups |
| `FTimerManager` | timers |
| GameMode, PlayerController, Pawn | a game-rules script, a `Controller`, and a `Pawn` component |
| Enhanced Input | actions mapped to keys, buttons and touches in a text file |

An actor file follows the style of the `.ocean` and `.weather` files:

```text
# scenes/cove/gate.actor
name gate
mesh ../../target/scan/large_iron_gate/large_iron_gate.gltf 1.15   # a model, and its scale
collider box 1.7 2.4 0.1             # from step 5
script gate.star
property open_seconds float 2.5      # editable per placement in the scene
property locked bool false
```

A scene places it as it places models, at x, y, z and a yaw in degrees, with its own settings:
`actor cove/gate.actor 20.7 0 0 90 locked=true`.

**Which ECS.** Both candidates were measured, by moving 10,000 entities by their velocity 100
times, natively and on the web, set up as for the scripting benchmarks below (one run each):

| | Download (gz) | Native, per entity | Web, per entity |
|---|---|---|---|
| `hecs` 0.11 | 31 KB | 0.90 ns | 1.58 ns |
| `bevy_ecs` 0.20, a plain query | 160 KB | 0.67 ns | 1.85 ns |
| `bevy_ecs` 0.20, a system in a schedule | | 0.92 ns | 1.28 ns |

They run at about the same speed:
- `hecs` is small and stable;
- `bevy_ecs` brings schedules, change detection and observers, and breaks its API every few
  months.

The framework only needs fixed tick groups on one thread, which is a few hundred lines on top of
`hecs`. So the choice, made on 8 November 2026, is `hecs`, with Solstice's own framework over
it.

### A frame

```text
 1. Input        gather keys, mouse, pads and touch; map them to actions
 2. Fixed steps  0 to 3 steps of 1/60 s, each one:
      a. Start        the step's input is taken; timers due on it deliver their events
      b. PrePhysics   Rust systems, root motion, then script ticks for the actors that tick
      c. Physics      one Rapier step; contacts and triggers are queued
      d. PostPhysics  hits and overlaps delivered to scripts
      e. Transforms   every entity's place in the world, keeping the step before
 3. Animation    state machines read gameplay's values; sample, blend, IK
 4. Late         the camera follows
 5. Extract      changed transforms, bone palettes, lights and environment go to the renderer
```

As built (step 2, `crates/solstice-world`): `Game::frame` runs items 1 to 4 of this list.
Extraction (item 5) comes with roadmap step 4, *Scenes, actors and play mode*. Items 3 and 4
are the per-frame tick groups `Animation` and `Late`.

- **Fixed steps.** Everything that decides what happens runs inside the steps, at 60 Hz from an
  accumulator: events, script ticks, Rust gameplay systems and physics. Input is gathered per
  frame but handed out per step, and recorded that way for replays. A frame runs at most three
  steps; past that, the game slows down rather than falling behind in a spiral.
- **Per frame.** Animation, the camera and extraction run once per frame, at the frame's own
  time. They only affect what is drawn, so they need not be deterministic.
- **Interpolation.** Rendering draws each body between its last two physics states, at the
  accumulator's fraction. Each instance's `prev_world` is the transform it was actually drawn
  with last frame, kept by the extract step, not the previous physics state: the two differ
  whenever a frame runs no step or several.
- **Root motion** is sampled in PrePhysics, for the step's length, and drives the character
  controller in that step's physics, as Unreal does for characters. The full pose is still
  sampled per frame.
- **Most actors never tick.** As in Unreal, ticking is opt-in, with an interval. Scripts mostly
  react to events: an overlap, an input action, a timer, the hour changing.
- **Events are queued** and delivered between tick groups. A handler never runs halfway through
  another system, so the order is always the same and replays give the same result.
  - Each event is stamped with the step and group that sent it. Every fixed-step group sees it
    exactly once, within a step: what was sent before it in this step, and what was sent from
    its own place on in the step before. So a script's event in PrePhysics reaches handlers in
    PostPhysics of the same step, and in PrePhysics of the next.
  - Events sent before a step (while setting up, or by timers as the step starts) are seen by
    all of that step's groups.
  - The per-frame groups see everything sent since they last ran, once, however many steps the
    frame held. They send no events, set no timers and spawn nothing, and that is checked: they
    run as often as frames come, which differs from run to run, and a replay doesn't run them.
    They may write only what is drawn (poses, the camera).
- **Input** is the frame's values, plus the actions pressed since the last step, so a tap
  between two steps isn't lost. The per-frame groups also see the frame's own input, for a
  camera that turns even in frames with no step.
  - It also carries where the game is watched from, the camera's place, which scripts read as
    `viewer()`. A camera flown freely is not an actor, and belongs to no step; being input, it
    is recorded, so a replay puts it where it was.
- **Replays are exact on the same build and platform.** Native and web builds may round some
  maths differently, such as sines in `glam` and `libm`.
- **Spawning and destroying** are deferred to the end of the tick group that asks for them.
- **Timers** count steps, not seconds, so they fire on the same step in every run.
- **Teleports** mark the entity `Snap`, so it is drawn at its new place at once, not sliding there.
- **Later steps will need:** animation notifies (footsteps, hit frames) worked out in the fixed
  step, alongside root motion, since the per-frame groups can't send events; and, for physics,
  places worked out again before the physics group when something moved earlier in the step.

### Where work runs

The rule, borrowed from Unreal: **scripts decide, Rust does the work.** A script sets a
character's desired speed, opens a gate or starts an animation state. The Rust systems then do
the per-frame, per-entity work: integration, physics, sampling, IK. This keeps the number of
crossings between script and Rust down to tens or hundreds per frame, not thousands, whichever
language is chosen.

## Scripting

### What was measured

Each candidate ran the same three scripts:
- an empty function called from Rust;
- `tick(dt)`, which calls two Rust functions, `get_x()` and `set_x(x + dt)`, as a script that
  moves something does;
- a loop adding up a million floats, for raw interpreter speed.

Setup:
- **Native:** Rust 1.98 release builds with thin LTO, on a Ryzen 7 9800X3D under Windows 11.
  Starlark 0.14, Rapier 0.36 and `hecs` 0.11 also build with the project's pinned Rust 1.95.
- **Web:** the same code built for `wasm32-unknown-unknown`, run in Node 24 on the same machine.
  Node uses V8, Chrome's engine.
- **Sizes:** the wasm after `wasm-bindgen`, with name sections removed, gzipped. There is no
  `wasm-opt` pass, which would shrink them a little. Pyodide's are its release files, gzipped.
- **Runs:** each figure is the best of three, and each benchmark was run two or three times;
  where the runs differed, the table gives the range. The code is in `tools/gameplay-bench`,
  with how to run it.
- **Start-up** on the web includes V8 compiling the module on its first run.

| | Download (gz) | Start-up | Empty call | `tick`, 2 Rust calls | Loop, per iteration | Python |
|---|---|---|---|---|---|---|
| **Starlark** 0.14, native | | 3.3 ms | 18 ns | 47 ns | 28 ns | syntax, a subset |
| **Starlark**, web | 1.33 MB | 26–31 ms | 33–43 ns | 89 ns | 54–72 ns | |
| **RustPython** 0.6, native | | 11 ms | 54 ns | 204 ns | 60 ns | nearly all of the language |
| **RustPython**, web | 4.15 MB, no stdlib | 77 ms | 63 ns | 233 ns | 105 ns | |
| **Monty** 1.5, native | | 0.9 ms | 1,730 ns | 4,470 ns | 80 ns | most of it: classes, try, while |
| **Monty**, web | 3.07 MB | 19–24 ms | 2,400–3,800 ns | 6,300–11,700 ns | 80–144 ns | |
| **Pyodide** 0.28.3, web only | 5.4 MB | 1.7–2.6 s | 140–240 ns | 810 ns | 55–82 ns | all of CPython 3.13 |
| **Rhai** 1.27, native | | 1.3 ms | 41 ns | 444 ns | 193 ns | no, Rust-like |
| **Rhai**, web | 0.56 MB | 15–25 ms | 44–70 ns | 600–670 ns | 260–320 ns | |
| Plain Rust, web, for scale | | | | 3 ns | 1 ns | |

The viewer's own wasm is 319 KB gzipped, so every interpreter here is larger than the rest of the
engine. Next to the scenes, though, it is small: the cove streams up to 209 MB. Size matters for
time to first frame, not for bandwidth.

What each one means:

- **Pyodide** is real CPython with its whole ecosystem, but it is the wrong fit for an engine.
  - It takes 1.7–2.6 s to start, and downloads 5.4 MB (2.8 MB of wasm, 2.4 MB of standard
    library).
  - It is a separate Emscripten module, so every call between a script and the engine goes
    through JavaScript twice.
  - It has no native counterpart: natively the engine would embed CPython instead, a different
    runtime with different packaging.
  - It has no sandbox. A script can reach everything the page can, through its `js` module.
  - It could still serve later as an optional console for tools, never as the game's language.
- **RustPython** runs nearly all of Python, with reasonable calls (233 ns on the web).
  - It is the largest download at 4.15 MB, even with no standard library, and takes 77 ms to
    start.
  - Its release did not build as published: one of its dependencies broke compatibility in a
    patch release, and had to be pinned back by hand.
  - Its embedding API offers no limit on running time that we could find, so an endless loop
    would hang the page.
- **Monty** is the most promising real Python. It runs classes, dataclasses, `try`, `while`,
  `global` and `math` (each checked), though not yet `yield`. It has time and memory limits, and
  starts in under a millisecond natively.
  - Its interface between Rust and scripts is built for an AI agent's tool calls, not a game
    loop. Each call from Rust compiles a small snippet, at 2.4 µs or more on the web. Each call
    from a script back into Rust suspends the interpreter and resumes it, at 2–4 µs on the
    web. A whole `tick` costs 70 to 130 times as much as Starlark's.
  - It is 3 MB, calls itself experimental, and needs Rust 1.96, newer than the project's
    pinned 1.95.
- **Starlark** is the fastest at every crossing and has the best tools.
  - Calls in both directions cost tens of nanoseconds, a thousand ticking scripts cost about
    0.09 ms a frame on the web, and on the web its loops are as fast as Pyodide's CPython.
  - It has optional type annotations with a checker, and a language server (the `starlark_lsp`
    crate) for editor support.
  - It is sealed off from the system: there is no I/O, no clock and no randomness unless the
    engine provides them. The engine can cap each call's work with `set_max_tick_count`, which
    counts steps and so stops a script at the same point natively and on the web, and its memory
    with `set_max_heap_size`. `starlark-rust` does not claim to be safe against hostile code,
    which is fine here: the scripts ship with the scene, written like the rest of it, and are
    not uploads from strangers.
  - It is not full Python. Checked against `starlark-rust` 0.14: no `class`, `while`, `try` or
    `global`. Recursion, lambdas, list comprehensions, type annotations and (with a dialect
    flag) f-strings all work. Values made at load time are frozen: a script cannot keep state in
    a global.
- **Rhai** is small and mature, but its syntax is Rust-like rather than Python-like, and it was
  slower than Starlark at every call and loop measured. It is here for contrast.
- **Luau**, for contrast, was not measured. Its VM is C++, which does not link into a
  `wasm32-unknown-unknown` build. It would need its own Emscripten module and JavaScript glue,
  with Pyodide's double crossing. It also isn't Python-like.
- **A language of our own**, Python syntax compiled to the engine's own bytecode (or to wasm),
  is the GDScript path. With static types it could beat every number above and be small. But it
  means writing a parser, type checker, VM, error messages, debugger and editor support: years
  of work, as Godot and Epic (Verse) both found. Compiling to wasm has a second problem: the
  browser can run wasm, but the native build would need a wasm runtime such as Wasmtime as well.

### The choice: Starlark

**Starlark, through `starlark-rust`.** Chosen on 8 November 2026, from the measurements above.
- It is Python to read and write: `def`, indentation, lists, dicts, comprehensions, `if`, `for`.
- It is the fastest at the crossings a game makes all the time.
- It is sealed off from the system, can be stopped, and has types and an editor language
  server.
- It behaves the same natively and on the web.

Its gaps fit the framework, and are filled by it:

- **No classes:** an actor's class is its `.actor` file. The script is the functions that
  handle its events, and `self` is a handle to the actor.
- **No mutable globals:** an actor's state lives in its components, as typed `property` values
  declared in its `.actor` file, and in a per-actor `self.vars` for anything else. This is also
  what makes hot reload keep state: none of it lives in the script.
- **No `while`:** loops run over ranges, lists and queries. Recursion is allowed, so a handler
  can still run away; the step cap stops it, and the error is reported like any other.
- **No `try`:** a failing handler stops, its error and line go to the on-screen log, and the
  actor carries on with its next event.

```python
# scenes/cove/gate.star
def on_begin(self):
    self.vars.open = False

def on_interact(self, who):
    if self.locked:
        say(who, "The gate is locked.")
        return
    self.vars.open = not self.vars.open
    target = 90.0 if self.vars.open else 0.0
    tween(self, "rotation.yaw", target, self.open_seconds)

def on_hour(self, hour):
    if hour == 21:
        self.locked = True   # the fort closes at night
```

**How scripts run:**
- **Loading.** Each script file is parsed, type-checked, run once and frozen. Its handler
  functions become frozen values shared by every actor that uses the script.
- **Each frame,** events are handled in a fresh temporary heap, which is dropped when the frame
  ends. So nothing a frame allocates can outlive it, and there is no garbage collector to tune.
  Step 3 confirms that frozen handlers run this way at the measured cost.
  Anything worth keeping goes into `self.vars` or a component, where it is stored as an engine
  value (number, bool, string, vector, entity, list, dict). Lists and dicts there are owned by
  the engine and changed in place, so `self.vars.path.append(p)` keeps `p`.
- **Hot reload.** When a file changes (a file watcher natively, a reload button on the web), it
  is reloaded and the actors switch to the new handlers on their next event. State stays,
  because it lives in components. If the new file fails to load, the old one keeps running and
  the error is shown.
- **The engine's API** is written once in Rust, as a table of functions with typed arguments.
  Starlark's bindings are generated from it, and so is a `.pyi`-style stub for editors. If
  Monty's interface becomes fit for a game, or Solstice ever grows its own language, only the
  generated layer changes; the API and the game's scripts stay put.

**As built (step 3, `crates/solstice-script`):**
- **Handlers.** `on_begin(self)` runs once, the first step an actor has the script.
  `on_tick(self, dt)` runs every step, but only for scripts that have one. `on_<name>(self, ...)`
  runs for each `ScriptEvent` named `<name>`, sent to the actor or to every actor, by a script
  or by Rust.
  - Each step they run in `PrePhysics`: beginnings, then events in the order they were sent,
    then ticks, each in the world's order of entities.
  - An event a script sends, even to itself, is handled the next step: one step, 1/60 s, of
    latency between one actor and the next.
- **`self`** is a copy of the actor on the call's heap:
  - its properties, which can be set but not added;
  - `self.vars`, where plain data can be kept: numbers, strings, `vec3`s, entities, and lists
    and dicts of those, at most 64 deep and 100,000 values, and never containing themselves;
  - `self.position` and `self.yaw`, written back to its `Transform`, local to its `Parent` if it
    has one; an actor without a `Transform` gets one when it first moves, and both must stay
    finite;
  - `self.entity`, which can be a dict key.

  Lists in `self.vars` are changed in place within a call, and kept.
- **A failing handler changes nothing but the log.** Its copy is thrown away, and what it asked
  of the world (events to send, timers to set or cancel) is held back until it succeeds, so an
  actor is never left half-changed. Its log lines are kept, to show why it failed; what failed,
  with file and line, goes to the host's errors.
- **Timers** set by `after` and `every` belong to the actor: their handles are its own, and
  `cancel` refuses, returning `False`, another actor's. `begin` and `tick` are the engine's own
  events, and can't be sent.
- **Loading.** Each script is parsed, run once and frozen, and its handlers are shared. A script
  that fails to load, or whose `on_*` names aren't functions, leaves the old one running. Each
  call gets a temporary heap of its own, dropped when the call ends; sharing one heap across a
  step was tried, and was slower. Type checking is left to editors for now.
  - Reloads aren't part of a `Recording`: a replay runs the scripts loaded when it starts.
- **Limits.** Each call, and each load, may do 100,000 of Starlark's steps of work, and hold
  16 MiB.
  - Counting work, not time, stops a runaway script at the same point natively and on the web:
    tested, at the same 100 lines of its log, plus one counting those dropped, on both.
  - It stops in about 4 ms on the web: one hitch, not a hang.
  - The memory limit is best effort. Starlark checks it now and then, and the host once more as
    the call ends, from Starlark's own count of what it allocated. That count is of values,
    which are larger on 64-bit than on wasm32, so a script near the limit may pass on the web
    and fail natively.
  - A call keeps 100 lines of log, of up to 1,000 characters each.
- **The API** is the one `#[starlark_module]` in `api.rs`: `vec3`, `send`, `after`, `every`,
  `cancel`, `find`, `input`, `held`, `pressed`, `viewer`, `now` and `log`. Starlark's own
  documentation, `api_reference()`, renders it for people and editors, in place of a hand-written
  stub.
- **Cost** (`tools/gameplay-bench/b_script`, 9800X3D):
  - about 1.1 to 1.3 µs a ticking actor on the web, and 1.9 to 2.4 µs natively, most of it
    setting up each call;
  - so the 1 ms scripts line of the budget holds about 400 actors ticking every step on an
    integrated-GPU laptop. Most actors don't tick: they wait for events.
  - Starlark adds 1.4 MB, gzipped, to the gameplay web build.
- **No `unsafe` code.** A frozen handler is called from a temporary heap by allocating it on the
  call's frozen heap, which keeps its own heap alive for the call.
- **`Name`** moved into `solstice-world`, since the world's own tools will find entities by it.

## Physics

**Rapier**, version 0.36, in `solstice-physics`.
- It is pure Rust, so it is the same code natively and on the web, inside the one wasm module.
- It has rigid bodies, joints, sensors, a kinematic character controller, ray and shape casts,
  and cross-platform determinism as an option.
- The alternatives don't fit. Jolt and PhysX are C++ and would need Emscripten, with the same
  problem as Luau. Avian is built around Bevy.

### What was measured

Boxes dropped in a jumbled pile on the ground. The table gives the mean step over the first two
seconds, while most are still moving (afterwards they fall asleep and cost almost nothing), and
1,000 raycasts down into the pile:

| Boxes | Native | Web | Web with SIMD | Worst step, web with SIMD | 1,000 rays, web |
|---|---|---|---|---|---|
| 100 | 0.16 ms | 0.59–0.66 ms | 0.35–0.37 ms | 11 ms (V8 still compiling) | 0.4–0.5 ms |
| 500 | 0.87–0.89 ms | 2.5–2.6 ms | 1.13–1.17 ms | 4.5–4.9 ms | 0.4–0.5 ms |
| 1,000 | 1.85–2.02 ms | 5.1–5.3 ms | 2.35–2.44 ms | 9.4–10.3 ms | 0.4 ms |

- **SIMD halves the web cost.** Built with `-C target-feature=+simd128`, so the compiler may use
  wasm's vector instructions, the web build ran 500 boxes only about 30% slower than native.
  Every browser that has WebGPU also has wasm SIMD, so the gameplay build turns it
  on. Size barely changes: 761 KB with it, 753 KB without.
- **Worst steps** come when a whole pile lands at once, and from V8 compiling on the first
  frames. Real scenes spread out their impacts. The first-frame cost can be paid behind the
  loading screen, by stepping an empty world.

### Budget

On the web, a laptop with integrated graphics has perhaps half the 9800X3D's single-thread
speed. To hold 60 fps there, gameplay gets 4 ms of each frame's CPU, and the renderer keeps the
rest. At 60 fps a frame runs one fixed step:

| Part | Integrated-GPU laptop, web | This machine, web |
|---|---|---|
| Physics, per step | 1.5 ms: about 300 moving boxes, any number asleep | 0.75 ms |
| Scripts, per step | 1.0 ms | 0.5 ms |
| Animation, per frame | 1.0 ms: perhaps 20 characters at full rate, an estimate until step 8 measures it | 0.5 ms |
| World, events, extract | 0.5 ms | 0.25 ms |

- **The per-step lines are paid per step.** A device that can't hold 60 fps runs two steps a
  frame at 30 fps, and so pays them twice. The budget is set so the integrated-GPU tier holds 60.
- **Boxes are a best case.** Contacts against the scene's triangle meshes cost more than box on
  box. Step 5 measures the cove's own collision.
- Every step that adds a system measures it against its line here, and adds it to the
  profiler's CPU sections.

### Colliding with the scene

- **The bake makes collision.** For each mesh it takes a coarse cut of the cluster DAG, at
  about 2–5 cm of error, as a triangle mesh. Small props get convex hulls instead. Terrain
  becomes a heightfield. So the shapes the player touches match what is drawn, without the full
  detail.
  - Shapes are made once per mesh and shared by every instance of it (Rapier's `SharedShape`).
  - They are stored in the file's data area, where streamed pages are, and listed by a small
    `COLL` section in the manifest. The manifest is read whole when a scene loads, so the shapes
    themselves stay out of it: they are fetched only when *Play* is pressed, and a scene that is
    only looked at never downloads them.
- **Moving actors** use primitives or convex hulls, from their `.actor` files.
- **The character controller** is Rapier's kinematic one: steps, slopes and snapping to the
  ground. It doesn't push dynamic bodies yet, and is stopped by them.
- **Determinism.** Rapier's `enhanced-determinism` feature is measured in step 5, and kept if
  its cost fits the budget, so that tests and replays give the same result natively and on the
  web.
- **Versions.** Rapier 0.36 uses `glam` 0.33 (through `glamx`). Solstice is on 0.30, and moves
  to 0.33 so the two share one copy.

**As built (step 5, `crates/solstice-physics`):**
- **The bake** makes collision only for a scene with gameplay; any other scene has none, and is
  written as before. Shapes are deflated into the data area, one by one, and listed in an
  optional `COLL` section. *Play* fetches them, so a scene that is only looked at never does.
  - A mesh is a cut of its cluster DAG at 5 cm of error, in world space: an instance scaled up
    gets a finer cut in its own space. A mesh with a bounding radius under 0.75 m gets a convex
    hull of the same cut, of at most 512 points, keeping those furthest out in every direction.
  - Each is built once for each size it is used at, and shared by those instances. They are
    two-sided, as scanned meshes are often open behind and mirrored instances are inside out.
  - Everything is checked before use, as a scene file comes from outside: sizes and offsets
    before anything is fetched, at most 1 GiB unpacked in all and 512 points to a hull, what
    unpacks no larger than it says, and every coordinate within a million metres, as physics
    panics on far larger ones. Sizes and positions in `.actor` files and `actor` and `blocker`
    lines have the same limit.
  - A line that places geometry can end with `collide=none`, `collide=hull`, or `collide=coarse`,
    a 10 cm cut for very large meshes such as ground or cliffs.
  - Terrain becomes a heightfield, sampled about every metre. Water doesn't collide.
  - An actor's mesh doesn't collide: its `collider` line does.
  - `blocker X Y Z SX SY SZ YAW` adds an invisible wall, or an invisible step.
    - A wall that something fast may hit wants depth behind its face. A car at 50 m/s goes
      0.8 m in a step: a wall 0.3 m thick stops it, but with its nose more than half-way
      through, and what is more than half-way through a box is pushed out of its far side.
      The circuit's barriers are 5 m deep for that (`apps/bake/src/circuit/paving.rs`).
- **Actors.** An `.actor` file's `collider box SX SY SZ` or `collider capsule RADIUS HEIGHT`
  gives it a shape, kinematic by default, so it goes where its script puts it and pushes what
  is in the way.
  - `at X Y Z` moves the shape off the actor's origin: a door hinged at its origin has its box
    out along the door.
  - `sensor` blocks nothing, and calls the script's `on_enter(self, who)` and
    `on_exit(self, who)`, which also comes if what was inside is removed.
  - `dynamic MASS` is moved by physics.
  - `mesh PATH SCALE part=NAME` draws one named part of a model, by its own origin.
- **Characters.** `character RADIUS HEIGHT` makes an actor walk, with Rapier's kinematic
  controller: steps up to 0.35 m, slopes up to 45°, snapping to the ground, falling. Its
  `Transform` is at its feet. The script says where it wants to go, `self.walk = vec3(...)`, in
  metres a second across the ground, and physics does the walking. A script that sets
  `self.position` moves it there at once.
  Stairs are tested up to a 0.3 m rise. Taller steps are climbed more slowly.
- **Order in a step.** Scripts in `PrePhysics`, then physics: kinematic actors move to where
  scripts put them, the simulation steps, and characters walk. Sensors' events reach scripts
  in the next step's `PrePhysics`.
- **Seeing the colliders.** `bake --scene SCENE --show-collision` bakes what a scene collides
  with instead of how it looks, to view in either viewer: blue triangle meshes, green hull
  points, grey ground, red blockers. It needs no change to the renderer.
- **Determinism.** Rapier's `enhanced-determinism` is on. It costs about 3% on a thousand
  moving boxes, and 20% of a very small number for walking. A run of `tools/gameplay-bench/b_walk`,
  of walkers, doors and a pile of falling crates, ends with the same bits natively and on the
  web. Actors are kept in a fixed order, so that adding and removing them repeats exactly too.
- **The gameplay web build** turns on wasm SIMD. It has a target folder of its own, as different
  compiler flags can't share compiled crates.
- **`glam` moved to 0.33.** It compiled unchanged, but deprecates the camera matrices the
  renderer uses (`look_to_rh` and friends). `solstice-render` allows those for now, with one line
  in its `lib.rs`; renaming them is left for a quiet moment in those passes.
- **Cost** (`tools/gameplay-bench/b_walk`, 9800X3D, a scene of 330,000 triangles and a
  heightfield):
  - the player and 8 doors: 0.05 ms a step on the web, 0.03 natively, against a budget of 0.75 ms
    on this machine;
  - 8 walkers: 0.25 ms a step on the web, 0.2 natively.
  - The cove's collision is 453,000 triangles and 6.5 MB, fetched on *Play*. Building the
    world takes 0.7 s on the web, once, when the game starts.
  - The gameplay web build grows from 2.15 MB to 3.0 MB gzipped, for Rapier. The plain viewer
    grows by 1.2 KB, for reading the section.
- **The demo** runs headless, with `viewer --play --capture --hold KEY@FROM-TO`: in the cove,
  the player is stopped by the gate, opens it and walks through into the courtyard, climbs the
  stair, and reaches the rampart, 7 m up, where the wall stops them. The stair's bottom step
  is 0.5 m high, too high to step, so the scene adds an invisible half-step with a `blocker`.

### Floating on the sea

A ship that floats must ride the waves that are drawn. The waves exist only on the GPU, but
`spectrum.rs` already reproduces their starting amplitudes exactly on the CPU, seeds and all
(`initial_amplitude`, checked against the GPU in `waves.rs`). So the sea gets a CPU height
query:
- **The waves:** a direct sum over the few hundred most energetic waves, advanced to the step's
  time. Cascade 0 holds only waves longer than about 21 m, which is enough for the ship, but a
  rowing boat also rides the shorter waves of cascade 1, so the query takes both.
- **Sideways motion:** the waves move the water sideways as well as up, to sharpen their
  crests. The height at a point is found by a few iterations that find which undisplaced point
  moved there.
- **The shallows:** heights shrink by a Rust twin of `shoal()`, tested against the WGSL one, with
  the depth from a small CPU copy of the seabed map.
- **Timing:** the query follows the same eased sea state the GPU draws, advanced by the fixed
  steps' clock, so it stays deterministic.

Buoyancy samples the query at a handful of points on each hull, and pushes each one up by the
water it displaces, as Sea of Thieves does. It has no delay. Reading heights back from the GPU
was the other option; it would arrive frames late and make the physics depend on the GPU's
timing. The query's cost counts in the physics line of the budget, and step 11 measures it.

## Rigging and animation

### Content

Chosen on 8 November 2026: a MakeHuman character, animated with Quaternius's Universal
Animation Library.

- **The character** comes from MakeHuman's Blender plugin, MPFB 2. What it makes is CC0, and
  so are its bundled assets: bodies, skins, hair and clothes. The plugin's own code is GPL, but
  none of it ends up in Solstice. MakeHuman makes realistic people, so the character suits the
  photoscanned cove better than a stylised one would.
  - The character is made by a Blender Python script kept in the repository, written like every
    other script here. Its body, clothes, rig and export settings are all in that script, so it
    can be remade or changed by editing it, the way scenes are made from `scenes/*.txt`.
  - It is exported by Solstice's own export tool (below), with MPFB 2's game-engine rig.
  - Blender 5.2.1 LTS is installed at `D:\Blender`, and the script runs it headless
    (`blender --background --python`). MPFB 2 is not installed yet. Installing it from Blender's
    extensions site waits for your go-ahead when step 6 begins, and that step first checks that
    it supports Blender 5.2.
- **The animations** come from the Universal Animation Library, volumes 1 and 2: over 250 CC0
  clips in glTF on one humanoid rig, with walking in eight directions, jogging, sprinting,
  sitting, climbing, swimming and more. The free tier is enough. The *Source* tier, with its
  Blender files, is not needed.
- **Retargeting.** The two rigs differ: they name, count and orient their bones differently,
  and their proportions differ. So the bake retargets each clip onto the character, as Unreal's
  IK Retargeter does:
  - each rig gets a small text file naming the bone that plays each role: pelvis, spine chain,
    neck, head, and each limb's chain;
  - rotations are carried across in each bone's rest frame, so a difference in rest pose cancels
    out;
  - the pelvis's movement is scaled by the ratio of leg lengths, so feet don't slide;
  - IK cleans up the hands and feet afterwards (step 9).

  Doing it in the bake, in Rust, keeps it reproducible and tested. Changing the character later
  means changing its role file, and no clip needs touching.
- **Licences** are recorded with each asset, as for the Poly Haven scans.

### The export tool

MakeHuman's glTF export has known rough edges, as its users report. Clothing's helper vertex
groups come out unless modifiers are applied, and materials come out grey when Blender's shader
nodes are too complex for its exporter. So Solstice gets its own export tool: a Blender Python
script in the repository (`tools/blender/solstice_export.py`), run headless. It doesn't write
glTF itself. It prepares the character, then hands it to Blender's own glTF writer with one fixed
set of options, then checks the result with Solstice's importer.

1. **Hidden geometry.** MPFB 2 hides the body under clothes with helper modifiers and vertex
   groups. The tool applies them for real, keeping the armature live, deletes the hidden faces,
   and drops every vertex group that isn't a bone.
2. **Materials.** Each material is baked, with Cycles, down to the textures the importer reads:
   base colour (with cavity), normal, and roughness and metallic. Every material then comes out
   right, whatever its node graph.
3. **Weights.** At most four bones per vertex, renormalised; tiny weights dropped; bones that
   deform nothing removed, except those the retargeting role file names.
4. **Space.** Metres, Y up, transforms and rest pose applied, so the import needs no
   corrections.
5. **Export** with fixed options, so every export of the same script comes out the same.
6. **Checks,** by reading the file back with Solstice's importer. The tool fails loudly unless:
   - no vertex group is left that isn't a bone;
   - every material has its textures;
   - every vertex has at most four weights, summing to one;
   - every bone the role file names is present.

It is a separate tool, so the engine only ever sees clean glTF, and it works for any rigged
character, not just MakeHuman's. Its first job, before any of it is written, is to export a
default MakeHuman character as it stands and see what actually goes wrong, since the reports
above are secondhand.

**What went wrong, first hand** (step 6). A default dressed character, with the game-engine
rig, exported with Blender 5.2's glTF writer as it comes:
- **The body's helper geometry was exported.** MakeHuman builds the body with helpers: a skirt
  and tights for fitting clothes, a stand-in for each eye, and joint markers. A mask modifier
  hides them, and the writer doesn't apply modifiers unless asked. Seen in the engine, the man
  wore a long skin-coloured dress and a strip over his face.
- **The body under the clothes poked through them.** The clothes hide it with more masks.
- **Eight of MakeHuman's own shape keys came out as morph targets,** named like `$md-$ca-$ma-$yn`.
  They are its mix of body shapes, which belongs baked into the mesh.
- **Every material was blended,** even the skin. Most had only their colour texture: no normal
  map, and no roughness, so the skin was flat. Clothes kept their colour and normal maps.
- **Some vertices had more than four weights.** The writer kept the four strongest and
  renormalised them, which is fine.
- **Space and scale were right:** metres, Y up, 1.67 m tall, 53 bones.

The tool fixes all of these. Reading the default export back, the checks find the morph
targets, the blended materials and the missing textures. The masked helper geometry can't be
seen in a glTF, and the writer had already cut the weights to four, so the tool checks those in
Blender, before exporting.

**As built (step 6):**
- **`characters/keeper.py`** makes the fort's keeper with MPFB 2: the body's sliders, the
  game-engine rig, a skin, eyes, brows, lashes, teeth, tongue, hair, a work suit and shoes, all
  from MakeHuman's CC0 system assets. Make and export it with
  `python tools/blender/make_character.py characters/keeper.py target/characters/keeper.glb`.
  The output stays out of the repository, as the scanned models do.
  - The asset pack is kept at `D:\SolsticeAssets\makehuman`, with where it came from and its
    checksum, since MakeHuman's community services are shutting down.
  - MPFB needs Blender's user preferences, so the character is made without
    `--factory-startup`. The export runs with it.
- **`tools/blender/solstice_export.py`** works as planned above, with these details:
  - It runs in this order: hidden geometry, weights, space, then materials, so the bake sees
    the rest pose.
  - Masks are carried out by deleting what they hide, vertex by vertex, rather than by
    applying the modifiers. They sit after the armature modifier, which must stay live, and
    Blender warns that applying a modifier that isn't first may not give what is expected.
    Disabled modifiers are dropped, and a mask that cuts smoothly is refused.
  - Only groups of bones that deform are kept as weights. A vertex whose weights are all tiny
    keeps its strongest, and locked groups are unlocked so they are tidied too.
  - Each material is baked at 1024 by 1024: base colour, tangent normals, roughness, and
    metallic. No cavity is baked yet.
    - Where the surface is a Principled BSDF, its base colour, alpha and metallic are baked
      exactly, through emission. Cycles' diffuse colour pass would darken them by alpha and
      turn metal black.
    - Other surfaces, such as MakeHuman's skin node groups, go through the diffuse colour pass.
  - The bake uses the UV map the glTF writer writes, the render one.
  - A material is cut out (alpha mode MASK) only if over 0.1% of the texels it covers,
    counted without the bake's margin, are see-through. MakeHuman wires alpha into every
    material, so otherwise the suit, the eyes and the teeth would all be alpha tested. On the
    keeper, only the brows, lashes and hair are. A material see-through all over, by a constant
    alpha, isn't supported; it comes out opaque.
  - Cut-out materials are two-sided, as hair cards need; the rest are one-sided.
  - Materials are baked once each:
    - a material in two slots of a mesh is baked once;
    - one shared between meshes is copied for each mesh before anything is baked, so each
      bakes into its own UVs;
    - a bake writes every material of a mesh at once, so the materials it isn't for write into
      a scratch image.
  - Bones removed because they deform nothing take their vertex groups with them.
  - The glTF writer's options are pinned, so a change of its defaults can't change the file.
  - It writes the file only once it has passed its checks. One that fails is left as
    `NAME.failed.glb`, and any error makes the tool fail. Blender itself must be run with
    `--python-exit-code 1`, or it exits as if all were well; `make_character.py` does that.
  - `--as-is` exports with Blender's own defaults, to show what the tool is for. Its checks
    fail.
  - It doesn't take meshes carried by a bone without an armature modifier, such as some rigs'
    eyes or props.
- **The checks** are in `solstice-import` (`character.rs`), run as
  `bake --check-character FILE.glb --roles RIG.roles`. Problems are listed, and the tool fails
  if there are any.
  - Besides those planned, they check what skinning will need in step 7:
    - an inverse bind matrix for each bone, undoing where it stands, which shows the file is in
      its rest pose;
    - the bones form one tree, of at most 256;
    - triangles only, with texture coordinates, which every texture reads through;
    - no negative weights, or weights that aren't numbers.
  - A leaf bone that deforms nothing must go. An inner one stays, since others hang from it.
    So does one the role file names.
  - `characters/rigs/game_engine.roles` names a role for each of the rig's 53 bones.
- **Tests.**
  - The checker's unit tests write small skinned glTFs by hand, and run in CI.
  - `tools/blender/tests/run_tests.py` runs Blender, so it runs locally, not in CI. It builds
    a small character with every fault built in (`faulty_character.py`), then reads the
    tool's file back and checks its values:
    - the faults: a mask; groups that aren't deforming bones; a shape key; five weights a
      vertex, two tiny; idle bones;
    - the materials: a blended material see-through over part of it, in two slots and shared
      with a second mesh; a metal whose metallic comes through a node; and one whose alpha is
      wired in but solid.
    - Exported as it is, the character fails the checks.
  - It then makes and exports the keeper twice from scratch. The two files are the same to the
    byte on this machine, and only the brows, lashes and hair are cut out. Exported as it is,
    the keeper fails the checks too.
  - The asset pack is read from MPFB's own library, where it was installed from the copy on
    `D:`.
- **Cost.**
  - Making and exporting the keeper takes about 16 seconds.
  - The file is 11.8 MB: 38,462 triangles, 22,088 vertices, 12 materials, 53 bones.
  - The default export was 25.8 MB and 68,608 triangles. Most of the difference is the hidden
    geometry gone, and the textures baked down to 1024.
- **Cut-outs.** The keeper's hair, brows and lashes are drawn with their holes, as anything
  authored `alphaMode: MASK` is: see `docs/cutouts.md`.

### Skinning in virtual geometry

Skinned meshes go through the same pipeline as everything else: the cluster DAG, streaming, both
rasterizers, the visibility buffer and resolve. They do not get a separate path.

- **Weights.** Each vertex gets up to four bone influences, as 8-bit bone indices into its
  cluster's own list of bones and 8-bit weights: 8 more bytes per vertex. They are stored only
  for skinned meshes, in their own part of each page. The vertex's spare 16 bits stay free.
- **Building the DAG** in bind pose:
  - clusters are grouped by the bones they use as well as by position, so each cluster's list
    stays short, mostly 8 bones or fewer, with a hard cap of 16. Coarse levels may want more,
    such as a hand with all its fingers. There, the smallest influences are dropped and the rest
    renormalised until the cluster fits, and that change counts in its simplification error;
  - simplification keeps the weights: collapsing an edge blends them, and the quadric error
    counts a change of weights, so joints aren't simplified away;
  - borders between clusters stay locked, as now, so a shared vertex keeps exactly the same
    weights on both sides.
  - Equal weights are not quite enough for no cracks: the skinned position must come out the
    same to the last bit. Two clusters can list the same bones in different orders, so each
    vertex's influences are stored sorted by the skeleton's own bone numbers, and summed in that
    order everywhere.
- **Culling.** Each cluster stores its bounding sphere in each of its bones' bind space. Each
  frame, its bounds are the union of those spheres, moved by the current bones, which is cheap
  for a list of 8. Hierarchy nodes get posed bounds the same way.
- **LOD.** LOD error is measured in bind pose and scaled by the largest bone scale, with a
  margin for bending. The cut stays free of holes and overlaps only if a group's clusters and
  the clusters made from them decide from the same sphere. So each group's LOD sphere is posed
  once, by one function over one set of bones stored with the group, and both sides of the
  decision read that result.
- **Rasterizing.** Both rasterizers, resolve and the shadow pass call one shared function,
  `skinned_position`. It applies linear blend skinning from a buffer of bone matrices: this
  frame's, and last frame's for motion vectors. Normals are skinned with the same matrices.
  - Vertices are skinned where they are used, as Unreal does: there is no pre-skinned copy, so
    no extra memory, and it works whatever the LOD cut and whatever has streamed in.
  - Resolve skins each pixel's three vertices, with four bones each, for this frame and the
    last: a real cost on integrated GPUs.
  - If the cost proves too high, the fallback is a compute pass that skins only the visible
    clusters into a scratch buffer each frame. Step 7 measures both.
  - Dual-quaternion skinning, for shoulders and wrists that would otherwise pinch, is an option
    to judge on real characters.
- **Moving things in the rest of the renderer** (step 1, for every moving instance, skinned or
  not):
  - *Marking them.* Each instance gets a "moving" flag. Culling for the cached views (the shadow
    cascades' cache, the seabed map and GI voxelisation) skips flagged instances, so none of them
    keeps a ghost of something at its old place.
  - *Shadows.* The cached cascades keep only static geometry. Moving instances are drawn each
    frame into a map of their own for each cascade they touch, and shading multiplies the light
    the two let through. Nothing is copied: copying all four 2048² cascades each frame would move
    about 128 MB, too much for integrated GPUs.
    - As built (step 1): the four moving maps share one more layer of the cascades' array, a
      quarter each, at half their resolution. The shaders that sample shadows already use
      WebGPU's limit of 16 textures, so a new texture was not an option, and one layer costs
      16 MiB rather than four layers' 64 MiB.
    - A cascade with nothing movable in it draws nothing more and is sampled once, as before.
    - The layer costs 16 MiB even in a scene with nothing movable: an array's layers can't be
      added later without making it again.
    - Cost: 0.45 ms on the RTX 5080 with the cove's ship moving in all four cascades, nearly all
      of it one culling pass per cascade. Culling once for all four is left to the stress-test
      work.
  - *GI voxels.* Moving things are left out at first. They receive GI and appear in screen-space
    GI, but don't occlude or bounce light in the voxels.
  - *Wetness.* The water-line map draws no geometry, and needs nothing. But its wetness belongs
    to places, so a moving hull would wrongly lose its wet band, and carry none. Moving things
    get their own wetness later, with the rain work.

**As built (step 7, the format and the DAG).** The plan above is simplified in three ways, each
checked on the keeper (38,462 triangles, 628 clusters in 12 meshes):

- **A vertex names its bones directly,** by the skeleton's own numbers, rather than through its
  cluster's list. Skeletons have at most 256 bones, so 8 bits is enough, and resolve and the
  rasterizers skip a lookup per influence. A skin is 8 bytes behind its cluster's vertices in the
  page, end to end (two to a 16-byte record until file version 5), so a page needs no new part.
- **The cluster's list is only for culling,** and a cluster's bounds stay its bind-pose sphere: the
  skinned position lies inside the sphere moved by each of its bones, so a union of those moved
  spheres contains it. No per-bone radii are stored.
- **Clusters are not grouped by bones.** On the keeper 626 of 628 clusters use 16 bones or fewer,
  most under 8. The two that don't are coarse ones around the hands, drawn with the whole mesh's
  bounds rather than losing influences, so no weights are dropped.

Simplification keeps every vertex's weights, since a collapse only removes vertices. Moving a
vertex's weights counts in the error: 2.5% of the mesh's size for a whole change. A skin is welded
as part of a position, so two vertices differing in skin stay apart in a pose. Thinned pieces such
as hair cards keep their source vertices' skins. Scene files gain an optional `SKIN` section, so
older files and readers are unaffected. The importer keeps a skin only when asked
(`ImportOptions::skinned`); otherwise a skinned model is still imported unposed, as before.

**As built (step 7, the renderer).** `Renderer::set_pose(pose, bones)` sits beside
`move_instance`, and `place_pose` beside `place_instance`. Once a frame, before culling, the rows
of the bone texture that changed are uploaded.

- **Bones and lists are in textures, not storage buffers.** Culling, rasterizing and resolve are
  each at WebGPU's limit of eight storage buffers per stage. The bone texture is `rgba32float`, 1024
  texels wide, this frame's matrices in its top half and last frame's in its bottom half, for motion
  vectors. The list texture is `rgba32uint`, one texel a skinned cluster.
- **Every path that places a vertex goes through `skin_rows`** (`common/skin.wgsl`): both
  rasterizers, opaque and cut-out, the shadow passes, and resolve. So a shared vertex lands on the
  same bit wherever it is drawn. A scene with nothing skinned has that compiled out of its shaders
  (`shaders::module_for`), since even the test of `Instance.bones` showed in resolve; in a scene
  with characters, rigid instances pay for that one test.
- **A cluster's bounds in a pose** are the union of its bind sphere moved by each of its bones. The
  two kinds of cluster that can't use that are the ones with too many bones to list, and nodes of the
  hierarchy, which have no lists at all. Those use the whole instance's reach: how far the pose
  carries anything (`Instance.lod_margin`, in the mesh's units, worked out on the CPU from each
  bone's own sphere). A bone's sphere is measured around its source vertices, so vertices that
  thinning has moved out, as on hair cards, can lie slightly beyond it; only those two kinds of
  sphere could be short, by at most about twice the cluster's LOD error, a pixel or two on screen.
- **LOD spheres grow by that same margin, for the whole instance,** so that both sides of a LOD
  choice see the same sphere and the cut stays watertight in any pose.
- A skinned instance is always movable: drawn afresh each frame, in no cached view, and left out of
  the GI voxels as other moving things are. Asking for it to go static or stay in the voxels is
  refused.
- The first pose given to a character, and a `place_pose`, show no motion. A pose that wasn't
  updated last frame settles, so a character that stops stops showing motion. Distant characters,
  posed every few frames, show motion only on the frames they are posed.

### The animation runtime

`solstice-anim`, on the CPU, in Rust:

- **Skeletons and clips.** Clips are resampled to 30 Hz and quantised, as in the Animation
  Compression Library: rotations as their three smallest components, translations within each
  track's range. Bones are sampled in batches of four (structure-of-arrays) for SIMD.
- **Blending:**
  - linear and additive blends;
  - per-bone masks, so the upper body can aim while the legs run;
  - 1D and 2D blend spaces, by speed and direction;
  - state machines with cross-fades, written in a small text format like the others;
  - sync groups, so blended walk and run cycles keep their feet in step.
- **Root motion,** and **notifies**: events at set times in a clip, such as a footstep or a
  sword's hit frame, delivered to scripts like any other event.
- **IK:**
  - two-bone analytic IK, with a pole vector, for legs and arms;
  - foot placement, with a raycast down from each foot and the pelvis lowered to suit;
  - look-at and aim, with limits;
  - FABRIK for longer chains, such as tails and ropes.
- **Secondary motion:** spring chains (Verlet), with sphere and capsule colliders, for hair,
  straps and ropes.
- **Animation LOD:** distant or small characters update less often, as Unreal's budget allocator
  does, within the 1 ms budget.
- **Ragdolls,** from Rapier joints, blended in and out of animation, come in a later step.

**As built (step 8, the runtime).** `solstice-anim` holds clips, poses, blends, blend spaces and
state machines. The rest of step 8 arrives with the character playing them. So far:

- Clips are quantised as planned: a rotation in 48 bits (three smallest components) and a
  translation in its track's range, with one sample kept for what doesn't move. A scene carries
  them in an optional `CLIP` section. Sampling isn't batched for SIMD: it wasn't needed, as the
  cost below shows.
- Blends, per-bone masks and additive poses work on whole poses. A state plays a clip or a 1D or
  2D blend space. Every clip in a blend shares one phase, weighted by its share of the blend, so
  blended walk and run cycles keep their feet in step.
- `.anim` files list where the clips come from, the parameters, the states and the transitions.
  Transitions cross-fade between states, and notifies fire as their time is crossed, from the
  clip with the largest share of a blend.
- `solstice_import::animation` reads a glTF's animations at 30 Hz. Bones that match by name need
  nothing more; different rigs go through a role file on each side, with rotations carried in
  each bone's rest frame and the pelvis's travel scaled by leg length.
- The first clips came from `tools/blender/make_clips.py`, which writes idle, walk and run for the keeper's
  rig as hand-keyed motion, in place of the Universal Animation Library until that is
  downloaded: 102 KB for all three.
- Cost, on a Ryzen 7 9800X3D: 20 characters of 53 bones, each blending two clips in which every
  bone moves every frame, take 0.06 ms a frame natively and 0.10 ms on the web, in Node. 100
  characters take 0.3 and 0.4 ms. So the 1.0 ms budget line holds easily.

**As built (step 8, actors).** An actor's `.actor` file can name a state machine with an
`animation` line, which makes the bake keep its mesh's skeleton and read the clips its `.anim`
file names. `yaw=180` on a `mesh` line turns a model made facing +Z, as MakeHuman's are, to face
the way actors do.

- A graph with a `speed` parameter is driven by how fast the actor walks, so idling and walking
  follow the character without a script. `anim_set(param, value)` and `anim_play(state, fade)`
  let a script do the rest, and clip notifies reach it as `on_anim(self, name)`.
- Animations move on in fixed steps, so transitions and notifies replay exactly at any frame
  rate; only working out what to draw follows the frame rate, and is smoothed between steps.
- Animated characters update every frame within 15 m of the camera, every second frame to 40 m,
  every fourth to 90 m, and every eighth beyond that.
- `scenes/walker.txt` is the smallest scene with an animated actor. It needs
  `tools/blender/make_character.py` and `python -I scripts/fetch-ual.py --zips FOLDER` run first. Play it with
  `viewer target/walker.solstice --play`, or `?scene=walker&play` on the web page.

**As built (the Universal Animation Library).** The clips are Quaternius's Universal Animation
Library, volumes 1 and 2 (CC0). They stay out of git: `scripts/fetch-ual.py` checks the two
Standard zips against their digests and unpacks the clip files to `target/ual`. They have to be
downloaded by hand from their name-your-own-price pages, which the script lists.

- `characters/rigs/ual.roles` names the library's bones. Clips carry only by those roles, so the
  library's mesh and root-motion variants aren't needed.
- **Rest poses are lined up first.** The library rests in a T, and the keeper with its arms down
  and elbows bent, 50 degrees apart, with its legs leaning slightly out. Before a turn is carried
  across, each limb is turned to lie along the source's, each bone from where its parent's lining-up
  left it. The hand is lined up by its palm, with the forearm sharing its roll, and the fingers go
  with it. The legs too, and the pelvis rises by what that lowered the feet. The collarbone isn't
  lined up, since that would drag the shoulder away from where the mesh has it, and nor is the
  spine, which is close enough. A role chain's bones are paired by where they lie along it, so an
  arm with fewer bones than the source's is lined up less exactly.
- **Speeds.** The library's root-motion files move 0.975 m/s for `Walk_Loop` and 5.36 m/s for
  `Jog_Fwd_Loop`. Scaled by the keeper's ratio of leg lengths, 1.084, that is 1.06 and 5.8. What
  matters to the eye is a planted foot, which in the retargeted clips goes back at about 1.04 m/s
  in the walk and 6.2 m/s in the jog. So the blend points in `scenes/walker/keeper.anim` are 1.0,
  which slips 3 cm over a stance, and 6.2. A walker going much faster than 1.0 m/s blends towards a
  jog, and its feet slide; give it `speed` 1.0.
- Feet slide about 8 cm over a stance, measured on the keeper. Step 9's IK removes the rest.
- The library's `Idle_Loop` stands with its feet 45 cm apart, as it was authored.

## Gameplay and the world around it

The renderer already eases the weather and the ocean. Gameplay gets them, with the time of day
and the sea level, as one `Environment` in the world, and sets targets, not values each frame.
The time of day and the sea level take effect at once in the renderer, so the `Environment`
eases those itself, on the fixed steps' clock: a tide is a sea level eased over hours.

```python
def on_begin(self):
    env.time.day_length = 1200          # a 20-minute day
    env.weather.set("fair", over=0)

def on_storm_bell(self):
    env.weather.set("storm", over=90)   # the renderer eases it in
    env.sea.set("ocean", over=180)      # and the sea builds more slowly

def on_hour(self, hour):
    if hour == 19:
        for lamp in find("lantern"):
            lamp.light.on = True
```

- **Reading:** `env.sun_direction`, `env.is_night`, `env.wind`, `env.weather.rain`,
  `env.sea.height_at(x, z)` (the CPU query above), and later how wet a surface is.
- **Events:** `on_hour`, `on_weather_changed`, `on_sunset`, `on_sunrise`.
- **New renderer APIs** that gameplay needs, all general:
  - per instance: moving it, hiding it, and scaling its glow, for opening gates and lit
    windows;
  - lights switched on and off, and moved. GI's lighting is updated every few frames
    (`LIGHT_SCHEDULE` in `gi/volume.rs`), so it follows a changed light.
- **A moving sun breaks the shadow cache.** A cascade is redrawn whenever its view changes, and
  its view includes the sun's direction. The 20-minute day above turns the sun about 0.3° a
  second, so every cascade would be stale every frame, and the most expensive pass on small GPUs
  would run all the time. So the sun's direction for shadows moves in small steps, a step every
  second or so, and the cascades catch up over the frames after it, far ones taking turns, as
  they already do. Step 10 measures how small a step looks smooth.
- **Who is in charge.** While a game runs, it owns the environment. The web page's time and
  weather controls become debug overrides, marked as such, and hand control back when released.
  A scene file can still set the starting values.

## Cars and circuits

Built for a racing game on the engine,
[solstice-racing](https://github.com/SamG-Coder/solstice-racing), and all of it as built.

- **A car is an actor.** `vehicle FILE [manual]` in an `.actor` file gives the actor a car's
  dynamics in place of a collider's. The pieces of its mesh whose origins are at its hubs are
  its wheels: they roll, steer and ride the suspension. A piece within 60 mm of a hub but not
  at it, such as a brake caliper, goes with the wheel without turning.
- **The vehicle model** (`crates/solstice-vehicle`) knows nothing of the engine: a body, four
  wheels, and a `Ground` to cast rays at.
  - Tyres: Pacejka's formula along and across, combined by similarity, with load sensitivity
    and a relaxation length.
  - Suspension: a spring, a damper, an anti-roll bar and bump stops at each wheel.
  - The engine, clutch, gearbox, differential and brakes are solved together, implicitly, ten
    times a step, so a locked wheel or a slipping clutch is stable.
  - Drag and downforce.
  - Anti-lock brakes, traction control and stability control, each a line in the car's file.
    Stability control watches the rear tyres' slip angle: past the angle they grip best at,
    it eases the engine, lets go of the rear brakes and brakes the outside front wheel.
  - A `.vehicle` file holds a car's numbers, each given once. `tests/gt.vehicle` is a
    rear-engined GT racing car, and its tests hold it to what its class does
    (`tests/rear_engined.rs`).
- **The driver** (`driver.rs`) is between the player and the car, as in every driving game.
  A key is a wheel hard over; a wheel hard over at speed is a spin. So steering eases, a full
  push gives less lock the faster the car goes, and past a few degrees of slide the wheel
  turns towards the way the car is going, as a real wheel pulls a driver's hands round.
  Pedals ease too, and the gearbox changes gear by itself unless the actor says `manual`.
  - `tests/keys.rs` drives the car as keys do: hard over and flat out, flicked one way and
    the other, on the brakes, with two wheels on the grass. Nothing there slides past 12
    degrees. Before stability control and the wheel that follows a slide, a flick left and
    right spun it at every speed.
- **A circuit** is a `.track` file, named by a scene's `track FILE`: its centre line as points,
  each with the way across the road, the road's width, and what lies either side of it
  (`kerb`, `grass`, `gravel`, `concrete`). From it each tyre knows what it is on, laps are
  timed with sectors, a lap that leaves the road does not count, and `reset` puts a car back.
  A car that falls far below its track is put back without asking.
- **Land that is no circuit** has a map of its ground instead, named by a scene's
  `surfaces FILE` (`solstice_game::SurfaceMap`, `crates/solstice-game/src/ground.rs`): a grid
  of squares, each of one surface, which each tyre reads the square it is on of. Besides a
  circuit's four there are `dirt`, `mud`, `bog`, `litter`, `rock`, `ford` and `sand`, each
  with how far a tyre sinks into it and what rain does to it. Where a scene has both, the
  map is what the tyres read. Soft ground keeps the ruts driven into it (docs/ruts.md).
- **The camera** follows the player's car. `camera` steps through four views behind it, each
  further out, and two on it. `look_round` and `look_over`, the right stick, swing it round
  the car and tip it over; `look_back` is the mirror.
- **Input actions** for all this are among the built-in ones in a scene with a car among its
  actors; a scene without one leaves them out, so its keys press nothing more. They are
  `throttle`, `brake`, `steer`, `handbrake`, `shift_up`, `shift_down`, `reset`, `camera`,
  `look_back`, `look_round`, `look_over`, and `autopilot`, which hands the car to a driver of its own that follows the
  track's centre line at a speed the bends ahead allow, changing gear itself even for a
  `manual` car (the player has their own gearbox back when they take over). That is for showing a circuit off,
  and for trying one headless: `viewer SCENE --play --press autopilot@40 --frames 5000`.
- **Building one.** `circuit SIZE` in a scene file builds a whole circuit from the scene's
  track (`apps/bake/src/circuit`): an island whose height follows the road, with cliffs, the
  road with its racing line of rubber, lines, kerbs, gravel and barriers, grass and woods.
  The ground a car drives on is the same land.
  - What is laid along the road is cut into meshes 96 m square. As one mesh the size of the
    circuit its points would be kept to 13 mm, more than paint stands above a road
    (`Mesh::grid_step`).
  - Beyond its edge the ground is level with the road's edge, even where the road is
    banked. Carried on outwards, a banked corner's tilt made a ramp of its gravel trap.
- **Viewers.** Natively the window's title is the dashboard. The browser build gives the
  page the same figures (`hud`), and the page draws them.
- **Left for later.** Sound; other cars; a dashboard drawn in the native window; glass that
  is seen through, which wants blended materials.

## Viewers

- **Native:** `viewer scenes/cove.solstice --play` runs the scene's game, if it has one, and
  watches its scripts for changes.
- **Web:** the page offers *Play* on scenes with gameplay, and loads the gameplay build. It
  captures the pointer and the keyboard, and shows script errors in a small overlay.
- **Input actions** are a text file mapping actions to keys, mouse, gamepad and touch, so the
  same game works on a phone.

**As built (step 4, `crates/solstice-game`):**
- **Actor files** are as above, with these differences from the plan:
  - `mesh` names a glTF file, relative to the actor file, and a scale. The bake places it
    wherever the scene places the actor, as one or more instances, and they move with it.
  - `camera X Y Z` makes an actor the player's, with its eye there.
  - `collider` comes with physics, in step 5.
  - Properties are `bool`, `int`, `float`, `text` (one word) and `vec3`. A scene line sets a
    vector as `x,y,z`.
- **In the scene file.** The bake carries a scene's gameplay in an optional `GAME` section. It
  holds named text files: each actor file and script, the input actions, and `scene.game`, the
  scene's `actor` lines with the instances each actor's mesh became. The section knows nothing
  of what is in it. A scene without actors has no section, so it is written byte for byte as
  before. The bake checks every actor, setting and script, and stops at the first mistake, with
  its line.
- **What several scenes share.** A game's maps have things in common that are no part of any
  of them: its truck, what its driver carries. Baked into each, they are in every map's file.
  - `shared` at the end of an `actor` line says the actor's model is not in this scene's file
    but in one beside it, `shared.solstice`, baked from a scene of its own that has an actor
    of the same `.actor` file. The actor itself, its script, physics and place, is still the
    scene's.
  - Whatever reads the two joins them into one scene (`solstice_asset::join::join`): the
    shared file's tables go after the scene's, renumbered, its pages are dealt in among the
    scene's from coarse to fine, and its bytes are numbered on from wherever the reader says
    (`Joined`), for a scene read from its file as it is drawn. The viewer does so
    (`apps/viewer/src/shared.rs`, `read_source` in `main.rs`): the scene's actor is given
    the instances that draw it, moved to where the scene puts it. A scene read from its file
    as it is drawn (`--stream`) reads the shared file the same way, its bytes numbered on
    from the end of the scene's. The viewer does not yet do so in a browser.
  - A joined scene is drawn as any other, and the first scene's own numbers do not change.
    What is refused: a scene split up for a browser, textures of different sizes, and
    collision or glowing cards in the shared file.
  - **Lending.** Maps have more in common than their truck: the same trees, grass and
    stone, the same textures under the ground. A map's own instances are of these, so they
    cannot simply be the shared scene's. Instead the shared scene lends
    (`crates/solstice-bake/src/lend.rs`):
    - `lends` in a scene's text makes it carry a list of every mesh and texture it has, each
      with a key, a hash of what it is (the gameplay file `shared.lends`).
    - A scene with a `shared` actor, baked beside that file, leaves out of its own whatever
      has a key on the list: the mesh becomes an empty place that keeps its number, the
      texture is dropped, and the scene carries a list of what it left (`shared.borrows`).
      Nothing says which things are shared. What both have bit for bit is left out, and what
      differs by anything is kept, so a map is never given the wrong tree.
    - As the two are joined the places are filled (`solstice_asset::join::fill`): each empty
      mesh is the lender's, each such material has the lender's layer. The map's instances
      and materials keep their numbers.
    - So the shared file is baked first, and a map baked beside one shared file is read with
      that one: if the stock has changed since, the join says so, and both are baked again.
  - With `SOLSTICE_WHOLE` set, the bake takes no notice of `shared`, leaves nothing out, and
    the scene has everything in it: to look at a map on its own, or to tell a fault in the
    joining from one in the map.
- **Input** (`.input` files, or built-in actions). An action's value is the sum of its
  controls, times their scales.
  - Buttons and sticks add up to between -1 and 1.
  - Movement (the mouse, and dragging on the screen) is in pixels a second. It builds up until
    a step uses it, over the time since the step before. So `self.yaw += input("turn") * dt`
    turns as far at 144 frames a second, where most frames run no step, as at 60. Looking up
    and down follows it every frame.
  - Sticks have a dead zone, and press their action pushed either way.
  - A tap presses its action without holding it.
- **Play mode** (`Play`) runs the frame, then says which instances moved and where the
  player's eye is.
  - Each actor is drawn between its last two steps, and only when it moved.
  - Looking up and down (`look_up`) moves the camera alone, per frame. Turning is the
    player's own yaw, which its script sets.
- **Viewers.**
  - Natively, `viewer SCENE --play` plays the scene, with keys and the mouse. It reloads
    scripts from `--scripts DIR` (by default `scenes`) when they change, checking twice a
    second. `--play --capture` runs a game headless, pressing whatever `--press ACTION@FRAME`
    names.
  - On the web, the plain build offers *Play* on scenes with a game, which reloads the page
    with the gameplay build. That build reads keys, the mouse, standard-layout gamepads and
    touch: a stick under the left thumb, looking under the right, and taps. It shows what
    scripts log, and their errors, in a panel.
  - Gamepads natively are left for later: they need a crate of their own (`gilrs`).
  - **A game from a folder.** `viewer SCENE --game DIR` plays the game in a folder in place of
    the scene's own: its `scene.game`, and the actor files, scripts and input actions that
    names, all named from the folder. On the web, `?game=NAME` does the same with
    `games/NAME/` beside the page, which `scripts/build-web.sh` fills from each folder in
    `scenes/` that has a `scene.game`. The page fetches `scene.game` to offer *Play*, and the
    rest only to play; without `?game=` it fetches nothing.
    - This is for scenes baked without a game, or too big to bake again: a script is changed
      and the page reloaded. `scenes/zorah/` gives Zorah doors that open this way.
    - Such an actor has no mesh of its own. Its line names the instances the scene already
      draws (`instances=FIRST+COUNT`), and placed at their hinge, it turns them about it.
      `viewer SCENE --instances-near x,y,z,radius` lists the instances near a point.
    - Instance numbers belong to one bake. Baked again from other sources, a scene may number
      them otherwise, and nothing checks that a game's numbers still mean what they did.
  - With actors but no player, the keys both fly the camera and reach the game.
  - **Flying from the player.** With a player, **V** leaves them standing and flies the
    camera from where it is, and V again puts it back at their eye. The page has a button
    for it too, `?fly` starts that way, and so does `--fly` natively.
    - The game goes on, and hears every key as before, except the script of the one who was
      left: to it nothing is held or pressed, so the keys that fly the camera don't walk
      them off. A door that opens to `viewer()` opens to the flown camera.
    - Being away is part of each step's input, as the viewer's place is, so a replay of a
      run with flying in it is still the same run.
  - Keys typed into the page's own controls don't reach the game.
- **Left for later.** An actor's instances are movable from the start, so a gate that seldom
  moves still has its own shadow pass and leaves the GI voxels. Making them movable only once
  they move, or keeping them in GI, waits until a scene has enough actors for it to matter.
- **Maths.** Starlark has no maths module, so the engine gives `sin`, `cos`, `atan2` and
  `sqrt`.
- **Until physics,** the player walks at a fixed height, through walls.
- **Cost** (`tools/gameplay-bench/b_play`, 9800X3D, warm runs):
  - **Play mode's own work.** Each frame it maps input, works out where each actor is drawn,
    and moves the instances of those that moved. That is about 0.09 µs an actor that stands
    still, on the web and natively.
  - **Turning actors.** With a thousand actors, each with a ticking script that turns it, a
    frame takes 0.75 µs an actor on the web, and 1.3 µs natively, scripts included.
  - **Downloads.** The plain web viewer grows by 879 bytes gzipped (387,396 to 388,275), for
    reading the gameplay section. The gameplay build is 2.15 MB gzipped, and is fetched only to
    play.

## Steps

Each step is its own PR, with mutation-tested tests, measured costs (natively and on the web),
clippy clean on native and wasm32, and a review by a separate AI agent. It is merged only with
your go-ahead, and nothing is published without asking.

1. **Things that move.** *Done.* The renderer API for moving instances, with `prev_world`
   set to the transform each was drawn with last frame; hiding instances and scaling their glow;
   the moving flag, with moving instances left out of the cached shadows, the seabed map and the
   GI voxels, and drawn into per-cascade shadow maps of their own.
   - Demo: the cove's ship bobs at anchor, driven by plain Rust for now (`viewer --bob`).
   - Checked, and decided: with its hull out of the GI voxels, the water around the ship stopped
     reflecting it, and the image moved by 4.5 levels in 255 on average. So an instance can now
     *stay in GI* while movable, held where it stood when the voxels were last drawn: right for
     things that move only a little, such as a ship at anchor. With it, the difference from a
     static ship is 0.4 levels in 255. The demo uses it; a character walking about would not.
   - Tests: a moved instance's motion vectors and shadow follow it; cached shadows of static
     geometry stay as they were; nothing static is redrawn; nothing moving is left behind in a
     cache.
2. **The world.** *Done.* `solstice-world`: entities and components on `hecs`, the
   transform tree, tick groups, events, timers, and the fixed clock with input per step.
   Headless.
   - Tests: tick order; event order; a recorded run replays to the same state; the clock under
     slow and uneven frames.
   - Cost: a step of the benchmark (`tools/gameplay-bench/b_world`: turning a thousand roots,
     working out every place, and an event per root) takes about 16 ns an entity on the web
     (wasm with SIMD, 9800X3D) and 13 ns natively. So 9,000 entities with transforms take
     0.15 ms a step on the web; perhaps 0.3 ms on an integrated-GPU laptop, within the world's
     line of the budget. Every entity with a transform is worked out every step. If scenes grow
     past that, skipping what hasn't moved comes next.
3. **Scripting.** *Done.* `solstice-script` with Starlark: actor scripts, properties and
   `self.vars`, events, timers, the engine API table and its generated bindings, hot reload,
   step and memory caps, and errors with file and line. The benchmarks are run again in place.
   - Tests: hot reload keeps state; a runaway script is stopped and reported, at the same point
     natively and on the web; a broken reload keeps the old script running.
4. **Scenes, actors and play mode.** *Done.* `.actor` files, `actor` lines in scene files and their
   tagged section, input actions, a player camera, and *Play* in both viewers, with the second
   web build.
   - Tests: actor files and scene lines read back what they wrote, and report mistakes clearly;
     an action reaches a script from each kind of input; a scene without gameplay loads into the
     plain viewer exactly as before.
   - Demo: in the cove, walk up to the fort's gate and press E, and it rises
     (`viewer assets/cove.solstice --play`, or *Play* on the web page).
5. **Physics for walking.** *In review.* `solstice-physics`: collision baked and fetched on *Play*, bodies,
   sensors, the character controller, a debug view of colliders, wasm SIMD, and determinism
   measured.
   - Demo: a capsule walks the fort, climbs the stair to the rampart, and opens the gate.
6. **The character and the export tool.** *Done.* The Blender script that makes the MakeHuman
   character, and `tools/blender/solstice_export.py`. Installing MPFB 2 waits for your go-ahead.
   - First, a default character exported as MPFB 2 does it, to see what goes wrong.
   - Tests: the export's checks fail on a deliberately broken character, and pass on the real
     one; two exports of the same script come out the same.
7. **Skinned virtual geometry.** Importing skins, weights through the DAG, per-cluster bone
   lists and bounds, posed node and group bounds, and skinning in both rasterizers, resolve and
   shadows, with motion vectors.
   - First the character in its bind pose, compared with the static import; then one looping
     clip.
   - Tests: no cracks between clusters in any pose; the LOD cut has no holes or overlaps while a
     clip plays.
   - The cost is measured, against the scratch-buffer fallback.
8. **Animation.** Clips and their compression, retargeting from the Universal Animation Library's
   rig onto the character's, blends, blend spaces, state machines, root motion, notifies, and
   animation LOD. Its budget line is measured.
   - Tests: a clip retargeted onto its own rig comes back unchanged; a retargeted walk's feet
     slide less than a set distance while they bear weight. IK in step 9 removes the rest.
   - Demo: the capsule becomes a character that idles, walks and runs.
9. **IK and secondary motion.** Two-bone IK, foot placement, look-at, FABRIK, and spring chains.
   - Demo: feet planted on the stair and the uneven courtyard. This is the first playable
     milestone: a character in the fort.
10. **Gameplay and the environment.** The `Environment` API and its events, with the time of day
    and sea level eased by gameplay; the sun stepped for shadows; lights switched by gameplay;
    viewer controls as overrides.
11. **The sea in physics.** The CPU wave query, checked against the GPU's waves, with its Rust
    `shoal()`; buoyancy: the ship floats, and a rowing boat can be pushed about.
12. **A playable cove.** Walk a character through the fort, open the gate, row out to the ship,
    and see the lanterns lit at dusk. README, screenshots and costs on the RTX 5080 and on the
    integrated-GPU tier. Published only with your go-ahead.

The steps run in this order. The first playable thing is a character in the fort (chosen on
8 November 2026), so walking, the character, skinning and animation (5–9) come before the
environment and the sea (10–11).

## Risks

- **Starlark isn't Python.** People who know Python will reach for `class`, `while` and `try`.
  The framework covers each: actor files, loops over ranges and queries, and errors that stop a
  handler. But it must be taught, with good error messages. The bindings are generated from one
  API table, so changing language later (to Monty, if its interface grows fit for games) changes
  the bindings, not the game's scripts' meaning.
- **Download.** The gameplay build adds about 1.3 MB for Starlark and 0.8 MB for Rapier, gzipped,
  to the viewer's 0.3 MB. Hence the second build, so that scenes without gameplay don't pay for
  it.
- **One thread on the web.** Everything in the budget shares one thread with the renderer's own
  CPU work. Physics on wasm, even with SIMD, ran about 1.3 times slower than native at 500 boxes,
  and its first frames are slow while V8 compiles.
- **The renderer assumes a static world.** Shadow caching, the GI voxels and the seabed map all
  do, and wetness belongs to places rather than objects. Step 1 handles the first three, wetness
  on moving things waits for the rain work, and anything new must say how it treats moving
  things. A moving sun is the same problem in another form, handled in step 10.
- **Skinned virtual geometry is new ground.** Unreal added it in 5.5, as an experimental
  feature, and has published little about it. Step 7 starts with a bind-pose comparison, so any
  crack or LOD error shows up before animation depends on it.
- **Content.** MakeHuman's glTF export has rough edges: users report exporting every clothing
  vertex group unless modifiers are applied, and materials coming out grey when Blender's shader
  nodes are too complex. The export tool (step 6) exists to fix both, and checks its output
  before anything depends on it. Retargeting
  between two different rigs can leave feet sliding or limbs twisted, which the retargeting
  tests and IK are there to catch.
- **Determinism across native and the web** depends on Rapier's option and on avoiding
  platform-specific maths in gameplay code. Tests compare the two builds.
- **Dependencies move.** Starlark and Rapier are both pre-1.0. Both are pinned, and upgraded on
  purpose, as the toolchain is.
