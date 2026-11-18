# The harbour: design

A small harbour town on an Atlantic coast, around 1860, built as the showcase for Solstice 0.3. You
can walk it from the town gate down the lamp-lit lanes to the market square and the quay, out along
the mole to the lighthouse, and back along the beach. Every system in the renderer has a job here
that a real town would give it. This document is the plan, written before anything is built, and
it will be kept up to date as each step lands.

## What makes it believable

- **Real sizes.** Doors are about 2.2 m high, storeys 3 m, lanes 2.6 to 3.2 m wide, and the quay stands
  1.9 m above the sea. The lighthouse's lamp is 22 m above the water.
- **Everything has a reason to be where it is.** The town grew around a sheltered anchorage. The
  mole protects it from the west, the warehouse stands where cargo lands, and the market is one
  street back from the quay, behind the row that takes the weather. Lamps stand where people walk
  after dark: at corners, stairs and the quay's edge, never in a neat grid.
- **The sea is to the west.** North is -Z and east is +X in Solstice, so the sun sets over the
  harbour mouth. Dusk, the scene's best hour, lights the quay front from across the water, with the
  lit windows and the sunset in the same view. The cove faces east, so this also gives a different
  picture from it.

## Layout

The origin is the middle of the quay's edge, at sea level. The sea stands at height 0. To scale:
5 m a column, 10 m a row.

```text
    z  (north is up)
 -130 ~~~~~~~~~~~~~~~~~~~~~~~~^^^^^^^^^^^^^^^^^^^^^^^^,,,,,,,,,,,,,,,,,,,,,,,,,,
 -120 ~~~~~~~~~~~~~~~~~~~~~~~~^^^^^^^^^^^^^^^^^^^^^^^^,,,,,,,,,,,,,,,,,,,,,,,,,,
 -110 ~~~~~~~~~~~~~~~~~~~~~~~~^^^^^^^^^^^^^^^^^^^^^^^^,,,,,,,,,,,,,,,,,,,,,,,,,,
 -100 ~~~~~~~~~~~~~~~~~~~~~~~~^^^^^^^^^^^^^^^^^^^^^^^^,,,,,,,,,,,,,,,,,,,,,,,,,,
  -90 ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~^^^^^^^^^,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,
  -80 ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~^^^^^^^^^,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,
  -70 ~~~~~~~~~~~~~~~~~L#######~~~~~^^^^^^^^^,,,,,,,,,,,,,,,,,,,,,,,I,,,,,,,,,,,
  -60 ~~~~~~~~~~~~~~~~~L#################,,,,,,,,,,,,,,,,,,,,,,,,,,,I,,,,,,,,,,,
  -50 ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~#######=WWWWWWHHHHH..HHHHHHHHH,,I,,,,,,,,,,,
  -40 ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~==HHHHHHHHHHH--HHHHHHHHH,I,,,,,,,,,,,,
  -30 ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~==HHH------HH--HHHHHHHHH,I,,,,,,,,,,,,
  -20 ~~~~~~~~~~~~~~~~~~~~~~~~~b~~~~~~~~~~==HH.......HH--HHHHHHHHH,I,,,,,,,,,,,,
  -10 PPPPPPPP~~~~~~~~~~~~~~~~~~~~~~~~~~SS==HH.......HH--HHHHHHHHH,I,,,,,,,,,,,,
    0 ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~SS==HH...F...HH--HHHHHHHHH,G,,,,,,,,,,,,
   10 ~~~~~~~~~~~~~~~~~~~~~~~b~~~~~~~~~~SS==HH.......HH--HHHHHHHHH,I,,,,,,,,,,,,
   20 ~~~~~~~~~~~~~~~~~~~~~~~~b~~~~~~~~~~~==HHH------------CCCC---,I,,,,,,,,,,,,
   30 ~~~~~~~~~~~~~~~~~~~~~~~~~~|||b||b|||==HHHHHHHHHHH..HHCCCCHHH,I,,,,,,,,,,,,
   40 ~~~~~~~~~~~~~~~~~~~~~~~~~~||b~~b~b~~==HHHHHHHHHHH..HHCCCCHHH,II,,,,,,,,,,,
   50 ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~::::::::,,,,,,,,,,,,,,,,,,,,,,I,,,,,,,,,,,
   60 ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~:::::::,,,,,,,,,,,,,,,,,,,,,,,,I,,,,,,,,,,,
   70 ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~::::::::,,,,,,,,,,,,,,,,,,,,,,,,,I,,,,,,,,,,,
   80 ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~:::::::,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,
   90 ~~~~~~~~~~~~~~~~~~~~~~~~~~~~:::::::,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,
  100 ~~~~~~~~~~~~~~~~~~~~~~~~~~~::::::::,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,
  110 ~~~~~~~~~~~~~~~~~~~~~~~~~~:::::::,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,
  120 ~~~~~~~~~~~~~~~~~~~~~~~~::::::::,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,
  130 ~~~~~~~~~~~~~~~~~~~~~:::::::::,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,
  140 ~~~~~~~~~~~^^^^^^^^^^^:::::,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,
  150 ~~~~~~~~~~~^^^^^^^^^^^^^^^^^,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,
  160 ~~~~~~~~~~~^^^^^^^^^^^^^^^^^,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,
            |         |         |         |         |         |         |       
      -150      -100      -50       0         50        100       150       
                                                                x (metres, east is right)

  ~ sea    = quay    # mole    L lighthouse    | timber pier    b small boat    S merchant ship
  P pinnace at anchor    H houses    W warehouse    C chapel    F fountain    . paving    - lanes
  : beach    ^ rock and cliffs    , grass    I town wall    G town gate
```

| Place | Where | Size | Height above the sea |
|---|---|---|---|
| Harbour basin | x -75 to 0, z -45 to 50 | 75 × 95 m | seabed -3.5 m at the quay, -7 m at the mouth |
| Quay | x 0 to 10, z -52 to 50 | 102 × 10 m | deck 1.9 m |
| Water stairs | z -10, cut into the quay | 2 m wide, 13 steps of 16 cm | |
| North mole | from the quay at (0, -48), curving to (-92, -65) | 100 m long, 7 m wide, reached from the quay by 8 steps | deck 3.2 m, seaward parapet to 4.4 m |
| Lighthouse | (-92, -64) | tower 18 m, base ⌀ 7 m, top ⌀ 4.8 m, lantern 4 m | lamp 22 m |
| Timber pier | from (0, 40) west, T-head at x -45 to -50 | 44 × 3 m, head 20 × 5 m | deck 1.58 m, two steps down from the quay |
| Quay-front row | x 11 to 23, z -36 to 50 | 13 houses, 4.4–7.9 m wide, 2 or 3 storeys, eaves 9–10 m and ridge 12–13.4 m at 3 storeys, eaves 6.5 m and ridge 10 m at 2 | 1.9 m |
| Market square | x 24 to 56, z -14 to 16 | 32 × 30 m | 3.0 m, falling 1:60 towards the sea |
| Fountain | (39, 0) | basin ⌀ 5 m, column 2.4 m | |
| Net Lane | z -30, x 11 to 120 | 3.2 m wide, climbing 1:4 by ramps and flights of steps | 2.5 to 19 m |
| Chapel Steps | z 25, x 11 to 120 | 2.6 m wide, flights of steps | 3 to 19 m |
| Upper Street | x 70, z -34 to 29 | 4 m wide | 7 to 8 m |
| Warehouse | x 14 to 38, z -46 to -36 | 24 × 10 m, 8 m to the ridge | 2 m |
| Chapel | (95, 36) | nave 9 × 18 m, door on Chapel Steps; tower 5 × 5 × 22 m on its seaward side | 12 m |
| Town wall and gate | x about 125 to 133, z -70 to 70 | 6.5 m high, opening 3.4 m | 20 m |
| Beach | z 50 to 140, running south-west | 110 m long, 30 m wide, slope 1:12 | -1 to 2 m |
| South headland, north cliffs | z 140 and beyond; z -130 to -60 | scans 87 m wide, 24 m high | to 30 m |
| Ground behind the town | x 133 and beyond | rising to 52 m at x 300 | |

The front row is broken three times: Net Lane at z -30, a 3.2 m passage from the water stairs up to
the square at z -10, and Chapel Steps at z 25. Above Upper Street the ground climbs 1 in 4, steeper
than the first plan's 1 in 10, because captures from the water showed one flat row of houses. Now
each row stands clear above the one in front, as in a real hillside port.

The player's round, from the gate to the lighthouse and along the beach, is about 450 m: four
minutes on foot, without stopping. The land continues as an island, as the cove's does, so its
edges lie under the sea.

## What each part shows, and why it is there

| Part | Reason in the town | System it shows |
|---|---|---|
| Quay and mole, in cut stone | Landing cargo, breaking the swell | Virtual geometry at real stone-course scale; shadows; water against vertical walls |
| The mole's end and the basin mouth | Open water beyond shelter | FFT ocean and shoaling: waves die in the shallows inside the mole |
| Beach and rocks | Where boats are hauled out, and the swell lands | Wetness: the wet band on sand, run-up, rocks that darken and shine |
| Moored boats and the ship | A harbour's reason for being | Moving instances, motion vectors and their shadows, kept in GI while they bob |
| Lighthouse | Marks the harbour mouth at night | A local light and glow, seen 200 m off across the water |
| Lanes and square at night | Oil and gas lamps | About 40 local lights with their shadows (PR #41), tiled lighting, GI bounce off limewash |
| Limewash, terracotta, granite | The local building materials | GI colour bleed: a white wall lit by a terracotta roof |
| Fountain | The town's water supply, the reason for the square | Particles: lit, sorted mist, solid drops, TAA |
| Chimney smoke, later | Wood fires | Particles (smoke), once particle step 7 lands |
| Weather and time | Moods: a clear dusk, a grey squall | Clouds, sky, stars and moon, sun and moon shadows, eased weather and sea |
| Gate and doors | People shut things at night | Starlark actors, moving instances, collision |

## Assets

Everything is CC0 and fetched by script. None of it is committed.

**From Poly Haven, as models** (`scripts/fetch-scans.py`, 2k):
- *Harbour:* `modular_wooden_pier`, `dutch_ship_medium`, `ship_pinnace`, `ocean_buoy`,
  `lateral_sea_marker`, `lifebuoy`.
- *Stone and rock:* `coastal_cliff_04`, `coast_land_rocks_02` to `04`, `coast_rocks_05`,
  `sand_rocks_small_01`, `boulder_01`, `rock_moss_set_01`.
- *Town:* `modular_fort_01` (the town wall, its gate arch and towers), `large_iron_gate`,
  `large_castle_door`, `street_lamp_01`, `street_lamp_02`, `wooden_lantern_01`.
- *Quayside and market:* `wooden_barrels_01`, `wooden_crate_01` and `02`,
  `wooden_bucket_01`, `wicker_basket_01`, `painted_wooden_bench`, `WoodenTable_03`, `cannon_01`,
  `treasure_chest` (in the warehouse), `tree_small_02`, `fern_02`.

**From Poly Haven, as textures.** The same site publishes each texture as a tiled glTF, so the
importer already reads them. `scenes/harbour.surfaces` lists them in `surface` lines, which
`fetch-scans.py` reads to fetch them with their height maps. So far: `stone_wall_02` (ashlar for the
walls), `cobblestone_floor_07` (setts on the quay), `granite_tile_04` (flags and copings) and
`square_cobblestone` (the square), `white_stucco` (house walls), `clay_roof_tiles_02` (roofs) and
`distressed_painted_planks` (doors and shutters). Ground textures follow.

**Made by a new tool, because no CC0 scan exists.** Poly Haven has no period houses, no small
boats, no lighthouse and no fountain. `apps/townkit` is a small Rust program that generates them
as glTF into `target/town/`, from the fetched textures:
- houses, from eight plans with variation: window reveals, shutters, doors, chimneys, eaves, and
  roofs with real tile geometry;
- the quay, mole, steps and paving, displaced from the textures' height maps, as virtual geometry
  is meant to be used. Each surface is one tile of real relief, instanced along the stonework, on
  plain solids that stay simple for collision;
- the lighthouse, fountain, chapel and warehouse;
- small boats, with planked hulls;
- the terrain, in patches by material.

The scene file then places all of it with the existing `model`, `part` and `actor` lines, as the
cove places the fort. So neither the bake nor the file format changes. The engine still sees only
clean glTF, as with the character's export tool.

**Terrain materials.** A terrain has one material today, so the kit cuts the ground into patches
(sand, grass, dirt, rock) along noisy edges, with ground cover over the joins. If captures show the
joins, the fix is blending by the vertex's spare 16 bits, which is a renderer change and would be
asked first.

## Interactions

Each has a reason, and each says what it waits for. Scripts are Starlark `.actor` files, as the
cove's gate is.

| Interaction | What it does | Status |
|---|---|---|
| Town gate | Press E: the iron gate swings open | Can be built now |
| Doors with a room behind | The tavern, warehouse and lighthouse doors swing on their hinges, onto a small lit room. Doors with nothing behind them stay shut: there are no fake interactions | Can be built now |
| Boats bobbing | Each moored boat rocks by its script, from its place and size, kept small inside the shelter | Can be built now. Riding the real waves waits for gameplay step 11 |
| Lamps at dusk | Lamps and windows come on as the sun goes down | Needs the small renderer change in the decisions below |
| Walls | The player can't walk through anything | Waits for gameplay step 5 |
| Gate locked at night, storm bell on the quay | Scripts reading the hour, setting the weather | Waits for gameplay step 10, after 0.3 |

## Viewpoints and flyover

| Name | From | Looking at | Best at |
|---|---|---|---|
| The harbour mouth | On the water, (-190, 3, 10) | The whole front: mole, ship, quay, town rising behind | Dusk |
| The quay | (5, 3.5, 44) | North along the quay, the ship on the left | Morning |
| The square | (28, 4.7, 14) | Across the fountain to Net Lane | Noon |
| Net Lane | (60, 6.2, -30) | Down the lane to the masts and the water | Night, lamps lit |
| The lighthouse | From the mole, (-40, 5, -60) | The lighthouse against the afterglow | Dusk |
| The beach | (-38, 2.4, 95) | The wet sand, back towards the pier and town | Sunset |
| Above the town | From the chapel tower, (88, 38, 30.5) | Roofs, harbour, sea | Dusk |

A **75 second flyover** (`scenes/harbour_flyover.txt`, as the cove's): in from the sea past the
pinnace, around the lighthouse, along the mole, low over the moored boats, up the water stairs,
through the square, up Net Lane, over the roofs, and out above the bay. It starts at 19:30 and ends
in the dark, so it also shows the lamps coming on.

## Budgets

The cove is the reference: 3.8 ms on average on the RTX 5080 at 1600×900 High, 30 ms on the
integrated GPU at 1280×720 Low, and a 209 MB streamed browser download. The harbour has more
instances and many lights, so its targets sit above that:

| | RTX 5080 | Integrated GPU (Radeon), Low |
|---|---|---|
| Average frame | 6 ms at 1600×900 High, 10 ms at 2560×1440 | No separate limit: 41 ms at 1280×720 with the houses in |
| 95th percentile, by day | 9 ms | 50 ms |
| 95th percentile, with lamps and fountain | The by-day figure plus the two rows below | 55 ms |
| Night, all lamps lit | At most 1.5 ms more | At most 4 ms more |
| Fountain particles | 0.1 ms | 0.5 ms |
| Physics and scripts | 0.5 ms of CPU | 2 ms of CPU on the web |

The integrated GPU's limits were first 33 ms on average and 45 ms at the 95th percentile. The
renderer's fixed passes at Low (GI flood 4.1 ms, GI light 5.8 ms, screen GI 5.3 ms and post 5.0 ms)
take about 20 ms of every frame whatever the scene holds, and step 2's plain blocks already used
all of the 45 ms. Textured walls alone reached 46–47 ms, and flat roofs saved only 0.5 ms, so no
cut to the scene could get back under it without losing the houses. The limits were raised on
2026-11-09 and the houses kept, at about 48 ms at the 95th percentile. Cutting those fixed passes at Low is separate
renderer work, which would help every scene.

| Content | Budget |
|---|---|
| Browser file, 1024 textures, BC | At most 300 MB, streamed |
| Source triangles | At most 40 M unique, instanced to at most 200 M |
| Instances | At most 6,000 |
| Texture layers | At most 40 |
| Local lights | About 40, each with range 12–18 m, at most 8 overlapping |
| Collision, fetched on *Play* | At most 5 MB |

If the integrated GPU overruns the raised limits, the first cuts are fewer lamps and half the
particles; roof tiles save little, about 0.5 ms for all of them. Each step measures on both GPUs.

## Collision

Physics is gameplay step 5, and the bake will make collision from the meshes (`docs/gameplay.md`).
This is what the gameplay session proposes, provisional until its PR. The player is a capsule 0.3 m
in radius and about 1.8 m tall. So the scene is built to make that easy:
- every walkable surface is one continuous mesh with no gaps, and steps are at most 17 cm high
  (the controller allows 30 cm), with slopes under 45°;
- doorways are at least 0.9 m wide and 2.1 m high, and lanes are far wider than the 1.2 m minimum;
- every wall is a closed shape with no thin slivers, and each house, whose mesh is an open shell,
  gets one plain solid for collision;
- small props (crates, barrels) are simple enough for convex hulls, and `collide=none` on a line
  marks what you walk through, such as ground cover;
- until the sea has physics, walking off an edge would drop the player to the seabed, so the quay
  and mole edges and the beach's waterline get `blocker X Y Z SX SY SZ YAW` lines;
- moving doors and the gate are actors with `collider box` lines. Their meshes are named parts of
  the kit (`mesh PATH SCALE part=NAME`), with each part's origin at its hinge;
- the ground is kit meshes, at most about 300 m across and 500k triangles each, marked
  `collide=coarse` for a 10 cm cut.

## Steps

Each step is its own PR off `main`, reviewed by a separate agent, with captures from several
viewpoints and times of day judged as a whole picture, and costs measured.

1. **The kit and the blockout.** `apps/townkit`; terrain, quay, mole and the basin, and plain
   shapes for everything else, in plain materials, with the sea. Judge scale and composition from
   every viewpoint.
2. **Stone.** `fetch-scans.py` for textures; ashlar walls and paving on the quay, mole and square,
   copings and iron bollards; the pier; props along the quay.
3. **The houses.** The eight plans and their variations; the quay-front row and the back rows.
4. **The town.** Lanes and steps; the warehouse; the chapel; the wall and gate; the fountain, with
   its particles.
5. **The lighthouse and the lamps.** The lighthouse, the lamp posts and wall lamps, window glow,
   and the lamps coming on at dusk. Needs PR #41 merged.
6. **The sea and the shore.** A `harbour.ocean` for the sheltered basin; the beach, rocks and
   cliffs, with rock armour where the land meets the sea abruptly at the quay's two ends; the wet bands judged in sunlight.
7. **The boats.** The small boats and the ship, moored and bobbing by script.
8. **The interactions.** The gate and doors, with the rooms behind them.
9. **Collision.** With gameplay step 5: walls, steps, edges and the doors.
10. **Finish.** Viewpoints, flyover, the web viewer's scene list, a publish entry (published only
    when you say so), README and costs on both GPUs.

## Decisions

Approved on 2026-11-09, all five as recommended.

1. **The look.** I recommend an Atlantic town of about 1860, Galicia or northern Portugal: limewash
   and granite with terracotta roofs. It gives colour for GI to bleed, stands up to rain and
   sunshine, and suits the cast-iron lamps and the fort's stone. The alternative is Cornish slate
   and granite, moodier but greyer.
2. **Houses and boats by our own tool.** I recommend `apps/townkit`, since nothing CC0 exists. The
   alternative is building them in Blender by script, which is slower to iterate on.
3. **Lamps at dusk.** Script control of lights is gameplay step 10, which comes after 0.3. I
   recommend a small general renderer change now: a light or glow can be marked *night-only*, and
   fades in as the sun sets. It would work in the plain viewer too, and in the flyover. It touches
   `renderer.rs`, so it goes through the parent session first. The alternative is lamps always on,
   which looks wrong by day.
4. **The fountain needs `effect` lines in scene files.** The particles design plans them, and the
   bake doesn't have them yet. I'll add them unless the particles session already is.
5. **Rain.** Particle rain is step 5 of that work, and rain wetting the streets isn't designed yet.
   I recommend 0.3 not waiting for either: a squall works with grey skies and a rough sea, and rain
   joins when it lands.

## Risks

- **House detail against download size.** Real tile geometry on about 70 houses is the largest cost. The
  roofs are instanced by plan, so a house shared by three plots costs once.
- **The seabed map is a square around the camera.** From well above the town (over 100 m) its edge
  shows as a straight line where the water's colour changes. The flyover stays below that height.
- **Waves pass through the mole.** The FFT sea knows only depth, so a shallow basin stands in for
  shelter. The boats' bobbing is scripted, so it may disagree a little with the waves.
- **Wet bands stay where they are.** A boat that moves carries no wet band of its own
  (`docs/wetness.md`). Moored boats move only a little.
- **Dependencies.** The lamps wait for PR #41 and the night-only lights, walking for gameplay step 5,
  and rain for particle step 5. Rain is the only one 0.3 can ship without.
