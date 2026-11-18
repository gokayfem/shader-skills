# Wetness: design

Surfaces the sea reaches get wet: the sand along the waterline, rocks standing in the surf, the
ship's hull. They stay wet a while after the water draws back, then dry. This document is the
plan, agreed before any of it was built, and kept up to date as each step lands. Foam waits until
later.

## Goals

- **Real wet materials, not a tint.** Wet sand turns darker and slightly glossy. Wet rock darkens
  less but shines. A painted or metal hull only gains a film of water. Each follows from how water
  fills a surface's pores and then covers it.
- **Driven by the water that is really there.** Wetness comes from the waves the sea is drawing
  this moment, shrunk over the seabed as the water surface is, and from where they reached in the
  last minute or so. Nothing is painted by hand for one scene.
- **It joins the whole system.** It is decided once, where each pixel's material is resolved.
  Lighting, sun and moon shadows, reflections and screen GI then all see the wet surface. GI's
  voxels see it too, so wet sand bounces darker light (from step 3; until then they stay dry).
- **Dynamic.** A rougher sea wets higher. A calmer one leaves the wet band to dry. Moving the sea
  level moves all of it.
- **Cheap.** Under 0.1 ms on the RTX 5080 at 1280×720, and modest on integrated GPUs.

## What there was before

- No wetness anywhere.
- The resolve pass (`vg/resolve.wgsl`) works out each pixel's albedo, roughness, metallic and
  normal and writes them to the G-buffer. Everything lit reads them from there.
- The seabed map (`ocean/seabed.rs`): a top-down depth map of the scene around the camera, which
  says how deep the calm water is at any point.
- The waves' displacement on the GPU, and `shoal()`, which shrinks them in the shallows.
- The weather has a `rain` value. Rain is out of scope here; see the last step.

## References

- Lekner and Dorf 1988, *Why some things are darker when wet*. Water in the pores lowers the
  index contrast, so light scatters forward and is absorbed more. The darkening is strongest for
  bright, rough, porous materials.
- Lagarde 2013, *Water drop 2: wet surfaces* (also used in Unreal-era games). It describes two
  stages. First, pores fill: albedo darkens and roughness falls a little. Second, a film forms:
  roughness drops to about 0.05–0.1 and fine bumps drown under the water's flat surface.
- Stockdon et al. 2006, *Empirical parameterization of setup, swash and runup*. How far waves run
  up a beach, from the wave height, wavelength and beach slope.

## The wet material

Wetness `w` runs from 0 (dry) to 1 (soaked, under a film). It is applied in the resolve pass,
just before the G-buffer is written.

- **Porosity.** Materials have no porosity value yet, so it is derived:
  `(1 - metallic) × smoothstep(0.3, 0.8, roughness)`. Sand and rough stone come out porous,
  painted wood and polished stone much less, and metal not at all. A per-material `porosity`
  value can come later through the scene file, if any material is judged wrong.
- **Pores filling**, for `w` up to about 0.6:
  - albedo darkens towards albedo^1.6 for porous materials, blended by porosity: between dry
    (1) and the limit Lekner and Dorf give for highly porous ones (2), which puts sand of albedo
    0.35 at about half, as Twomey et al. measured;
  - roughness falls by about a third.
- **A film forming**, for `w` from about 0.6 to 1:
  - roughness goes to 0.07;
  - on ground facing up, the normal map's bumps flatten towards the surface's own normal, as
    water fills the hollows; on steep faces the film runs off thin and follows them;
  - F0 is left as it is. A thin film over a dielectric reflects about as water does anyway, and
    over metal the metal still shows through.
- **Cavity.** Cavities stay wet longest, so they reach their wettest first.
- **Under the water** there is no film. A film's gloss comes from its own surface against the
  air, and under the sea the sea's surface gives the reflection instead. The pores still fill, so
  the seabed is darkened but not glossy.
- **Debug view `wetness`**, to see `w` on its own: how wet the material is taken to be,
  cavities included.

## Where the water has been: the water-line map

The sea's surface is only drawn where it is. Wetness needs to know where the water has *been*.
One small map, held from frame to frame, remembers it.

- **Texels:** 512² over 128 m around the camera, so 25 cm each (`ocean/waterline.rs`). Each holds
  two heights: the highest the water has stood at that spot lately, and where it stands now.
  - They are heights in the world, not above the calm sea. When the sea falls, as a tide going
    out, the line it reached stays behind and dries.
  - Two Rg32Float maps take turns, one read while the other is written. Rg16Float would do, but it
    cannot be written by a compute pass in core WebGPU.
- **Fixed in the world.** The map wraps around rather than moving with the camera, so a remembered
  wet band stays put as the camera moves. Texels that come newly into view start out at today's
  level.
- **Updated each frame** by a small compute pass:
  1. The water's height at that spot, from the waves, shrunk over the seabed by `shoal()` exactly
     as the surface is. The drawn surface carries each point sideways by the waves' sway, so the
     height is read from where the water came from, by one step of that correction.
  2. What the water reaches there: the highest of the water at that spot and at the four beside
     it. A texel holding a rock's face or a sea wall stands over ground above the sea, where no
     wave stands, so without this its face would be wet only to the calm level; with it, the
     waves beside the face wet it as high as they stand. Each workgroup works out the heights of
     its texels and the ring around them once, in shared memory.
  3. Run-up on beaches (step 3): waves rush up the slope beyond where the water itself stands, to
     a height from Stockdon's formula, using the sea's significant wave height and the beach slope
     from the seabed map. Without this, the wet strip of sand above the calm waterline is only as
     wide as the fringe.
  4. The remembered level becomes the higher of the new height and the old memory. The memory
     falls back towards the current water as it dries, with a half-life of about 30 s.
- **Reading it in resolve,** for a surface point `p`:
  - fully wet below the remembered level;
  - a soft fringe above it, as water wicks up through the pores: 3 cm on a surface without pores,
    up to 20 cm on the most porous (step 3 tunes these);
  - drier the longer it has been since the water was there, as the remembered level falls back;
  - under the water now, by the present height: darkened, but with no film.
- **Beyond the map**, only the calm level and the fringe count: no memory is needed that far
  away. The map is a square, reaching 64 m from the camera along the axes and about 90 m on the
  diagonals. Over its last 8 m what it holds fades into the calm level, so the wet line has no
  edge where it ends.
- **Cost** at 1280×720 on the RTX 5080: 0.02 ms for the update (its own profiler section, "wet
  line"), and 0.01 ms more in resolve.
- **Hollows inland** that lie below the sea's level read as sea in the seabed map, which has no
  idea whether they connect to it. They are wet and under water, just as the drawn water fills
  them. Cut-off water is for later.

This covers vertical faces too: a rock or hull gets wet up to the highest the water beside it
has stood. Points under the water are wet anyway, and the water pass draws over them.

## Steps

Each step is its own PR, reviewed by a separate AI agent, and merged only with your go-ahead.

1. **The wet material.** The shading model in resolve, porosity, and the `wetness` debug view. It
   is driven for now by a fixed band, from the calm sea level up to 30 cm above it, so the look
   can be judged on its own: sand, rock, the hull, metal.
   - Tests: wet sand comes out darker than dry and smoother; metal does not darken.
2. **The water-line map.** The persistent map, its update from the waves and the seabed, drying,
   and reading it in resolve. The fixed band is removed.
   - Tests: a wave's wet band stays after it draws back and dries out later; nothing high above
     the sea is ever wet.
3. **Run-up and wicking.** Stockdon run-up on beaches and the fringe by porosity, tuned against
   photos of real beaches and rocky shores. GI's voxels see wetness too.
4. **README, screenshots, costs**, measured on the RTX 5080 and the integrated GPU tier.

Rain is left for later: wetness from the weather's `rain`, sheltered under overhangs, with
puddles in hollows. It would reuse the same wet material from step 1.

## Risks

- **Derived porosity may misjudge some materials,** for example a smooth but porous plaster. The
  fix would be a per-material value.
- **Wetness only changes the G-buffer.** Anything that skips resolve (sky, clouds, foliage cards)
  never gets wet, and none of those should.
- **Wet bands will be soft-edged.** The map's 25 cm texels limit how sharp a wet edge can be on
  flat ground. Vertical faces are exact, because height comes from the surface point itself.
- **Run-up from a formula is an average.** Individual waves vary, so the wet band's top edge will
  wander less than on a real beach.
