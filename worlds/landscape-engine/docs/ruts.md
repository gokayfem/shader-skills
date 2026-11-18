# Ruts: ground that keeps what is driven into it, and mud that looks wet

A truck driven through a bog leaves two ruts as deep as it sank, with what it pushed aside
heaped along them and its tread printed in the floor. The wheels behind ride in the ruts of the
wheels ahead, a wheel that spins digs itself down to its axle, and water stands in whatever is
deep enough. This says how that is built, what to tune, what it costs, and how to look at it.

## Where everything is

| What | Where |
| --- | --- |
| The map of what has been pressed into the ground | `crates/solstice-vehicle/src/rut.rs` (`Ruts`, `Press`) |
| How each surface takes a mark | `crates/solstice-vehicle/src/surface.rs` (`Soil`, `Surface::soil`) |
| A tyre standing in a rut, digging, and leaving its mark | `crates/solstice-vehicle/src/vehicle.rs` (`Ground::rut`, `Ground::press`, `Vehicle::mark`, the "floor" lines in `substep`) |
| The scene's map, and the belly dragging | `crates/solstice-physics/src/car.rs` (`SceneGround`, `drag_belly`), `Physics::ruts` |
| The map on the GPU | `crates/solstice-render/src/ruts.rs` (`RutMap`, `RutSource`), `Renderer::sync_ruts` |
| Drawing it, and mud as it lies | `crates/solstice-render/shaders/ground/ruts.wgsl`, called from `vg/resolve.wgsl` |
| Joining the game's map to the renderer | `apps/viewer/src/play.rs` (`draw_ruts`) |

## The approach, and why

The land is baked. It is virtual geometry: clusters of triangles made once, with points half a
metre apart and an eighth of a metre along a track, streamed in pages and never changed. A rut
is a tyre wide and a hand deep, its berm a few centimetres, its tread's print a centimetre. So:

- **Moving the land's own points would not do it.** Even along a track there are two or three
  points across a tyre, and off it none. And every pass that places a point would have to move
  it the same way (both rasterizers, the shadow passes, culling bounds, the GI's voxels), as
  swaying in the wind does, for a result too coarse to show a rut's edge.
- **Laying new geometry over the land would not either.** A rut is below the land's surface,
  which would hide it.

What is built is **a map of how far the ground has been pressed down or heaped up, read by the
wheels and by the shader that works out each pixel's material**:

1. **One map for both** (`rut.rs`). Cells of 4 cm, in squares of 64 × 64 cells (2.56 m) that are
   made only where something has been driven. Each cell holds the ground's height against how
   it was made, the tread's print, how wet it is, and how much churned soil shows: eight
   bytes, laid out as the renderer samples them, so the game's copy is copied to the GPU as
   it is.
2. **The wheels read it and press it** (`vehicle.rs`). A wheel's ray meets the land as before;
   the depth of the rut at that place is then the floor the tyre stands on. The tyre sinks
   below that floor only as far as it would have sunk in ground nobody had pressed, and each
   step presses the map to where the tyre's bottom now is. So the tyre sits in the rut that is
   drawn, to the cell.
3. **The shader draws it** (`ground/ruts.wgsl`). For each pixel of ground, the eye's ray is
   followed through the pressed ground to where it meets it (relief mapping, with the land's
   own surface as the level nothing was pressed from). The ground's textures are laid there,
   its facing is the pressed ground's, and it is shaded as what it is. A rut is seen into: its
   near wall hides its floor, its berm stands in front of what is behind it, and the low sun
   leaves its floor in shade. A tyre sunk in it is drawn where the physics has it, with the
   land's surface hiding what is below the ground: it sits in the rut.

What this does not give: the rut does not change the land's silhouette (a deep rut seen
exactly side on, at ground level, is flat), the sun's shadow maps and the GI do not know of it
(the shade in a rut is worked out in the shader and laid on the surface's colour), and things
other than wheels and a truck's belly do not fall into it: a loose thing or somebody walking
still stands on the land as it was made.

## What the ground does

### A tyre in soft ground

`SurfaceGrip` (in `surface.rs`) already said how far a tyre sinks into each surface and how
much further a spinning one digs. That is unchanged, and on ground that keeps no ruts (the
circuit tests' `Flat`) nothing behaves differently. On ground that keeps them:

- **The floor.** `Ground::rut` gives the depth of the rut under the tyre. The tyre stands on
  it. If the tyre would sink further than that in fresh ground, it sinks the rest of the way
  (taking `SINK_SECONDS`) and the rut is pressed deeper; if not, it rides in the rut as it is.
  The wheels behind therefore ride in the ruts of those ahead, and a second truck down the
  same ruts makes them no deeper.
- **Digging.** A wheel that spins reaches a little below the floor of the hole it is in
  (`DIG_ON` times the surface's `dig`), each moment, so a wheel left spinning goes down
  steadily: about 5 cm a second flat out in a bog, a quarter of that with traction control
  holding the slip. It stops at `deepest`: `DIG_SOFT` times the surface's sink plus `DIG_SPUN`
  times its dig, and never more than `DIG_RADIUS` of the tyre's radius, by when the truck is
  on its belly.
- **Drag.** As before, `RUT_DRAG` times the tyre's depth over its radius, as a share of its
  load. The depth is now the rut's, so a deep rut holds a truck back whoever made it.
- **Berms.** A tyre on a berm rides over it (the "depth" there is below zero) and, in soft
  ground, squashes it.
- **The belly.** Where the truck's underside is within `BELLY_NEAR` of soft ground, the crown
  between the ruts is pressed flat `BELLY_PRESSES` deep in a broad smear (`drag_belly`). The
  truck's body still collides with the land as it was made.

### The mark a tyre leaves (`Vehicle::mark`, `Ruts::press`)

Once a step, each tyre on the ground presses a strip from where its contact was to where it
is:

- **The rut**: flat under the tread, rounded at the shoulders (`profile`), as deep as the tyre
  has sunk, and no deeper than that ground can be pressed, so a tyre climbing out of a bog on
  to rock leaves the rock alone. The ground only ever goes down under a tread.
- **The berm**: the share of what was pressed out that the surface's `Soil::berm` says is
  heaped rather than packed (half in mud, a fifth in leaf litter), in a ridge just outside each
  shoulder, at most 0.6 of the rut's depth and 12 cm. A berm heaped where a rut already is
  fills some of it.
- **The tread's print**: each cell under the tread takes where in the tread's pattern it was
  pressed (the wheel's own angle, `TREAD_LUGS` lugs to a turn) and how crisp the print is: 1
  for a tyre that rolls, falling to 0 as its slip passes a third. So a rolling tyre prints
  every lug, a spinning or locked one smears its own print and whatever was there, and the
  rear tyres print over the fronts'.
- **Wet and churned**: `Surface::soil` says how wet what is churned up is and how wet it stays
  (`Soil::wet`, `stays`), and how much bare soil is left showing (`churn`). A bog's ruts are
  sodden and stay so; a clay track's are damp after rain and dry pale.
- **What the tyre carries**: a tyre out of a creek is wet, and one out of a bog muddy
  (`Vehicle::carried`), for `CARRIES_WATER` and `CARRIES_MUD` metres of rolling. It leaves
  wet, muddy tracks on whatever it rolls on next, rock included, with no rut under them.
- **Drying** (`Ruts::advance`): what is wetter than it stays dries towards it over
  `DRY_SECONDS`, a few squares at a time.

A game can lay lumps of thrown mud in the same map (`Ruts::splat`): a blot a few centimetres
across, wet, standing a little proud, covering whatever print was under it.

Ruts last as long as the map is being played. They are not kept in a saved game, and a map
that is left and come back to is fresh.

## How it looks (`ground/ruts.wgsl`)

All of it is decided where a pixel's material is, before lighting, so the sun, the sky, the
lamps, reflections and the rain all light the pressed ground as they light anything.

- **Churned soil** (`rut_shade`): where the ground was broken, soil the colour of the country
  (`Renderer::set_rut_soil`, red clay unless told) shows in place of what covered it, in clods
  and smears, lumpier and paler on a berm.
- **The print**: `rut_tread` is a mud tyre's pattern, blocks down the middle and lugs at the
  shoulders, pressed in `RUT_PRINT_DEPTH` deep. It is drawn within `RUT_PRINT_TO` metres.
- **Shade**: low ground sees less sky (its cavity is deepened), and a few steps towards the
  sun find whether a wall or a berm stands in the way.
- **Wet**: the cell's wetness goes through the same wet material as the sea's and the rain's
  (`common/wetness.wgsl`): pores fill and the soil darkens, then a film forms and it shines.
  A fresh rut in a bog is dark and glossy; one in a clay track after rain is dark and damp;
  both dry paler.
- **Standing water**: water stands in a rut deeper than a brim that falls from 22 cm to
  2.5 cm as the cell's wetness goes from 0.55 to 1, and falls further as the rain fills the
  puddles. It is brown with clay, level and smooth, with a wandering edge. So a bog's ruts
  hold water at once, a clay track's after rain, and dry ground's never.
- **Mud nobody has driven on** (`mud_lies`): ground whose material is marked `Material::MUD`
  is drawn as mud lies. Where there is most of it, it is wet clay: darker, with a sheen that
  comes and goes across it, and water standing in its hollows. Towards its rim, where it
  thins out, it has dried to a paler crust cracked into plates a hand or two across, each its
  own shade, with finer cracks inside them. Rain wets all of it and closes the cracks.
  **A map baked before this has no such mark**: its mud is drawn as before until it is baked
  again. Ruts need no rebake.

## Numbers to tune

| Number | Where | Now | Does |
| --- | --- | --- | --- |
| `CELL` | `rut.rs` | 0.04 m | The map's fineness. Halving it quadruples memory and what is copied. |
| `TILES` | `render/src/ruts.rs` | 24 (12 in a browser, on a phone) | How far round the truck ruts are drawn: 61 m across (31 m). |
| `DRY_SECONDS` | `rut.rs` | 150 s | How long a fresh rut keeps its sheen. |
| `Soil` values | `surface.rs` | per surface | Wet, stays wet, churn, berm, print. |
| `SurfaceGrip::sink`, `dig` | `surface.rs` | per surface | How deep a tyre sinks and a spinning one digs: as before. |
| `DIG_ON` | `vehicle.rs` | 0.25 | How fast a spinning wheel goes down. |
| `DIG_SOFT`, `DIG_SPUN`, `DIG_RADIUS` | `vehicle.rs` | 1.5, 2.5, 0.7 | How deep it can dig itself. |
| `RUT_DRAG` | `vehicle.rs` | 0.35 | How hard a rut holds a truck back. |
| `TYRE_WIDTH` | `vehicle.rs` | 0.7 of the radius | The rut's width. |
| `CARRIES_WATER`, `CARRIES_MUD`, `WASHES_MUD` | `vehicle.rs` | 18, 30, 4 m | How long a tyre's tracks stay wet and muddy. |
| `BELLY_NEAR`, `BELLY_PRESSES` | `physics/src/car.rs` | 2, 2.5 cm | When the belly drags, and how deep its smear is. |
| `RUT_ABOVE`, `RUT_BELOW`, `RUT_REACH` | `ruts.wgsl` | 0.13, 0.42, 1.6 m | The slab the eye's ray is followed through. |
| `RUT_TRACED_TO` | `ruts.wgsl` | 45 m | Beyond it ruts are shaded but not seen into. |
| `RUT_PRINT_DEPTH`, `RUT_PRINT_TO` | `ruts.wgsl` | 7 mm, 14 m | The tread's print. |
| The brim (`rut_shade`) | `ruts.wgsl` | 22 cm to 2.5 cm | How deep a rut must be to hold water. |

## Cost

- **Where nobody has driven: next to nothing.** The map is an empty table: a wheel's look in
  it returns at once. `Renderer::sync_ruts` returns at its first line. The shader tests one
  number of the frame's constants (`rut_window.z`) for each pixel of ground and skips all of
  it. Mud's own look (`mud_lies`) runs only on pixels of mud.
- **Memory.** A square of the map is 36 kB in the game (its cells, and how wet each stays), so
  a kilometre of two ruts is some 30 MB. On the GPU, one texture of 1536² texels in two layers
  with its smaller sizes: 25 MB on a desktop, 6 MB in a browser or on a phone, allocated
  whether or not anything is driven.
- **Each step**: four strips pressed, some two hundred cells each, and four looks in the map
  for each of ten short steps.
- **Each frame**: the squares that changed are copied to the GPU, at most 24 a frame, nearest
  the truck first: under a moving truck, four to eight, 48 kB each with their smaller sizes.
- **The shader**: on ground with ruts within a metre or two, one look to find that out, up to
  22 steps of the eye's ray and 5 to settle where it meets, four for the facing, seven for
  the shade, two for what is there, and six more for the tread's print close to the camera.
  Elsewhere, the one look.

Measured on an RTX 5080 at 1920 × 1080, on a flat map of bog made by the game these came from
(`gpu ms` in a `--capture` run's report): the resolve pass takes 0.32 ms with the bog in view
and nothing driven, and 0.39 ms with a truck in the bog and its ruts behind it across the
middle of the picture. Not measured: an integrated GPU, a browser, a phone.

## Looking at it

Ruts are kept wherever a car is driven on ground that takes a mark (`Surface::soil`): deep in
mud, a bog and sand, shallow on grass, gravel and dirt, and not at all on tarmac, concrete and
rock. The viewer draws them round whatever is driven (`play.rs`, `draw_ruts`).

- On a circuit, what takes a mark is what its track says lies beside the road: the grass and
  the gravel traps.
- Anywhere else, a scene's text names a map of its ground, `surfaces FILE`
  (`solstice_game::SurfaceMap`, `crates/solstice-game/src/ground.rs`): a grid of squares, each
  of one surface, which each tyre reads the square it is on of. A scene with such a map that
  has mud in it, and a car, is all it takes. None of the engine's own scenes has one yet.

## What is not right yet

- **Deep ruts in a storm.** With the ground soaked the truck digs ruts of 25 to 30 cm, whose
  berms are 12 cm high. Under an overcast sky, beside ground shining with rain, those berms
  read as dark slabs. They want lumpier, paler crests, or less height.
- **A rut has no edge against the sky**, and the sun's shadow maps do not know of it: see
  "The approach".
- **None of the engine's own scenes has mud or a bog**: this has been driven in the game it
  was built for, and here only in tests.

## Tests

- `crates/solstice-vehicle/src/rut.rs`: pressing, berms, that a rut stays and only a deeper
  wheel deepens it, crossings, the tread's print and its smearing, drying, the squares listed
  as changed, and that the bytes the renderer is given are the height the wheels read.
- `crates/solstice-vehicle/tests/ruts.rs`: a truck on ground that keeps its ruts. A standing
  truck's tyres are as deep as the map says; it leaves ruts and berms behind it; rear wheels
  ride in the fronts' ruts; a spinning wheel digs in; four-wheel drive still crosses a bog and
  two-wheel drive still does not; a deep rut slows a coasting truck; wet tracks on rock out of
  a creek.
- `crates/solstice-render`: the frame's constants lay out as their shader twin, and every
  shader compiles, with the ruts' among them.
