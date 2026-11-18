# Wind, rain and thunder: design

Grass that waves as a gust rolls across the meadow, trees whose trunks lean while their branches
bob and their leaves flutter, a sea and ponds roughened by the same gust as it passes, rain that
leans with that wind, darkens the stone and fills the hollows, and lightning that throws the
scene's shadows for a moment before the thunder arrives. This is the plan as it was written before
any of it was built. It has been built since (#67 to #71); where the code went another way, the
code and its module comments are the reference, and the notes below say so.

## Goals

- **One wind.** A single wind field, mean wind plus gusts and turbulence, that grass, trees, the
  sea, ponds, rain and every particle read. A gust is seen crossing the meadow, then the pond, then
  bending the rain, in that order, because it is one thing moving downwind.
- **Real motion.** Grass bends with the square of the wind speed and springs back. Trees move as a
  hierarchy: the trunk sways slowly, branches bob faster on top of it, leaves flutter fastest.
  The sea gets cat's paws and whitecaps from the gusts; ponds get wind ripples.
- **Real rain.** Streaks lit by the scene, falling at their terminal speed and leaning with the
  wind; splashes on the ground and rings on water; surfaces darkening as they wet, with shelter
  under overhangs; puddles in hollows with rings on them; drips from eaves; haze and rain shafts
  in heavy rain.
- **Real thunder.** Lightning that lights the scene and casts sharp shadows, a branching bolt,
  clouds flashing from inside, and thunder heard after the flash by its distance.
- **Cheap, and free when unused.** A scene with no swaying plants, no rain and no storm pays
  nothing: no pass dispatched, no shader branch taken. Budgets are below.

## What there is today

- **Weather** (`clouds/weather.rs`, `weather/*.weather`): coverage, type, base, thickness,
  density, cirrus, `wind speed= from=` at cloud height, evolution and `rain` 0..1. `rain` only
  darkens cloud bases and greys the air. `WeatherCycle` (KeyC in the viewer) moves between the six
  built-in weathers. `Observation` reads WMO codes, 95–99 being thunderstorms, from Open-Meteo.
- **Wind today** is a constant vector. The renderer halves the cloud-height wind for the ground.
  Particles add a log profile and a fixed gust noise of their own (`particles/common.wgsl`). The
  sea's spectrum follows the weather's wind slowly (`ocean/sea.rs`). Ponds ripple in five fixed
  directions regardless of the wind (`ocean/pond.wgsl`). Nothing else moves.
- **Foliage is real geometry in the virtual-geometry pipeline**: grass patches of folded blades
  (`solstice-procgen/src/foliage/grass.rs`, 2 m patches in the Forest) and trees grown as tubes of stems with
  leaf cards (`foliage/tree.rs`), with the `FOLIAGE` material flag (two-sided, transmission).
  Vertices are 12 bytes: position, normal, UV. There is no per-vertex room for wind data.
- **Skinning** (PR #50/#54) already deforms virtual geometry "where it is used": cull bounds
  (`posed_sphere`), both rasterizers (`posed_position`), and resolve (positions, normals, and
  last frame's pose for motion vectors). Bones live in a texture, uploaded from the CPU.
- **Shadows** cache static instances per cascade and redraw only when the view moves. Movable
  instances are redrawn every frame into a half-resolution shared layer. Making all foliage
  "movable" would redraw the whole Forest into four cascades every frame.
- **Particles**, steps 1–4a of `docs/particles.md`: GPU pool, lighting, TAA mask, sorting,
  forces (wind, curl), collision against depth, the distance field and the water, and events.
  **Not built:** 4b (the cover map) and 5 (rain and snow). This plan takes those over.
- **Wetness** (`common/wetness.wgsl`, `docs/wetness.md`): the wet material (pores filling, then
  a film), driven only by the sea's water-line map. Rain was left for later, to reuse it.
- **Ocean**: FFT waves, foam where crests fold, shading with sub-pixel slope variance.
- **Lighting**: sun with cascades, sky, GI volume (with a distance field), local lights whose
  shadows are traced through the GI volume once a frame. Exposure adapts over time.
- **No audio** anywhere.
- **Which scenes have what**: the Forest has the grass and ~hundreds of trees and bushes; the
  Cove has the sea and photoscanned rock, no plants; the Temple has stone, no plants or water.
  The circuits (PR #64) have grass and trees too.

## References

- **Wind**: Taylor's frozen-turbulence hypothesis (gusts carried downwind at the mean speed); the
  log wind profile; gust factor ~1.3–1.6 over open land (WMO); Kaimal spectrum for how fast
  turbulence varies.
- **Grass**: Habel et al., *Physically Guided Animation of Trees* (2009); the "honami" waves in
  crop fields (Inoue 1955), which are gust cells advected over the canopy.
- **Trees**: Sousa, *Vegetation Procedural Animation and Shading in Crysis* (GPU Gems 3, ch. 16):
  main bending, detail bending, leaf edge flutter; Pivot Painter (Unreal) and SpeedTree's
  hierarchical wind; Zhao & Barbič natural frequencies (trunks ~0.2–1 Hz, branches 1–4 Hz,
  leaves 5–15 Hz).
- **Sea**: Monahan & O'Muircheartaigh (1980) whitecap coverage `W = 3.84e-6 · U^3.41`; Cox & Munk
  (1954) slope variance growing with wind speed, which is what a gust patch changes.
- **Rain**: Garg & Nayar (2006) rain streaks; Tatarchuk (2006) city rain: streak layers, cover
  map, splashes, drips; Marshall & Palmer drop sizes; Gunn & Kinzer fall speeds; Lagarde,
  *Water drop* series (2012–13): wet surfaces, puddles, rain ripples.
- **Lightning**: Rakov & Uman, *Lightning: Physics and Effects*: return strokes 3–5 per flash,
  40–60 ms apart, ~30 µs bright peak followed by continuing current; Kim & Lin (2007) bolt shapes.
  Thunder is heard at 343 m/s, rolling for seconds from a long channel.

## The design

### 1. The wind field

One small module, `wind.rs` in the renderer, with its WGSL twin `common/wind.wgsl`. Everything that
moves with the wind includes the WGSL and reads the same few values.

- **Mean wind.** From the weather: the cloud-height wind brought to 10 m (as now, about half),
  then a slow veer of ±10–20° and a slow swell in strength from low-frequency noise, so the
  direction is never exactly constant.
- **Gusts and lulls.** A 2D field, 128² texels of `rgba16float` over 256 m around the camera,
  fixed in the world and wrapping as the water-line map does, so it doesn't slide with the
  camera. Each texel: wind velocity (east, south) near the canopy, a gust strength, and
  turbulence (how chaotic the air is there, for flutter).
  - Updated each frame by one small compute pass: noise cells elongated across the wind,
    advected downwind at the mean speed (Taylor), evolving slowly as they go. Gust strength by
    the weather's `gusts` setting; gust size ~20–60 m, as over grassland.
  - Last frame's field is kept (two textures taking turns), so anything displaced by the wind
    can write correct motion vectors for TAA.
  - Beyond the field's 128 m reach the mean wind applies, with the gusts fading out over the
    last 16 m.
- **Height.** The log profile, as particles have now: weaker near the ground, stronger above.
  A tree's crown feels more wind than the grass below it.
- **The CPU's copy.** The same function on the CPU (`wind.at(position, time)`), used for the
  trees' simulation tests, for particles' reference, for gameplay (flags, cars' crosswind, a
  ball's drift) and for the sound of the wind later. The GPU texture is checked against it.
- **Weather settings**, new in `.weather` files (a format change, see Decisions):
  `wind speed=16 from=230 gusts=0.6 turbulence=0.5`. Old files read gusts from the wind speed.
- **Debug view `wind`**: arrows and gust strength over the scene, from above.

**Cost**: one dispatch of 128² threads, two 128 KiB textures. Under 0.01 ms on the RTX 5080.

### 2. Grass

Grass is moved where its vertices are used, as skinning is: in the cull's bounds, both
rasterizers, and resolve.

- **Which instances sway.** A new material flag, `SWAYS` (a new bit in the existing flags, see
  Decisions). Grass patches, weeds and bushes' leaves are marked by their bake. Nothing unmarked
  is ever moved.
- **The bend.** Each vertex is bent by its height above the instance's own ground plane (the
  patch's local `y`; blades grow up from it):
  - the wind at the vertex's place in the world, read from the gust field, so neighbouring
    blades move together and a gust rolls across the meadow as a wave;
  - a bend angle from drag, growing with the wind speed squared and limited, plus a small
    oscillation at the grass's natural frequency (~1.5 Hz, varied per blade by a hash of its
    position) so a steady wind still shimmers;
  - the offset grows with height squared, as a cantilever bends, and the vertex is lowered to
    keep the blade's length, so tips never stretch;
  - normals tilted with the bend, so a gust shows as a lighter or darker band as blades turn
    their faces to the sun: much of what makes rolling grass read.
- **Motion vectors** from the same bend with last frame's field and time.
- **Bounds**: a patch's clusters and LOD spheres grow by the most a bend can carry a point
  (stored per mesh like `lod_margin`), so culling and LOD choices stay correct.
- **Shadows**: see [Shadows of moving things](#shadows-of-moving-things).
- **Distance**: the bend fades out from 40 m to 60 m on High (20–30 m on Low). Beyond it grass
  is a texture-scale carpet where the bend can't be seen, and it skips the wind code entirely.

### 3. Trees: a hierarchy

Trees need branches that move on their own, which a function of position cannot give: a branch's
vertices must turn about the branch's own base. The grower knows the hierarchy when it makes the
tree, so it keeps it.

- **Each tree gets a small skeleton** at grow time: trunk segments, the first-order branches and
  the larger second-order ones, ~20–40 bones per tree, at most 255 per mesh as skinning allows.
  Each vertex is weighted to the stem it belongs to and its parent, through the existing skin
  records. Twigs and leaves follow their branch. **This uses the existing skinned-mesh format,**
  so the file format does not change. The Forest must be re-baked (see Decisions).
- **The bones are simulated on the GPU** each frame by a compute pass, one thread per bone of
  each visible tree, writing straight into the bone texture:
  - each bone is a damped spring (angle and angular velocity, two axes) with a natural
    frequency from its length and thickness: trunk ~0.3–0.8 Hz, branches 1–3 Hz;
  - it is driven by the drag of the wind at its own place in the world, read from the gust field
    and the height profile, so a gust reaches the windward side of a crown first;
  - rotations compose from the trunk outwards, so a branch rides on the swaying trunk;
  - the state (two vec4 per bone) persists frame to frame, so motion has momentum and a gust
    leaves the tree ringing down, rather than snapping back;
  - a CPU reference of the same spring, tested against the GPU.
- **Leaves flutter** on top, procedurally per vertex in the rasterizers and resolve: a fast
  twist and lift of each leaf card about its stalk (~6–12 Hz, phase per leaf from a hash), its
  size scaled by the local turbulence and wind speed. Leaves' undersides catch the light as
  they flip, as aspens do.
- **Distance LOD**: trees beyond ~120 m (60 m on Low) stop simulating; their bones ease to rest
  and they are drawn unskinned from then on, which also returns them to the static shadow cache.
- **Bushes** get the same treatment with fewer bones (~6–10).

The alternative was a new vertex stream of pivots and levels, as Unreal's Pivot Painter bakes.
It gives the same look without bones, but it changes the file format and would need a second
deformation path through every pass that skinning already covers and tests.

### Shadows of moving things

The cached cascades are what keeps shadows cheap, and the Forest is almost all foliage. So:

- **Far cascades (2 and 3) keep swaying instances static**, at rest. Sway is invisible there.
- **Near cascades (0 and 1, out to 38 m)** draw them as movers: excluded from the static cache
  and redrawn each frame into the moving layer, at its half resolution. Dappled light under the
  trees moves with the leaves, which is one of the strongest cues that wind is real.
- **Grass** stays static in all cascades by default: its shadows are blade-scale and soft at
  half resolution. If its frozen shadow is visible on the moving blades, the near cascade alone
  can take it, measured first.
- **On Low**, swaying instances are static in every cascade.
- **GI voxels** and the cover map see everything at rest.

### 4. Water

- **The sea** already takes its waves from the mean wind. It gains, from the gust field:
  - **cat's paws**: under a gust the short, unresolved waves grow, which the shading already
    accounts for as slope variance (Cox and Munk). A gust patch darkens and roughens the water
    and its glitter as it runs across the bay, ahead of the long waves;
  - **whitecaps** scaled by the local wind by Monahan's law, adding to the foam the folding
    crests already make, so a gale's sea is streaked white and a breeze's is not;
  - **spray** off crests in strong gusts, as particles step 6 planned, if time allows.
- **Ponds** lose their fixed ripples. Their short waves come from the gust field: direction from
  the local wind, height growing with the wind and with fetch (the distance across the pond
  upwind), none in the lee of the bank. A gust is seen crossing the pond as a dark patch.
- **Rain on water**: rings and splashes from a shared ripple texture (below), and a calmer,
  matte look between rings, as heavy rain damps the capillary waves.

**Cost**: one more texture read per water pixel. Under 0.03 ms on the RTX 5080.

### 5. Rain in the air

This is `docs/particles.md` steps 4b and 5, built on the particle system that exists.

- **The cover map** (particles 4b): the scene seen from above, 512² over 128 m around the
  camera, drawn through the shared culling and rasterizing, at rest, cached and redrawn only
  when the camera moves far or the scene changes (like a shadow cascade). Rain stops at the
  first surface above it; the same map shelters surfaces from wetting and finds eaves for drips.
- **Drops** (built as a pass of its own, `rain/draw.wgsl`, rather than as a particle effect):
  camera-following, its rate and drop
  sizes from the rain rate by Marshall and Palmer, falling at Gunn and Kinzer's speeds, carried
  sideways by the wind field: rain leans with the gusts, and slants harder as one passes.
- **Streaks**: each drop drawn as its path over one exposure, thin, lit by the sun's shadow and
  the sky, but not traced through the GI volume (step 2's measurements showed that was most of
  the cost on integrated GPUs). Brightest against dark backgrounds and backlit, as real rain is.
- **Splashes** where drops hit the ground (crowns of droplets), and **rings** where they hit
  water, through the collision events that already exist.
- **Far rain**: beyond the particles' 25–50 m, rain as a few layers of scrolling streak texture
  on cylinders around the camera, leaning with the wind, depth-faded against the scene, as
  Tatarchuk's. Thousands of drops' worth of rain at the cost of a few full-screen layers.
- **Haze**: heavy rain thickens the air near the ground (shorter visibility, a grey lift in the
  aerial perspective), and **rain shafts** hang under the storm cells: the clouds' rain darkens
  the air beneath dark bases, visible from afar.

### 6. Rain on surfaces

Reusing the wet material from `docs/wetness.md`, with a second source of wetness.

- **Rain wetness**: a surface's wetness from rain rises with the rain's accumulated depth and
  dries afterwards (minutes, faster in wind and sun). Only where the sky reaches it, by the cover
  map: under an arch or a roof, the stone stays dry. Vertical faces wet less than upward ones,
  and wetter where water runs down them.
- **Puddles**: where the ground is near-flat and lies below its surroundings (from the cover
  map's heights, blurred, and a little noise so they aren't perfectly round), they fill as rain
  accumulates and shrink as it stops. A puddle is the wet material's film with the normal
  flattened, plus the ring ripples; the existing reflections show the sky and scene in it.
- **Ripples**: a shared texture of rings, 256² tiling over a metre, made each frame by a tiny
  compute pass while it rains (expanding, fading rings at random places and times). Puddles,
  ponds and the sea read it. Its density follows the rain rate.
- **Running water** on steep wet faces: faint darker streaks flowing downwards in the wet
  material's film, by a scrolling noise, only on faces facing the rain.
- **Drips**: a particle emitter that spawns at eaves, where the cover map steps down sharply,
  dripping at a rate set by the rain and for a while after it stops.
- **Foliage** gets wetter-looking (glossier, darker) through the same wetness. Leaves shed drops
  in gusts after rain, if time allows.

### 7. Lightning and thunder

Everything here costs nothing until a strike.

- **Strikes**: a seeded random process on the CPU. The weather's new `thunder` setting (strikes a
  minute, from Observation's codes 95–99 too) sets how often. Each flash has a place (mostly
  1–10 km away, occasionally near), a kind (in-cloud, more common, or to the ground), and 1–5
  return strokes 40–80 ms apart, each a brilliant spike then a decaying glow, measured shapes
  from Rakov and Uman.
- **The bolt**: a branching channel grown on the CPU from the cloud base to the ground (stepped
  leader: jagged segments, branches dying out), drawn as glowing HDR ribbons depth-tested into
  the scene and hazed by distance, so it blooms. Only the strokes' moments show it, with an
  afterimage fading over ~0.2 s.
- **Cloud flashes**: the clouds' march gains one light, inside the cloud at the flash, while it
  lasts: the cloud lights from within and glows around it. In-cloud flashes light only clouds.
- **The scene's light**: during a flash, a directional light from the bolt, very bright and
  bluish-white, plus a sky term (the whole cloud deck lit from below) through the existing
  ambient.
- **Its shadows**: at the flash's start, one 2048² shadow map from the bolt's direction over the
  ~150 m around the camera, drawn once through the same culling and rasterizing as a cascade
  and reused for all the flash's strokes. Sharp, because the channel is thin. Water and
  particles sample it as they do the sun's. On Low, no flash shadow map: the GI volume's
  distance field gives a soft one.
- **Exposure** is held during a flash, so it blinds and passes as real ones do, instead of the
  camera adjusting to it.
- **Thunder**: heard at distance / 343 m/s after the flash. A sharp crack for a near strike, a
  long low rumble for far ones (the rumble's length from the channel's spread of distances).
  Synthesised in the web page with the browser's Web Audio (filtered noise shaped by the strike):
  no sound files and no new dependency. Silent until the visitor turns sound on, as browsers
  require. The renderer only reports each strike (time, place, power) through the viewer.

## Gating: scenes without it pay nothing

- **Wind field pass**: dispatched only if something reads it this frame (a swaying instance in
  view, water, rain, particles with `wind`).
- **Sway in shaders**: behind the instance's flag, checked once per cluster, so workgroups take
  one branch. The pipelines are built with a WGSL `override` constant: a scene with no swaying
  material compiles the rasterizers and resolve without the wind code at all.
- **Tree simulation**: dispatched only if the scene has swaying skeletons in range.
- **Rain**: the cover map, drop emitter, far layers, ripple texture, puddles and drips all only
  while `rain > 0`, and while it is still drying afterwards for wetness. Resolve's rain
  wetness is behind the same kind of `override`, switched when rain begins.
- **Lightning**: nothing at all between strikes; the flash shadow map is made on a strike.
- **Tests** check that a scene without the features dispatches nothing new and that its image
  is unchanged.

## Budgets

At 1280×720 like the other plans, measured natively with the profiler and in Chrome by frame
time, on the RTX 5080 (High) and as a phone (Low, with PR #65's default-limit fallbacks). A
heavy storm in the Forest must keep the Galaxy S26 near its present ~40 fps.

| Effect | RTX 5080, High | Phone / integrated, Low |
|---|---|---|
| Wind field | 0.01 ms | 0.03 ms |
| Grass sway (raster + resolve), Forest | 0.15 ms | 0.5 ms (shorter range) |
| Tree bones + skinning + leaf flutter, Forest | 0.2 ms | 0.5 ms (shorter range) |
| Moving shadows of trees, near cascades | 0.3 ms | 0 (static) |
| Water: gusts, whitecaps, rain rings | 0.03 ms | 0.1 ms |
| Rain: drops, splashes, far layers, haze | 0.3 ms | 1.2 ms |
| Rain on surfaces: wetness, puddles, ripples | 0.05 ms | 0.2 ms |
| Cover map (amortised; a one-off redraw when far moved) | ~0 (0.3 one-off) | ~0 (1 one-off) |
| Lightning between strikes | 0 | 0 |
| Lightning during a flash (one-off shadow map + bolt) | 0.5 ms one frame | 0.1 ms (no map) |
| **Storm in the Forest, all of it** | **~1.1 ms** | **~2.5 ms** |

**Limits**: no new storage buffers in any pass that is near its limit; the wind field and
ripples are sampled textures. The bones are already a texture. Passes at 16 sampled textures
(water shading, as PR #65 found) must not gain one on default-limit devices: the ripple and
gust reads share a texture array there. Checked against WebGPU's default limits in tests.

## Steps

Each is its own PR, reviewed by a separate AI model, with screenshots or captures from the
browser, costs on the RTX 5080 in Chrome and as a phone, and the coordinator told when it is
ready. Tests are mutation-checked as the particle steps were.

1. **The wind field and grass.** *Built; in review.*
   - `wind.rs` with `common/wind.wgsl` (the field, CPU and GPU twins), `wind/field.wgsl` (the
     pass filling a 128² field, 2 m texels, this frame's and last frame's wind, with a row of
     parameters below it), `common/sway.wgsl` and `common/sway_vertex.wgsl` (the bend).
   - `gusts=` and `turbulence=` on the `wind` line of weather files; every built-in weather has
     its own, the storm gustiest. `Weather::observed` sets them from the wind and the weather
     code.
   - `Material::SWAYS` (flag 8) and `INSTANCE_SWAYS`. The Forest's grass and the weeds in it
     carry it.
   - Grass bends where vertices are placed: both rasterizers (`place_vertex`, only in views
     through a camera), resolve (positions, normals turned with the blade, and last frame's bend
     for motion vectors), and culling (spheres grown by `GRASS_REACH` of their top's height).
     Views kept from frame to frame, the sun's cascades, the seabed and GI voxels, see the grass
     at rest.
   - A scene with nothing that sways compiles `SWAYING` out of every shader and never runs the
     field pass; its shaders are otherwise the same.
   - Swaying fades out with distance: 40–60 m on High, 30–45 m on Medium, 20–30 m on Low.
   - The `wind` debug view (number 15), and a profiler section, `wind`.
   - Particles still carry their own gusts. They move to the shared field with the rain (step 4),
     which needs the field bound to the particle passes.
   - Tests:
     - Still air stays still; a steady wind is its mean everywhere; turbulence turns it from
       place to place, within its limit.
     - The mean follows the weather at half its speed, turning and swelling only within limits.
     - The air carries everything at its mean speed, and gusts are carried downwind with it.
     - Gusty weather gusts to 1.3–2.4 times the mean, with lulls below 0.6; a steady day less.
     - With no time passing, nothing moves; the first frame has no motion.
     - The GPU's field is the CPU's at every texel, now and a frame ago.
     - A blade, run through the bending shader itself, keeps its length to 0.2%, leans downwind,
       never further than culling allows, its root staying put; the rasterizers' place is
       resolve's; up turns downwind.
     - A stronger wind bends it further; a gale lays it over; a scaled instance bends by its
       height in the world; last frame's bend is last frame's wind's.
     - Only what is flagged and near moves; nothing does with swaying off.
     - Rendered, a flagged post leans downwind with its foot in place, writes motion where the
       floor writes none, and the same post unflagged stands still even in a gale.
   - Seen in: the Forest (re-baked with the flag).
2. **Trees.** *Built; in review.* **A change from the plan: no skeletons.** Measured against
   what the Forest needs, skeletons would have meant uploading every tree's bones each frame,
   skinning each leaf vertex through up to four bones, and treating all its trees as moving,
   so every shadow cascade redrew them each frame. Instead trees sway as grass does, where
   their vertices are placed, from the same wind, read at the trunk so a whole tree moves
   together (`common/sway_vertex.wgsl`, `tree_offset`):
   - the trunk leans with the wind's drag, following the gusts and saturating in a gale, and
     sways about that at its own slow rate, 0.25–0.37 Hz, mostly along the wind and a fifth as
     much across it, in step, so the crown rocks rather than circling; its offset grows with
     the square of the height, and the top comes down as it leans, so it bends rather than
     shears;
   - branches are pushed downwind by their reach from the trunk, dipping as they go, and swing
     a little behind the trunk, 0.4–0.55 Hz, each part of the crown slightly later than the
     next. A branch's phase depends only on which way it points, never on its height, so every
     branch moves whole. An earlier version phased branches by height too, and swung them up
     and down at another rate than along the wind: branches rippled and tips traced loops, and
     in the Cove trees looked to dance. The sway and the push grow with the drag up to a fresh
     wind and stop growing at 12 m/s;
   - leaves flutter, fastest, each spray its own way;
   - a tree's sway fades out at 60–90 m on High; its branches' bob and its leaves' flutter,
     being small, are worked out only nearer.
   - `Material::TREE` (16) with `SWAYS`, and `INSTANCE_TREE` and `INSTANCE_FLUTTERS` for
     leaves; the material mask is now 24 bits, and 23 since `INSTANCE_BLENDS` (land.rs). The Forest's trees and bushes carry it.
   - Culling grows a tree's spheres by 0.1 of their top's height plus 0.6 m, but sizes a
     swaying cluster by its own sphere when choosing a rasterizer. Sized by the grown sphere,
     the wood's tiny leaf clusters left the software rasterizer for the hardware one, which
     cost 0.7 ms on the RTX 5080.
   - Trees cast their shadows at rest, as grass does; dappled light under them stays still.
   - Tests, through the bending shader: a tree's foot stays put and its top leans downwind,
     further than halfway up; leaves stay on the wood they grow on; a branch's tip moves more
     than its base; nothing goes beyond what culling allows; a far tree and a calm one stand
     still.
3. **Water.** Cat's paws and whitecaps on the sea, wind-driven pond ripples with fetch.
   - Seen in: the Cove; ponds in the Zorah and harbour scenes.
4. **Rain**, in the air and on surfaces, built together as one step. *Built; in review.* You
   asked that it be done first and that it feel real and wet, so it came before trees and water.
   - **A change from the plan: no particle simulation for rain.** The particle steps measured
     about 2 ms for heavy rain on an integrated GPU. Here every drop's place follows from a
     hash and the clock (`rain/draw.wgsl`): a lattice of drops is carried downwind by the air,
     each falling through it at its terminal speed, round a column around the camera. Rain
     costs only its drawing, and a drop stays put however the camera moves. Splashes and drips
     are drawn the same way.
   - **Three rings of drops**, coarser outward, to 120 m and more. Each outer streak stands for
     more drops, so it is drawn wider, stronger and a little longer, and the rain keeps its
     density on screen into the distance.
   - **Streaks** are a drop's path over about 1/45 s, whatever the frame rate; at a frame's
     own length they shrank to ticks on fast displays. They lean with the mean wind that also
     carries them, so a streak points along its drop's motion.
   - **Lit** from a small light volume around the camera, worked out once a frame: the sky,
     the sun through its cascaded shadows and the clouds, scattered onward by the drop's phase
     function, and the scene's lamps.
   - **The cover map**: the scene from above, 512² over 128 m, through the shared culling and
     rasterizing, redrawn only when the camera moves 16 m or geometry streams in. The rain map
     made from it holds the first surface's height, the depth a puddle can gather (the ground
     below the ground around it, where level, ignoring what stands on it), and the eaves
     beside each spot, which drip.
   - **On surfaces** (`rain/surface.wgsl`), through the sea's wet material:
     - wet only where the sky reaches, read through a 4×4 tent so the edge of a shelter is
       soft, not a row of squares; walls read their shelter a little out from them;
     - walls wet less, more on the windward side, with water running down in streaks; faces
       turned down not at all; sheltered surfaces grow damp only in a long rain;
     - porous ground, sand and soil, darkens with only a sheen, as rain drains through it;
       smooth stone and paint take a full film;
     - puddles fill hollows of level ground over minutes and drain over a quarter of an hour,
       their edges wandering; in them the surface is a sheet of water, ringed by the drops.
   - **On water**: rings on the sea and ponds, worked out where they are seen (the sea's pass
     already binds as many textures as phones allow), and heavy rain dulling the water's
     glitter.
   - **In the air**: heavy rain thickens the haze, so distance greys.
   - `RendererSettings::rain` by quality; profiler sections `rain` and `rain draw`; the viewer's
     `--weathered SECONDS` runs the weather on, so a capture can show a soaked scene.
   - Tests: Marshall and Palmer's drop counts at 10 mm/h; Gunn and Kinzer's speeds within 4%;
     surfaces soak in a minute of downpour and puddles more slowly; both dry after, faster in
     wind; the cover map lies under the camera; the uniforms match their shaders; rendered, a
     roof keeps the floor under it dry while the open floor soaks, and no rain falls under it.
5. **Lightning and thunder.** *Built; in review.*
   - **Strikes** (`lightning.rs`): a Poisson process at the weather's new `thunder` rate, in
     strikes a minute (the storm has 3; `Weather::observed` gives 2 for code 95, 4 for 96–99).
     A third reach the ground with a bolt, the rest light the cloud. Flashes fall 0.6–9 km
     off. To the ground: one to five return strokes 30–90 ms apart, each a spike decaying over
     about 45 ms, the last with a continuing current glowing for a quarter of a second.
   - **The light** goes to every lighting pass through the clouds' uniform (`flash`):
     - the sky and clouds glow towards the flash and a little everywhere, through
       `sky_radiance`, so reflections, water, rain and the GI see it too;
     - surfaces facing it are lit directly, shadowed by a trace through the GI volume's distance
       field, with the lit cloud deck's light from above; the trace runs only during a flash;
     - a flash is never brighter than twenty times the light already about, so by night it lights
       the scene brilliantly, not to a white sheet, and by day it is a sharp flicker.
   - **Exposure holds** while a flash lasts, as an eye stays dazzled.
   - **The bolt** (`lightning/bolt.wgsl`): a jagged channel from the cloud base to the ground
     with branches dying out, a white-hot core in a glow, blooming, hidden by what stands in
     front, dimmed by the air.
   - **Not done from the plan:** a shadow map from the bolt's direction. The distance-field
     trace gives the same hard edge at the GI volume's resolution, costs nothing between
     flashes, and works on phones; the clouds' own march is not lit from within either: the
     flash's glow is laid over the clouds' light instead.
   - **Thunder**, made in the page from filtered noise, no sound files: heard after the flash at
     its distance over 343 m/s; a near bolt cracks then rumbles, a far one only rolls, longer.
     Silent until "Sound (thunder)" is ticked under Extras. **L** calls a strike down ahead.
   - The viewer's `--strike-at FRAME` makes one strike during a capture.
   - Tests: no thunder, no lightning; strikes come about as often as asked; thunder follows
     the flash by its distance at the speed of sound, far thunder longer and quieter; a flash
     to the ground is one to five strokes 30–90 ms apart, each flashing and fading, its bolt
     running from the cloud base to the ground; a nearer flash lights more, by the square of
     the distance, but never beyond its cap; a strike can be asked for; weather files read and
     write `thunder`.
6. **Showing it.** README, captures in the Cove, Forest and Temple, costs, viewer controls
   (wind strength, rain, a "strike now" key).

## Decisions

Taken on 12 November 2026, when you approved the plan with the recommendations below as they
stand: skeletons for trees, the new weather settings, the `SWAYS` flag, re-baking (each publish
still waits for you), plants for the Cove and Temple, synthesised thunder that is off until
turned on, the order of steps as listed, and the budgets as proposed.

1. **Trees as simulated skeletons**, using the existing skinned format (recommended), or a new
   per-vertex pivot stream (a file-format change) for the same look.
2. **New weather settings**: `gusts`, `turbulence` and `thunder` in `.weather` files and in the
   weather the page and gameplay read. A weather-format change.
3. **The `SWAYS` material flag**: a new bit in the existing material flags, set by bakes. Old
   files without it never sway. Alternatively, every `FOLIAGE` material sways, with no new flag,
   but then anything foliage-flagged that shouldn't move (ivy on a wall, a card) would bend.
4. **Re-baking and publishing the Forest** (and the circuits) with swaying grass and tree
   skeletons, and later the Cove and Temple with rain-ready scenes. Each publish waits for you.
5. **Plants for the Cove and Temple.** Neither has any, so wind shows there only on the sea,
   in the rain and through lightning. I'd add grass and a few shrubs on the Cove's headland and
   weeds and a tree among the Temple's ruins, so all three show wind in plants. A scene change.
6. **Thunder sound** in the page through Web Audio, synthesised, off until the visitor turns it
   on. Or no sound at all, if you'd rather the engine stays silent for now.
7. **Order of the steps**: as above (wind and grass first). Rain first would show the most
   change in the Cove and Temple soonest.
8. **Budgets**: about 1.1 ms for a full storm on the RTX 5080 and 2.5 ms on a phone. Tighter, or
   more held back on Low?

## Risks

- **Moving shadows are the biggest cost.** If redrawing near trees each frame costs more than
  budgeted, the near cascade alone takes them, or sway in shadows is limited to the trunks.
- **Skinned trees and the DAG.** Skin weights already pass through simplification, but coarse
  clusters of a whole crown blend many bones; the 16-bone cluster limit falls back to the whole
  mesh's reach for bounds, which is looser culling for distant crowns. Measured on the Forest.
- **TAA and fine motion.** Fluttering leaves and blades move by under a pixel a frame; correct
  motion vectors keep them from smearing, but thin, fast flutter can still ghost or shimmer.
  The reactive mask is the fallback.
- **The SW rasterizer's cost per vertex** grows with the wind code. It runs on tiny triangles,
  where grass mostly is. The `override` keeps scenes without it free; Forest's cost is measured.
- **Binding limits on phones**, as above.
- **The cover map draws through shared culling** that other sessions are changing (Zorah,
  streaming). It lands as its own PR and coordinates with them.
- **Sound in browsers** waits for a click and differs between browsers; thunder must never be
  the only sign of a strike.
