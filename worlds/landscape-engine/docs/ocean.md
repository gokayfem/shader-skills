# Ocean: design

A design for open water: waves that move with the weather, water that shows what lies beneath
it, and a surface that takes part in everything else the renderer does. To be built slowly, one
measured step at a time.

## Goals

- **Real.** Waves follow a measured ocean spectrum, driven by the wind. Light reflects and
  refracts as it does at a water surface, and water absorbs and scatters with depth.
  - A calm morning is a glassy mirror with a long swell.
  - A storm is a heaving grey sea streaked with whitecaps.
  - Shallows over sand are clear and turquoise.
- **Fast.** At most about 0.8 ms at 1080p on a desktop GPU and 1.5 ms on integrated graphics,
  with quality tiers. Each step is measured, not guessed.
- **Dynamic.** Every property can change at any time and eases into the new value, as the weather
  does: wind speed and direction, swell, water colour and clarity, foam, and choppiness.
- **Part of the whole system.** The ocean must work with everything else in the renderer:
  - It answers to the weather and the time of day.
  - It takes the sun's and clouds' shadows, and reflects the scene, sky, clouds, moon and stars.
  - It lights the scene through GI, fades into the haze, and meets the sky at the horizon without
    a seam.
  - It holds steady under temporal anti-aliasing.

Not in the first version:
- an underwater camera;
- wakes and objects disturbing the water;
- floating objects;
- rivers and lakes with currents.

The design leaves room for each.

## What there is today

The bake's `water` command adds two flat planes: a 4 km one that borrows the sand's ripples as
waves, and an 80 km one beyond it. They go through the visibility buffer like any other mesh, so
the water is opaque. It has no idea of the seabed below it, and it never moves.

## References

- Tessendorf, *Simulating Ocean Water* (2001): waves as a sum of many sinusoids, from a spectrum,
  summed by FFT, with horizontal displacement for sharp crests.
- Elfouhaily, Chapron, Katsaros and Vandemark, *A unified directional spectrum for long and short
  wind-driven waves* (1997): the wind sea, from the long waves the fetch allows to the ripples the
  wind's friction raises.
- Hasselmann et al. (1973), the JONSWAP spectrum, for the swell, and Horvath, *Empirical
  Directional Wave Spectra for Computer Graphics* (DigiPro 2015), for Donelan and Banner's
  directional spreading.
- Cox and Munk (1954), the slopes of the sea measured from sun glitter; Monahan and O'Muircheartaigh
  (1980), how much of the sea is whitecaps at each wind speed. Both are targets for the tests.
- Bruneton, Neyret and Holzschuch, *Real-time Realistic Ocean Lighting using Seamless Transitions
  from Geometry to BRDF* (2010): waves too small to see become surface roughness, so distant water
  neither shimmers nor turns too dark, and glitter spreads correctly towards the horizon.
- Unreal Engine's Single Layer Water shading model. Water is shaded after the opaque scene, over
  what lies behind it, with absorption and scattering by depth.
- *The Technical Art of Sea of Thieves* (GDC 2018) and Atlas's interactive water (GDC 2019):
  foam, light through wave crests, and how a game ocean holds up in motion.

## Data and API

Like trees, grass and weather, an ocean is a short text file, so it can be changed without
touching code:

```text
# crates/solstice-render/ocean/clear.ocean
name clear
absorption 0.45 0.07 0.04    # per metre, red is absorbed fastest
scattering 0.002 0.006 0.007 # per metre: blue-green glow from within
turbidity 0.0                # 0 clear, 1 murky
foam 1.0                     # how readily whitecaps form
choppiness 0.85              # how sharp crests are
swell 0.6 11 250             # height (m), period (s), from (degrees)
```

- The **wind** comes from the weather. The weather's wind is at cloud height, and is brought down
  to 10 m with the usual log profile. When the weather changes, the sea follows over minutes, not
  seconds, as a real sea builds and calms.
- **Presets:**
  - *clear*: tropical shallows;
  - *coastal*: the default, for the cove;
  - *ocean*: deep Atlantic;
  - *murky*: a harbour.
- **Scenes:** a scene asset records its sea level and ocean preset. The bake's `water` command
  becomes `ocean HEIGHT PRESET` and stops adding meshes.
- **Library:**

  ```rust
  renderer.set_ocean(Ocean::by_name("ocean").unwrap(), 60.0);
  ```

  This is eased over 60 seconds, as with `set_weather`. `Ocean` is public, with every field
  documented.
- **Viewers:**
  - The web page gets **Sea**, which by default follows the weather.
  - **Extras** gets sliders for wave height and water clarity.
  - The native viewer gets `--ocean NAME`.

## Waves

**Spectrum.** Elfouhaily's spectrum for the wind sea, with fetch, and Donelan and Banner's
directional spreading, plus a narrow JONSWAP swell from its own direction. The wind sea and the
swell can come from different directions, as at a real coast. Calm water then keeps a long swell,
and a storm's waves run with the wind.

JONSWAP alone was the first choice and was tried. It gives the right wave heights, but above its
peak its tail saturates at a level that hardly depends on the wind, so a breeze and a gale look
equally rough close up. Elfouhaily's short waves grow with the wind's friction on the water, as
real ripples do, and its wave heights still match measured seas. A fully developed sea is within
5% of Pierson and Moskowitz, and fetch-limited seas are within 25% of JONSWAP's growth law.

**Cascades.** Three tiles, 1000 m, 128 m and 17 m across. Their ratios are not whole numbers, so
no repeat lines up. Each carries only the wavelengths the others don't, so none is counted twice:
the three together hold 98–100% of the spectrum's energy at both resolutions. The first is large
enough for a long swell.

| Tier | Resolution per cascade |
|---|---|
| High | 256² |
| Medium | 256² |
| Low | 128² |

**Per frame, on the GPU:**
1. Advance each wave's phase by the deep-water dispersion relation, ω² = gk.
2. Run an inverse FFT, Stockham, in workgroup memory: one dispatch per axis per cascade, six in
   all. Four complex channels are packed per cascade:
   - height and the two horizontal displacements;
   - the slope and Jacobian terms for normals and foam.

   That is 8 KB of the 16 KB WebGPU allows.
3. Write each cascade into a mipmapped texture array:
   - displacement;
   - normal derivatives;
   - the Jacobian;
   - persistent foam, which grows where waves fold and fades over seconds.

The initial spectrum is only rebuilt when the wind or ocean changes, at a low rate while easing.
Phases are kept within one period per frequency, so precision does not decay over a long session.

**Filtering.** Every texture is mipmapped. Wave slopes finer than a pixel's footprint are added
to the surface's roughness from the spectrum's own slope variance (Bruneton 2010). This removes
shimmer and aliasing at distance, and keeps the far sea's brightness and the sun's glitter path
correct all the way to the horizon.

## Geometry

**The grid.** A camera-centred grid of concentric rings, each twice as coarse as the last. Each
ring is snapped to its own texel grid, so waves don't swim as the camera moves, and blends into
the next so there is no popping. That is about 100,000 triangles in all, out to around 10 km.
Beyond that, displacement is too small to see, and the surface is a flat plane carried to the
true horizon, shaded exactly as the near water.

**The water layer.** The grid is drawn by the hardware rasterizer into its own layer:
- a depth target, so waves hide each other correctly;
- one attribute target holding the undisplaced position, from which everything else is looked up.

The opaque scene is untouched. The visibility buffer, G-buffer and lighting run as now, and the
seabed, rocks and the ship's hull are lit as usual, under water or not.

**The seabed map.** Waves feel the bottom: as the water shoals they stand up and break, and no
crest stands taller than about 0.78 of the depth beneath it (McCowan). Over dry land there are no
waves at all. A depth map of the scene seen from straight above, 1024² over 1 km around the
camera, tells the water how deep it is everywhere (`ocean/seabed.rs`). It is drawn by the shadow
maps' depth-only pass through the same cluster culling, from 2 m above the sea down 64 m, so
decks and cliff tops are cut away. The scene is static, so the map is redrawn only when the camera
has moved 128 m, the sea's level has changed, or finer geometry has streamed in.

The waves' height is squeezed smoothly within 0.4 of the depth, by `tanh`, and their sideways sway
fades in the last 2 m; the water pass scales their slopes to match. So:
- the sea meets the shore along its calm waterline, and does not cut through the sand wherever a
  crest happens to rise;
- the sea never surfaces in hollows inland, however low the land lies;
- deep water is untouched.

Shore foam (step 4) reads the same map.

## Shading: the water pass

A compute pass after deferred lighting and before post. On each pixel where the water is nearer
than the opaque scene, it works as follows.

1. **Surface.** It samples the cascades at the right mip for the pixel's footprint, for the normal,
   foam and Jacobian, and turns unresolved slope variance into roughness.
2. **Reflection.** Fresnel for water (n = 1.33) weights a reflected ray, which goes in turn:
   1. through screen space, against the opaque scene, reusing the existing hierarchical trace;
   2. through the GI volume;
   3. to the sky, clouds included, with the sun's and moon's glitter from their discs.

   The ray stays above the horizon, so sea and sky meet without a seam, as they already do.
3. **Direct light.** GGX specular from the sun or moon at the wave's roughness, taking:
   - the cascaded sun shadows at the water's surface;
   - contact shadows;
   - the clouds' shadows.

   The shadow code moves from `deferred.wgsl` into a shared include for this.
4. **Through the water.** It refracts the view ray and reads the lit scene behind it. A depth test
   stops it from picking up things in front of the water. The water's depth along that ray then
   sets:
   - how much of the seabed's light survives, by Beer-Lambert absorption;
   - how much light the water scatters itself, lit by the sky and by sun refracted into it.

   Shallows come out clear and turquoise, and deep water dark blue-green. Only one layer of water
   is shaded, as in Unreal.
5. **Light through wave crests.** Light scatters forward through the thin top of a wave towards the
   viewer, when the sun is behind it. This is single scattering, from the wave's height above the
   mean and the angle between sun and view.
6. **Foam.**
   - Whitecaps form where waves fold (Jacobian below one) and persist as they fade. The fold is
     judged from all three cascades together, not each alone: in a gale nearly all the energy is
     in the largest cascade, whose long waves rarely fold by themselves. How readily foam forms is
     calibrated so that the share of sea under whitecaps follows Monahan's measurements, about 1%
     at 10 m/s and 5% at 16 m/s.
   - Shore foam forms where the water is shallow.
   - Foam is a rough, white diffuse layer, lit by the sun with its shadows and by the GI volume's
     light.
7. **Haze.** The same aerial perspective as the rest of the scene, from one shared function.
8. **Outputs.**
   - The composited colour, into the HDR image, so exposure, bloom and tone mapping see it.
   - Its depth and motion into the G-buffer, so TAA, haze and everything after treat the water as
     the surface.
   - Motion vectors follow the moving waves, from last frame's displacement, so waves don't ghost
     under TAA.

On the Low tier, the reflection trace runs at half resolution and is upsampled by depth and
normal.

## How it joins everything else

| System | With the ocean |
|---|---|
| Weather | Wind drives the spectrum, storms raise whitecaps, and the sea builds and calms over minutes. Later, rain roughens and rings the surface (the wetness work). |
| Time of day | The sun's glitter path, the moon's at night, and sunset and twilight colours in reflection. |
| Clouds | Reflected, and their shadows dim the sea and its glitter. |
| Shadows | Cliffs, the fort and the ship shade the water. |
| GI | A flat water surface at sea level is voxelized, so light bounces off the sea onto cliffs and hulls, and rays from the scene that hit the sea see its colour. |
| Reflections | Rays from the scene reaching the sea see water; the water's own reflections see the scene. |
| Haze and horizon | One aerial perspective, and below-horizon views beyond the scene shaded as distant sea, not a fixed colour. |
| TAA and post | True depth and motion vectors; the sun's glints go into bloom. |
| Debug views | Water only, each wave cascade, foam, and water depth. |

## Steps

Each step is its own pull request, with renders, before-and-after measurements and tests, and none
starts until the last is right. Steps 1 and 2 touch nothing in lighting, so they can begin while the
lighting flicker investigation finishes.

1. **Waves.** *Done.* Spectrum, cascades and FFT. Tests:
   - The GPU draws exactly the waves the Rust reference does.
   - The transform matches a direct sum over every wave.
   - The height's variance is the waves' energy (Parseval).
   - The cascades together carry the whole spectrum.
   - Wave heights match Pierson and Moskowitz and JONSWAP's growth law.
   - Drawn slopes stay within Cox and Munk.
   - The sea follows a change of wind over about a minute, whatever the frame rate.

   About 0.03 ms a frame at 256² on an RTX 5080, and 0.02 ms at 128². A picture test draws calm,
   fresh and gale seas for review. The on-screen debug view comes with step 2, which gives the
   waves a surface to show on.
2. **Surface.** *Done.* The grid, the water layer and the bake's `ocean` command, replacing the
   planes; the cove is rebaked. Simple shading at first. Tests:
   - no cracks;
   - no popping or swimming as the camera moves;
   - the water meets the land correctly.

   How, decided from the code:
   - **Scene files.** The manifest gains optional tagged sections after its fixed part: a
     four-byte tag, a length, then the contents. The first is the sea: its level, and the
     ocean's own text, so a scene can carry an ocean of its own.
     - Readers skip tags they don't know, and older readers never look past the fixed part, so
       files stay readable both ways with no change of version.
     - The temple and forest keep working unbaked; only the cove is rebaked.
   - **Wave mips.** Each frame, the displacement and slope textures get their mip chains, so
     geometry and pixels far away sample waves as large as they can show, not alias smaller ones.
   - **The grid.** Concentric rings were the first plan. But a ring snapped to its own spacing
     no longer lines up with the ring inside it, by up to a cell, and filling that takes
     per-frame trim strips. Strugar's CDLOD (2010) avoids this:
     - A quadtree of patches fixed in the world, finer near the camera, chosen each frame by
       distance. Vertices never slide over the waves.
     - Each vertex blends onto the coarser grid by its own distance, so neighbouring levels meet
       exactly, with no cracks and no popping.
     - Fourteen levels, from 4 m patches with 12.5 cm cells to 32 km, out to ±48 km.
     - Patches are 32 cells across, 16 on Low. That is under 700,000 triangles at the surface
       on desktop GPUs, and under 180,000 on Low.

     Tests check the tiling, that every shared edge holds the same points (and that an unsafe
     range would crack), and that vertices stay put as the camera moves.
   - **The water layer.** The grid is drawn by the hardware rasterizer into its own depth target,
     with the undisplaced position of each pixel's water in a colour target.
   - **The water pass.** A compute pass after deferred lighting. Where the water is nearer than
     the opaque scene, it shades it: Fresnel reflection of the sky, sun specular, deep-water
     colour, and the same haze as everything else. It writes the result into a new HDR target,
     which post then reads, and the water's depth and motion into the G-buffer. Opaque light
     stays untouched in the old HDR target, ready for refraction in step 3.
   - **Debug view.** "Ocean": the water's normals, so the waves, cracks and coverage can be
     checked on screen and by tests. GPU tests check that every pixel below the horizon is sea,
     from low, high and distant cameras at every quality, and that what stands out of the water
     is not covered by it.
   - **Cost**, on an RTX 5080 at 1280×720: waves 0.05 ms, surface and water pass 0.06 ms.
3. **Water shading.** Reflection, refraction, absorption and scattering, sun and moon specular with
   shadows, filtering to roughness, and the horizon. Compared against photographs, one condition at
   a time: calm, choppy, storm, shallows, sunset, night.
4. **Foam and light through crests.**
5. **The whole system.** GI water surface, motion vectors, weather coupling and easing, presets,
   viewer controls and debug views.
6. **README, screenshots and GPU tests.** The tests check:
   - no seam at the horizon;
   - a storm sea is rougher and whiter than a calm one;
   - shallows are brighter than deep water;
   - the sea holds steady under a still camera.

Wetness follows as its own design: rain darkening and glossing surfaces, puddles, the wet band
along the shore, and rain on the sea.

## Risks

- **Moving water and TAA.** Fast normals can ghost or smear. Motion vectors from the waves are the
  first defence; if that is not enough, the water pass marks itself so TAA trusts history less.
- **Precision.** Wave phases in f32 over long sessions, which is why they are kept within one
  period, and depth precision far out on the plane, under reversed-Z.
- **WebGPU limits.**
  - The water pass writes three storage textures (colour, depth and motion), within the 5 the
    renderer already requires.
  - The FFT uses 8 KB of workgroup memory.
- **Shared code.** The water pass reuses the reflection and shadow code that the flicker
  investigation may change, so step 3 waits for its findings.
