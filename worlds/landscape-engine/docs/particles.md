# Particles: design

Rain that streaks past and splashes on the flagstones, spray thrown off the rocks, smoke from a
chimney leaning with the wind, sparks off a forge. This document is the plan for a particle
system in the spirit of Unreal's Niagara, agreed before any of it was built, and kept up to date
as each step lands. Modules and syntax that later steps add are shown as planned; the module
table says which exist.

## Goals

- **Data-driven.** An effect is a short text file of emitters, each built from small reusable
  modules: spawn, initial attributes, forces, drag, wind, curl noise, collision, colour and size
  over life, kill. Like `.ocean` and `.weather` files, it changes without touching code.
- **Dynamic.** Effects have named parameters that the weather, the time of day or gameplay can
  drive at any moment: the weather's `rain` sets how hard it rains, a script sets how hot a fire
  burns.
- **Simulated on the GPU.** A persistent pool of particles, a free list, compute simulation and
  indirect draws. The CPU never touches a particle and never waits for the GPU.
- **Part of the whole system.** A particle is lit as the scene around it is:
  - the sun, its cascaded shadows and the clouds' shadow;
  - light bounced from the GI volume, and the sky;
  - the same aerial perspective as everything else.

  It also:
  - fades softly where it meets the scene;
  - holds steady under TAA;
  - glows into bloom when emissive;
  - can cast a shadow.

  It collides with the scene and the sea, and spray can wet the rocks it lands on.
- **Realistic.** Physical units throughout: drops fall at their measured terminal speed, rain
  rate sets how many there are, fire glows at its temperature.
- **Performant everywhere,** the web and integrated GPUs included. Budgets are fixed, LOD is by
  distance, and the cost is measured at every step (see [Costs](#costs)).

Not in the first version, though the design leaves room for each:
- ribbons and beams;
- particles driving the GI volume or lighting the scene, beyond an effect's own light;
- fluid simulation;
- particles seen through the water's refraction;
- particles under water.

## What there is today

- **No particles.** The weather's `rain` (`clouds/weather.rs`) only darkens cloud bases and
  greys the air. There is no snow setting. `Observation` already carries WMO codes for snow.
- **Frame order** (`renderer.rs`, profiler sections in `profiler.rs`):

  ```text
  sky → clouds → ocean (waves) → shadows → seabed map → wet line → GI volume
      → cull/raster ×2 → resolve → clouds on screen → screen GI → lighting → water → post
  ```

  Post (`post.rs`) is TAA and upscaling, then exposure, bloom and tonemapping.
- **The water pass is the precedent for compositing.** It is a compute pass after deferred
  lighting. It reads the lit HDR target, the G-buffer's depth and motion, and writes its own
  colour, depth and motion (`WaterTargets`), which post reads in place of the G-buffer's. The
  lit opaque scene stays untouched for refraction. Particles go after it, in the same way.
- **What a particle can be lit and collided with, all already on the GPU:**
  - `sun_irradiance_at` (atmosphere and cloud shadow) and `sun_shadow` (four 2048² cascades,
    cached and rarely redrawn), in `common/environment.wgsl` and `lighting/sun_shadow.wgsl`;
  - `sky_radiance`, the sky with its clouds;
  - the GI volume (`gi/volume.rs`): five 96³ clipmap levels from 0.2 m voxels, holding radiance
    and the distance to the nearest surface. That is a signed distance field of the scene near
    the camera, which can be traced anywhere, on screen or off;
  - the scene's depth and normals, this frame;
  - the seabed map (`ocean/seabed.rs`): 1 km of the scene seen from above, clipped 2 m above the
    sea;
  - the water-line map (`ocean/waterline.rs`): where the water stands now and has stood lately,
    over 128 m around the camera, at 25 cm;
  - the scene's static local lights (`Light` in `common/scene.wgsl`).
- **TAA** (`post/taa.wgsl`) reprojects with the closest depth's motion in a 3×3 neighbourhood
  and clips history to the neighbourhood's colour. It has no notion of translucency. Anything
  drawn over the scene without its own motion would ghost.
- **Limits** the renderer already requires (`gpu.rs`): 8 storage buffers and 5 storage textures
  per shader stage. Core WebGPU has no float atomics, no 64-bit atomics, no multi-draw indirect,
  and no non-zero `first_instance` in indirect draws without an optional feature.
- **Scene files** (`.solstice`) have optional tagged sections after the manifest, each a
  four-byte tag, a length and its contents. The sea is one (`solstice-asset/src/file.rs`). Bake
  scripts (`scenes/*.txt`) have one command per line.
- **Gameplay** (`docs/gameplay.md`, PR #24) asks the renderer for general APIs only. It never
  sees an actor. Particles follow that rule.

## References

- **Unreal Engine's Niagara.** Systems of emitters, emitters as stacks of modules in spawn and
  update stages, user parameters, GPU simulation, events between emitters, and collision against
  scene depth and distance fields.
- **Gareth Thomas, *Compute-Based GPU Particle Systems* (AMD, GDC 2014).** Dead and alive index
  lists, emission and simulation in compute, sorting, indirect drawing.
- **Lorach, *Soft Particles* (NVIDIA, 2007).** Fading sprites by their distance to the scene's
  depth.
- **Bridson, Hourihan and Nordenstam, *Curl-Noise for Procedural Fluid Flow* (SIGGRAPH 2007).**
  Divergence-free turbulence from the curl of a noise field, so smoke swirls without bunching up
  or thinning out.
- **McGuire and Bavoil, *Weighted Blended Order-Independent Transparency* (JCGT 2013).** The
  alternative to sorting; see [Decisions](#decisions).
- **Garg and Nayar, *Photorealistic Rendering of Rain Streaks* (SIGGRAPH 2006),** and
  **Tatarchuk, *Artist-Directable Real-Time Rain Rendering in City Environments* (ATI, 2006).**
  How a drop looks as a streak over one exposure, a top-down map so that rain doesn't fall
  indoors, and splashes.
- **Gunn and Kinzer (1949),** terminal velocities of raindrops. **Marshall and Palmer (1948),**
  the distribution of drop sizes by rain rate. **Locatelli and Hobbs (1974),** fall speeds of
  snowflakes, about 1 m/s.
- **Henyey and Greenstein (1941).** The phase function for light scattering through droplets
  and smoke: strongly forward for mist and spray, mildly for smoke.
- **Six-way lighting** (Unity's VFX Graph, with maps baked from simulations): smoke lit from any
  direction by six baked light maps. A later quality step for smoke.
- **Motion-vector flipbooks** (as in Unreal's SubUV motion-vector blending): flipbook frames
  blended along baked motion, so slow smoke animates smoothly from few frames.

## The design

### Effects as data

An effect is a text file of one or more emitters. The built-in ones are in
`crates/solstice-render/particles/`: a fountain, motes adrift in still air that shine where they
cross a shaft of sun, and fireflies for dusk:

```text
# crates/solstice-render/particles/rain.particles
# Rain: streaks that fall with the wind and splash where they land.
param rain 0                       # 0..1, set by the weather
param size 1                       # drop size relative to the rate's typical one

emitter drops
  budget 40000                     # most particles alive at once
  follow camera                    # spawns in a box that moves with the camera
  spawn rate=14000*rain            # per second, scaled by the parameter
  shape box 50 24 50
  offset 0 12 0
  lifetime 2.5
  size 0.0015*size 0.003*size      # drop radius in metres, uniform between the two
  velocity terminal                # starts at its terminal speed (Gunn and Kinzer)
  gravity
  drag terminal                    # quadratic, so that it holds its terminal speed
  wind 1                           # carried by the weather's wind at its height
  collide cover water kill event=splash
  render streak exposure=0.016 opacity=0.3 lit
  lod near=25 far=50

emitter splash
  budget 6000
  spawn event=splash count=4
  lifetime 0.25 0.4
  velocity 0 1.6 0 cone=30 spread=0.25
  size 0.001 0.002
  gravity
  render sprite opacity=0.5 lit
```

- **Lines are a keyword, then values,** as in weather files. `#` begins a comment. Every module
  has defaults, so a file names only what differs. Two values where one would do are a range:
  each particle takes its own value, uniformly between them.
- **Options** are `key=value`, and take the plain values that follow them too, as in
  `color life 0=1 1 1 0 1=1 1 1 1`. Plain words come before options.
- **Parameters.** `param NAME DEFAULT` declares one. A value anywhere may be a number, a
  parameter, or a number times a parameter (`14000*rain`). That is all the expression language
  there is. A module that needs more takes a curve.
- **Curves** for anything over a particle's life, such as `color life 0=1 .6 .2 1 .5=.3 .3 .3 .6
  1=.2 .2 .2 0`, as keys from 0 to 1, linear between them. Each is baked to 32 entries per
  emitter on the GPU. The colour curve's fourth value scales opacity.
- **Errors name the line,** as with weather and oceans, and a bad file falls back to no effect
  with a warning, never a crash.

**The modules**, by stage. Step 1 built those in **bold**; the rest come with the steps that
need them.

| Stage | Modules |
|---|---|
| Spawn | **`spawn rate=`** per second, **`burst=`** at the start and **`every=`** seconds after, **`event=`** with **`count=`** from another emitter's hits, `distance=` per metre moved (trails); **`follow camera`** or **`follow effect`**; **`budget`** |
| Shape | **`point`**, **`sphere R [surface]`**, **`box X Y Z`**, **`offset`**, `disc`, `cone`, and two that join the scene: `sea crests` (where waves break) and `surf` (where the sea rises against a face) |
| Initial attributes | **`lifetime`**, **`velocity X Y Z cone= spread=`**, `terminal` and inherited velocity, **`size`**, **`color`**, **`brightness`**, rotation (random for now), `flipbook` frame, **`temperature`** in kelvin |
| Forces | **`gravity`**, **`drag`** (linear; quadratic and `terminal` later), **`wind`**, **`curl`** noise, **`buoyancy`** (hot gas rising), **`attract`** to the origin, **`vortex`** |
| Over life | **`color life`** (with opacity), **`size life`**, `spin`; **`cooling`** towards the air, and **`glow`** (emissivity), for fire and sparks |
| Collision | **`collide`** against **`depth`**, **`field`** (the GI volume's distance field), `cover` (the top-down cover map, step 4b) and **`water`**; then **`bounce`**, **`stick`**, or **`kill`**, with **`restitution=`**, **`friction=`** and an optional **`event=`** |
| Kill | **by age**; by leaving the follow box; below the water |
| Render | **`sprite`** facing the camera (velocity and up later), **`blend=alpha`** or **`add`**, **`soft=`** fade distance, **`opacity=`**, **`shading=lit`** (the default) or **`unlit`**, **`phase=`** asymmetry; `streak`, `flipbook`, later `mesh`; `shadow` to cast one |

**How modules run.** Modules are not compiled into a shader per emitter, as Niagara does. Each
emitter becomes a fixed record on the GPU (`EmitterRecord`, 192 bytes): its values, worked out
on the CPU each frame from the effect's parameters, flags for the modules it uses, and its slice
of the pool. Its curves sit in their own buffer. One simulation shader reads that record and
branches on it.
Every particle in a workgroup belongs to the same emitter, so the branches are uniform and cost
almost nothing.
- **Why:** browsers compile pipelines slowly, and on the first use. One set of pipelines,
  created at start-up, means loading an effect never stalls a frame.
- **If it measures slow,** the hottest emitters can get specialised pipelines through WGSL
  `override` constants, without changing the format.

**Effects in scenes and from code.**
- **Bake scripts** gain `effect PATH X Y Z YAW SCALE [param=value ...]`. Placed effects go into a
  new tagged section, `PRTC`, holding each effect's text, as the sea's section holds its ocean.
  Older readers skip it.
- **The library** gets general calls only:

  ```rust
  let fire = renderer.particles().spawn(&Effect::by_name("campfire")?, transform)?;
  renderer.particles().set(fire, "heat", 0.8);       // from the next frame
  renderer.particles().set_transform(fire, transform);
  renderer.particles().stop(fire);                   // stops spawning; what is alive dies out
  renderer.particles().kill(fire);                   // ends it at once
  ```

  `set` takes effect at once. Easing, as the weather eases, can be added where an effect needs
  it; a script can ease a value itself.

  Gameplay's `effect` component calls these. A footstep or a sword's hit is a `burst`.
- **The weather's own effects,** rain and snow, are owned by the renderer, which sets their
  parameters from the weather each frame. A scene does not place them.

### Simulation on the GPU

**The pool.** One storage buffer of particles, shared by every emitter. Each emitter has a fixed
slice of it, its `budget`, and keeps its own free list and alive list there.
- **A particle is 64 bytes:**
  - position and age;
  - velocity and lifetime;
  - the position at the start of its last step, for motion vectors, and its size;
  - its spawn index, which seeds its random values, and its rotation, with 8 bytes spare for
    later steps' temperature and flipbook frame.
- **Its light is 32 bytes more,** in a separate buffer written by the lighting pass (below):
  five colours in the shared-exponent RGB9E5 form, a word each, with the transmittance through
  the haze, a count of ambient samples, and the spawn index of the particle the light belongs
  to.
- **The pool's size** is set by quality:

  | Quality | Particles | Memory |
  |---|---|---|
  | High | 262,144 | 25 MB |
  | Medium | 131,072 | 13 MB |
  | Low | 65,536 | 6 MB |

  An effect started while the pool is nearly full gets smaller budgets than it asks for, down
  to the largest free run of slots; with none, it does not start. Changing quality makes a new
  pool, and running effects start again in it.
- **At most 64 emitters** run at once, across every effect, and 8 in one effect.

**Each frame** is a handful of dispatches and the draws, however many emitters there are. Step 1
has four: plan, spawn, update, and a small `finish` that writes the draw counts. Light, sort and
cull come with steps 2 and 3:

1. **Plan**, one workgroup. For each live emitter, it works out:
   - how many particles to spawn: rate times time, plus bursts and queued events, limited by its
     free list;
   - the emitter's LOD;
   - where its workgroups start in the spawn and update dispatches.

   It writes the indirect arguments for everything after it. One small pass replaces a dispatch
   per emitter, which matters on the web, where each dispatch costs CPU time in the browser.
2. **Spawn.** The plan takes the frame's spawns off the top of each free list at once, so each
   new particle reads its slot without an atomic. It is seeded from the emitter's running spawn
   count, not its slot, so a particle's random values are the same however the GPU orders the
   atomics. Spawns are spread evenly through the frame: each is born part way through it and
   moved on to its end, so a fast emitter makes a smooth stream, not clumps a frame apart.
3. **Update.** Ages each live particle and applies its emitter's forces and collisions. Then it
   kills the particle, returning its slot to the free list, or appends it to next frame's alive
   list. The two alive lists take turns, one read while the other is written. Collisions append
   events for other emitters to spawn from next frame.
4. **Light**, per particle, described under [Rendering](#rendering).
5. **Sort**, for emitters that need it (below), and **cull**: particles outside the frustum or
   smaller than a third of a pixel are left out of the draw lists, and the indirect draw counts
   are written.

Then one `draw_indirect` per visible emitter. The vertex shader pulls its particle from the
emitter's draw list by `instance_index`, so no instance buffer is built. The emitter's offset
into the list comes from a uniform array indexed by the draw, since a non-zero `first_instance`
needs an optional feature.

**Integration.** Gravity and linear drag are integrated exactly, in closed form, so a particle
lands in the same place however time is split into frames: a slow frame gives what two fast ones
would. Forces without a closed form (wind, curl noise, quadratic drag) and collisions will take
substeps of at most 1/30 s, so a slow frame doesn't throw sparks through walls.

**The CPU never reads a particle.** Statistics come back asynchronously, as culling statistics
do: alive, spawned and dropped for lack of budget, per emitter.

**A CPU reference** in Rust runs the same modules for one particle at a time. GPU tests compare
against it, as the waves' tests compare against `spectrum.rs`.

### Budgets and LOD

- **Hard budgets.** Each emitter has its slice; when the free list is empty, spawns are dropped
  and counted. Nothing grows, nothing overflows, and memory is fixed at start-up.
- **LOD by distance.** Between an emitter's `lod near` and `far`, its spawn rate falls to a
  fraction, and the particles that are spawned grow by its inverse square root, so the effect
  covers the screen as much as before with fewer particles. Beyond `far`, or outside the frustum
  by its bounds, it stops spawning. It keeps simulating what is alive until that dies, and wakes
  with a short pre-warm when it comes back.
- **Camera-following emitters,** rain and snow, are densest near the camera and thin with
  distance. Beyond about 50 m, rain is the weather's grey haze, which the clouds already draw.
- **Quality** scales every spawn rate: 1 on High, 0.6 on Medium, 0.35 on Low. Low also draws
  large translucent particles at half resolution (step 7).
- **Overdraw** is the real cost on integrated GPUs, more than particle count. So:
  - sprites fade out as they reach the camera, which avoids a full-screen layer of smoke;
  - each emitter's on-screen size can be capped;
  - a debug view, `particles`, shows overdraw as a heat map.

### Rendering

**Where it goes in the frame.** Particles are simulated after the scene is resolved, so they can
collide with this frame's depth, and lit after the sun's shadows, the GI volume and the clouds
are ready. They are drawn after the water, into the same colour target that post reads, the
water pass's when there is a sea and the lit HDR target when not. Two new profiler sections:

```text
… → resolve → clouds on screen → screen GI → lighting → water → particles → particle draw → post
```

**Depth.** The opaque scene's depth with the water in it is R32Float storage, which the hardware
can't depth-test against. A tiny full-screen pass copies it into a Depth32Float attachment, so
hidden particle fragments are rejected before shading. That matters most for overdraw on
integrated GPUs. Particles test against it but never write it.

**Lit translucent particles.** Light is worked out once per particle, in compute, not per pixel
(`particles/light.wgsl`, after the simulation, in the `particles` profiler section). That is the
main saving over shading each fragment, and on a small sprite it looks the same.
- **Sun:** the sun's light through the atmosphere and the clouds' shadow, times its cascaded
  shadow at the particle. Particles larger than a metre also take the shadow at four points
  across themselves, so a smoke column is lit along its length.
- **Ambient, the sky and light bounced off the scene together:** each time a particle is lit,
  it traces one direction and its opposite through the GI volume. A hit gives the scene's light
  there, and a miss the sky, or a share of the horizon below it. The particle keeps a running
  average of up to 16 samples. Smoke in an archway is darker than smoke in the open, and spray
  by a sunlit cliff takes its colour. A new particle starts from the open sky's average. Beyond
  the volume, the sky alone.
  - This replaced the plan's sky harmonics and separate enclosure term: one estimate gives both,
    and needs no extra pass.
- **Local lights:** each of the scene's lights in range.
  - **Not by screen tile.** The lighting pass's tiles cut each tile's lights to the depth range
    of the opaque surfaces in it. A particle in front of those surfaces, such as smoke before a
    wall, would miss lights that do reach it, so particles loop over the scene's lights instead,
    as the voxel pass does.
  - **Only the two strongest are traced.** Tracing a light through the GI volume, to stop it
    shining through walls, costs far more than the rest of the loop. So only the two strongest
    lights at each particle are traced, and the rest, each fainter, count as open.
  - **What that costs:** the `particles` section for 40,000 particles lit every frame, at High,
    among lights of 8 m range scattered over 12 m (the ignored test
    `particle_light_cost_with_many_lights`), in milliseconds:

    | Lights | RTX 5080 | Integrated |
    |---|---|---|
    | 0 | 0.07 | 4.9 |
    | 16 | 0.09 | 5.8 |
    | 64 | 0.10 | 5.5 |
    | 256 | 0.16 | 7.0 |

    The trade-off: a third light behind a wall counts as open, and as a particle moves, two
    lights can trade places. A light traced as blocked can then show as open, so light can leak
    or flicker by walls in rooms with three or more lamps near each other.

    Tracing every light in range cost 0.32 ms at 64 lights and 2.5 ms at 256 on the RTX 5080,
    and 17 ms at 64 on the integrated GPU. The loop still visits every light, so it grows with
    the scene's light count. A few hundred lights is comfortable; past about a thousand, lights
    should be gathered per cell of a coarse grid in front of the camera, as Unreal's
    translucency lighting volume does.
- **Glow:** a black body's light at the particle's temperature, from Planck's law integrated
  against the CIE colour matching functions (Wyman, Sloan and Shirley's fit), times the
  emitter's `glow`. Its luminance in candela per square metre is scaled to the scene's units as
  daylight is, taking full daylight on the ground as 100,000 lux. Bloom and exposure then treat
  it as they treat any bright pixel, with no special case.
- **Haze:** what the aerial perspective adds over black between the particle and the eye, and
  what it lets through, per particle.
- **What each sprite does:** its colour, as an albedo, scatters the stored light towards the
  eye. The sun goes through the emitter's Henyey-Greenstein phase, scaled to average 1 over
  every direction, so that a particle is taken as thick enough to scatter all it does not pass
  on. A white particle that scatters evenly is as bright in the sun as a white surface facing it,
  and mist and spray glow when backlit by a low sun. Then its glow is added, the haze laid over,
  and the result premultiplied by its opacity. Unlit particles skip the scattering.
- **Cost, by quality.** On High each particle is lit every frame. On Medium and Low it is lit
  afresh once every 2 and 4 frames, a workgroup at a time, so that those not due sit idle. In
  between it keeps the light it had. New particles head their emitter's list, so they share the
  workgroups lit in their first frame. The ambient rays take 16, 12 and 8 steps.
  - A particle's place in the list moves as others are born and die, so each records the frame
    it was last lit. One that has waited a full turn is lit whatever its workgroup: none waits
    longer than 2 or 4 frames.
  - Ambient light is stored with random rounding, so slow changes average out rather than
    stall on RGB9E5's precision.
  - The scene's GI intensity scales it, as it scales surfaces' indirect light.
  - A slice given to a new emitter has its light cleared, so no particle can take the light its
    slot's last owner had.
- **Additive particles** cover nothing, so the haze over what is behind them is already there:
  they add only their own light, dimmed by the air between.

**Soft particles.** Each fragment fades by its distance to the scene's depth over the emitter's
`soft` distance, so smoke never shows a hard line where it crosses the ground or the water. It
also fades near the camera's near plane.

**Aerial perspective.** The same `aerial_perspective` as everything else, per particle, applied
before premultiplying. A distant chimney's smoke fades into the haze exactly as the chimney does.

**Motion and TAA.** Translucent sprites over a moving scene are what TAA handles worst: it
reprojects by the opaque scene's motion and keeps the particle's colour as stale history.
- **Sprites are drawn with the jittered projection,** so TAA anti-aliases their edges.
- **A reactive mask.** Each translucent particle writes how much of a pixel it covers into an
  R8 target at render size, with a max blend. Where the mask is high, TAA weights the current
  frame up to 60% rather than 4–30%. This is what Unreal's responsive AA and FSR's reactive mask
  do.
  - **At the pixel alone, and not all the way.** Read over a pixel's neighbourhood, the mask
    spread a hard-edged band of little history around every particle. At 90%, TAA all but
    stopped under a large plume of smoke, and what showed through lost its anti-aliasing. 60%
    keeps a small, faint particle moving fast at about three quarters of its contrast.
  - **Faint particles are marked by presence, not just opacity:** four times their opacity, up
    to 1. They are what TAA loses most easily. A small, faint particle's neighbourhood looks much
    like the wall behind it, so the colour clip keeps the wall's history and the particle fades
    into a smear. Bright, solid-looking ones are kept by the clip alone.
  - A frame with no particles clears the mask once, so it never keeps an old frame's marks.
- **Solid particles** (`render sprite solid`: sparks, debris, later meshes) are opaque where
  their shape is more than half covered. They write their own depth and motion into the scene's
  targets that TAA reads: the motion runs from where the particle is now, unjittered, to where
  it stood last frame. TAA then follows them as it follows any surface. Translucent particles
  behind them are hidden, and fade softly into them.
- **Three passes.** A solid particle writes the scene's depth, which translucent ones read for
  their soft fade, and one pass can't do both. So drawing is:
  1. the depth copy;
  2. solid particles, if any, into colour, depth and motion;
  3. translucent particles, into colour and the reactive mask.

**Sorting.** Only where it shows.
- **Not sorted:** additive emitters (sparks, fire's glow), and tiny particles such as rain,
  splashes, spray and snow. Their order is invisible.
- **Sorted back to front** (`render sprite sort`): large, dark, alpha-blended emitters such as
  smoke and dust.
  - After the light pass, one workgroup per emitter sorts its whole alive list in workgroup
    memory, with a bitonic network, and rewrites the list from the keys.
  - A key is the particle's depth along the view, on a log scale in 20 bits, farthest
    smallest, above the particle's place in its slice in 12 bits.
  - 4,096 keys of four bytes fill the 16 KiB of workgroup memory WebGPU promises, so a sorted
    emitter's budget is at most 4,096, which the parser enforces. An effect changed in code
    past that is drawn unsorted, with a warning, and the shader also refuses to sort more than
    it can hold. Larger sorted emitters would need a sort across several dispatches.
- **Between emitters:** drawn far to near by where their shapes are centred.

**Shadows from particles** (step 8). The sun's shadow maps are cached and rarely redrawn, so
particles cannot go into them. Instead, a small per-frame map of opacity seen from the sun:
- 512², over the nearest cascade's area;
- particles splat their opacity into it with a multiplicative blend.

The lighting pass, the water pass and particle lighting all multiply the sun by it. Smoke then
shades the ground below it and shades itself. Only emitters marked `shadow` cast one, and it is
off on Low.

### Forces

The forces that vary through space and time (step 4a) take substeps of at most 1/30 s; with
gravity and linear drag alone, the step is still exact.
- **Moving air** is the wind, curl noise and vortices, summed. It pulls a particle through its
  drag, as an extra acceleration of drag times the air's velocity: a particle with no drag isn't
  carried at all, and one with plenty soon moves with the air.
- **`wind S`** is S times the weather's wind near the ground.
  - The weather's wind is at cloud height; 10 m up it is half that, as the sea takes it.
  - It is brought to the particle's height by the log profile over open ground (a roughness
    length of 3 cm), from the sea's level, or 0 with no sea, and no lower than half a metre.
  - It gusts by up to 30% either way, with slow noise in space and time.
- **`curl SPEED scale=L change=R`** is the curl of three gradient noises (Bridson et al.).
  - Its divergence is zero, so it swirls particles without gathering them or thinning them out.
  - The noise has an analytic gradient, so its curl is exact.
  - It moves through time at R noise cells a second.
- **`vortex W`** is air turning about the effect's up axis at W radians a second.
- **`attract A`** is an acceleration towards the effect's origin.
- **`buoyancy B`** lifts a hot particle by B times gravity, times how much hotter than the air it
  is, as a share of its temperature.

### Collision

Collisions run as a pass of their own (`particles/collide.wgsl`), after the simulation and
before the light, over the same workgroups as the light pass. Each test is cheap, and an emitter
picks the ones it needs, as `collide depth field water` then `bounce`, `stick` or `kill`, with
`restitution=`, `friction=` and `event=`:
- **`depth`.** This frame's opaque depth and normals, so a particle bounces off exactly what is
  drawn.
  - It touches the plane of the surface drawn at its pixel once its centre is within its radius
    of it. A particle that began the frame behind that plane passed behind what is drawn, and is
    left alone.
  - It is detailed, but blind off screen and behind things.
- **`field`.** The GI volume's distance field: everywhere within the volume, on screen or not,
  at 0.2 m near the camera and coarser further off. This is Niagara's distance-field collision.
  - The field holds the distance to the nearest surface without saying which side a point is
    on. So a particle's path this frame is marched from where it started, in steps no longer
    than that distance, which can't pass through a surface. It touches where the distance first
    falls within its reach: its radius, or half a voxel.
  - The normal is the field's gradient. A particle in touch with a surface remembers that normal
    in its flags, because under the ridge of a coarser level the gradient points the wrong way.
  - One that began the frame within reach, and knows its side, keeps its movement along the
    surface and loses any into it. If what is within reach is beside it instead, such as a wall
    by a ledge it has slid off, nothing holds it up.
  - On the two finest levels its path, offset three voxels from the surface, is
    still marched for another surface ahead, so a wall stops it and anything lower than that
    doesn't, nor a ledge sticking out from a wall by less than that. On coarser levels it isn't.
  - Where a wall meets a floor the field is blurred for about a voxel. A bouncing particle landing
    there can take a diagonal normal and slide along it through the floor. Stick and kill end at
    the contact, so only bounce is affected.
  - One within reach that remembers nothing was born there. It is trusted when the gradient points
    up, off a floor; otherwise it is left alone until it has come out of reach, and what it meets
    after that counts. So a particle born under the field's ridge, within half a voxel of a floor,
    and moving down passes through it, and one born between a ceiling and its ridge is taken to be
    on a floor there. Particles spawned by an event remember their surface.
- **`water`.** The water's height now, from the water-line map within 64 m and the calm level
  beyond it. A particle more than 25 cm under it when the frame began stays under.
- **What a hit does.** Only a particle moving into the surface bounces, sticks, dies or sends an
  event.
  - **Bounce** keeps the restitution's share of speed into the surface, and loses the
    friction's share along it.
  - **Resting.** A bouncing particle moving into the surface slower than two frames' worth of
    what pushes it there is resting rather than landing. It is held at the surface and sends no event, and slides on it,
    losing the friction times the speed the surface took from it (Coulomb's law).
  - **Stick** marks the particle, which then only ages.
  - **Kill** ages it out at once: it is drawn as nothing and freed the next frame.
- **Events.** A hit's `event=NAME` reaches the emitter of the effect that spawns on it
  (`spawn event=NAME count=N`).
  - An event has one listener; the parser refuses two.
  - Each emitter takes up to 256 events a frame, into its own slice of an event buffer. What
    those past it would have spawned counts as dropped.
  - The next frame it spawns N particles at each, before any by rate. They leave the surface
    with the emitter's velocity turned so that its up is the surface's normal.
  - The parser refuses an event that no emitter of the effect spawns on.
- **`cover`.** A new top-down map of the scene's highest surfaces: 512² over 256 m, half a metre
  a texel. It is drawn by the shadow maps' depth-only pass, like the seabed map but with nothing
  clipped away, and only when the camera has moved far or geometry has streamed in. Rain and
  snow stop where they first meet a surface, so it doesn't rain indoors or under the gate's arch.
  Splashes spawn where the drops land. Later, rain wetness reads the same map to know what is
  sheltered.
- **`water`.** The water's height now, from the water-line map within 64 m of the camera, and
  the calm level beyond it. A drop or spray hits the waves that are drawn, not a flat plane. The
  map's height is what the water reaches at each texel, so it is a few centimetres generous
  against cliff faces, which is good enough for a landing.

**What a hit does:** `bounce` with restitution and friction, `stick` (snow settling, for its
lifetime), or `kill`. Any of these can send an `event` to another emitter, which spawns from
the hit's position and normal next frame: splashes from raindrops, rings from drops landing on
the sea, sparks from sparks.

**Wetness.** Spray that lands on the rocks wets them. Its impacts raise the remembered water
level at that texel of the water-line map, with an `atomicMax` on the height, as a quantised
`u32`. The water-line update then folds it in, so it dries as the sea's own wet band does, with
no new wetness logic. Rain wetness is its own later design. It will use the same wet material
and the cover map, not individual drops.

### The first real uses

Each one proves part of the system, in the cove:

| Effect | Proves | Driven by |
|---|---|---|
| **Rain**: streaks, splashes on stone, rings on the sea | Camera-following emitters, cover collision, water collision, events, streak rendering, the reactive mask | The weather's `rain` and wind |
| **Snow**: flakes drifting, settling and melting | Wind, curl noise, `stick` | A new weather setting, `snow` |
| **Sea spray**: thrown off breaking crests and off the rocks in the surf | Shapes that read the sea, backlit phase, wetting rocks | The waves' folding and the water-line map's rise against faces |
| **Smoke**: from a chimney or a fire | Sorting, soft particles, sky and bounce light, buoyancy, curl noise, casting shadows | Wind, and a script's `heat` |
| **Fire**: flames over the smoke | Emissive by temperature, bloom, a light of its own | A script's `heat` |
| **Sparks**: from the fire, and from impacts | Additive, field collision with bounce, solid motion vectors | Bursts from gameplay |
| **Dust**: lifted from dry ground in a strong wind | Wind, LOD, cover-map spawning, wetness keeping it down | Wind speed, and wetness |

- **Rain is realistic,** not a texture:
  - the rain rate in mm/h sets the number and size of drops, by Marshall and Palmer;
  - each drop falls at its terminal speed, about 4–9 m/s, and leans with the wind;
  - a streak is the drop's path over one exposure, lit by the sun and the sky behind it, as Garg
    and Nayar describe.
- **Snow** falls at about 1 m/s and wanders with curl noise. Flakes settle and melt; settling as
  a white layer on the ground is out of scope here.
- **Spray** comes from the joins with the sea that already exist:
  - from crests, where the waves' surface folds over: the Jacobian of their displacement falls
    below a threshold. Ocean step 4, foam, needs the same quantity, and the two will share it;
  - from the surf, where the water-line map shows the water rising fast against a steep face.
- **Fire, a fire's light.** A fire carries one point light, which flickers with its heat. It uses
  the moving-light API that gameplay is adding (`docs/gameplay.md`), not one of its own.

### Debug views

- **`particles`:** overdraw as a heat map, the thing to watch on integrated GPUs.
- **`particle light`:** each particle's stored light, without its texture or colour.
- **Statistics overlay:** alive, spawned and dropped per emitter, and the pool's use.

## Steps

Each step is its own PR off `main`, and none starts until the last is merged. Each has:
- GPU tests that are mutation-tested: the bug is put back, and the test must fail;
- costs measured on the RTX 5080 and the integrated GPU, repeated, with cold first runs
  discarded;
- `cargo test --workspace` passing;
- clippy clean with `-D warnings`, natively and for `wasm32-unknown-unknown`;
- a review by a separate AI agent, and your go-ahead before it is merged.

1. **The core.** *Built; in review.*
   - The `.particles` format and its parser, and the CPU reference.
   - The pool, free lists, alive lists, and the plan, spawn and update passes.
   - Indirect drawing of unlit, soft sprites; the depth copy.
   - The basic modules: rate, burst, point, sphere and box shapes, lifetime, velocity, size,
     colour, gravity, drag, and colour and size over life.
   - Profiler sections, the `particles` overdraw view, and statistics.
   - A test fountain effect for the viewer (`--effect NAME`).
   - Tests:
     - The GPU's trajectories match the CPU reference.
     - Particles are conserved: alive plus free always equals each budget, over thousands of
       frames of spawning and dying. A double free or a leaked slot fails it.
     - Spawns beyond the budget are dropped and counted, and never write outside the slice.
     - A sprite fades to nothing where it meets a surface.
     - Uneven frame times give the same result as even ones.

   Further tests, from the review:
   - the WGSL records match their Rust twins field by field, as naga lays them out;
   - a running effect moves to a resized pool;
   - camera-following emitters spawn around the camera;
   - additive particles add light while alpha ones cover;
   - a paused frame moves nothing;
   - bursts survive low detail;
   - the overdraw view is black with nothing running.

   Every test was mutation-checked: each of 21 bugs was put back in turn, and each made a test
   fail. They included:
   - a leaked or doubly freed slot;
   - drag ignored, and plain Euler steps;
   - seeding by slot;
   - unclamped spawns;
   - no soft fade, and no depth test;
   - drawing into the target that post does not read over the sea;
   - additive particles covering;
   - WGSL fields out of order.

   In a browser (Chromium's WebGPU, through the web viewer's new `?effect=NAME@x,y,z`), the
   fountain draws in the overdraw view with no GPU errors. Unlit, it is faint against the sunlit
   scene; step 2 lights it.

   Known until step 3: particles have no motion vectors or reactive mask, so in the final image,
   with TAA on, moving particles leave short trails. The tests run with TAA off.

   **Cost** at 1280×720, the median of repeated runs with the first discarded. On the
   integrated GPU (AMD Radeon, Low quality) the render scale is 0.6 and spawn rates are 0.35.

   | Case | RTX 5080, High: simulate, draw | Integrated, Low: simulate, draw |
   |---|---|---|
   | No effects | 0, 0 | 0, 0 |
   | One particle: the fixed cost, mostly the depth copy | 0.02, 0.02 ms | 0.03, 0.11 ms |
   | The fountain, close: 3,200 particles (1,100 on Low) | 0.02, 0.04 ms | 0.05, 0.19 ms |
   | 100,000 small drops (35,000 on Low) | 0.07, 0.07 ms | 0.29, 0.41 ms |

   The profiler reads to a hundredth of a millisecond. The pool takes 20 MB on High and 5 MB
   on Low.

   Running on the integrated GPU also showed that the profiler's readback cast its mapped
   bytes in place to 64-bit numbers. A mapped range is only promised 4-byte alignment, and with
   this step's extra buffers the AMD driver gave no more, so the viewer panicked. The bytes are
   now copied out instead.
2. **Lighting and haze.** *Built; in review.*
   - Per-particle light: sun, cascaded shadow and cloud shadow; ambient from rays through the
     GI volume, which gives the sky with enclosure and the bounce together; local lights.
   - Phase functions, glow by temperature with `temperature`, `cooling` and `glow`, and aerial
     perspective; `shading=lit` (the default) or `unlit`.
   - The `particle-light` debug view, number 14.
   - Tests, each mutation-checked: 19 bugs were put back in turn, and each made a test fail.
     - RGB9E5 holds every channel to within a step of its largest, and a black body at 2,042 K,
       the freezing point of platinum, has the luminance that once defined the candela,
       60 cd/cm², to within 3%.
     - A black additive particle in the haze leaves the wall behind it as it was.
     - An effect started in a just-ended effect's slice shows none of its glow, even at Low.
     - While an emitter keeps spawning, Medium and Low bring all of a cloud into a newly risen
       sun within six frames, as High does.
     - With GI turned off, a particle in shadow is dark.
     - Local lights light particles in range, the ball stops a lamp from lighting a particle
       behind it, and lights beyond the two traced still count.
     - A particle in the ball's shadow is darker than one in the sun, in the light view and in
       the final image.
     - When the sun moves, a particle's light follows within a few frames at every quality.
     - 600 m off, a particle hazes as the glowing wall beside it does.
     - Beside a sunlit red ball, a particle's light is redder than beside a white one.
     - A particle that scatters onward glows far brighter with the sun ahead than behind.
     - A particle at 2,400 K glows orange and blooms; with no temperature, it doesn't.
     - Temperatures at birth match the CPU reference, and cooling follows its exponential.

   **Cost** at 1280×720 in the cove, with the same method as step 1. The figures are
   simulate-and-light, then draw:

   | Case | RTX 5080, High | Integrated, Low |
   |---|---|---|
   | The fountain, close | 0.06, 0.03 ms | 0.18, 0.20 ms |
   | 100,000 lit drops (35,000 on Low) | 0.17, 0.06 ms | 2.0, 0.52 ms |

   On the integrated GPU, lighting every particle every frame cost 4.4 ms for the 35,000 drops;
   lit once in four frames, with overdue particles lit too, it is 2.0 ms. Of what remains, about 0.7 ms is simulation and
   bookkeeping, and the rest is mostly the two GI traces and the 12-tap shadow. That is over the
   budget for heavy rain on Low. Rain's drops are small and fast, so step 5 should give them a
   cheaper light, such as the sky and the sun's shadow without GI traces.
3. **Motion, TAA and sorting.** *Built; in review.*
   - The reactive mask and its use in TAA.
   - Solid particles' depth and motion.
   - The bitonic sort, and ordering between emitters.
   - Tests, each mutation-checked: 18 bugs were put back in turn, and each made a test fail.
     The CPU's and the shader's refusals to sort too many guard the same case, so they were
     removed together.
     - An emitter changed in code to a budget too large to sort keeps its particles whole.
     - Sorting follows a turned camera.
     - The reactive mask is clear once particles stop.
     - A solid particle hides a translucent one behind it.
     - The solid particle's depth and motion are checked with a sea too, where they go into the
       water's targets.
     - A solid particle moving across the view writes its own depth, and the motion that takes
       it to last frame's place, to within 10%; the wall beside it does not move.
     - A small, faint particle moving fast keeps about three quarters of its contrast under
       TAA. Without the mask it fell to a quarter.
     - Sorted lists of 3, 100, 1,000 and 4,096 particles run far to near and hold each particle
       once; an unsorted emitter is left alone.
     - Nearer emitters draw over farther ones, in either order of starting.

   The built-in fountain now uses both: its jet's drops are solid, and its mist is sorted.

   **With no particles,** the cove's final image differs from `main`'s as much as two runs of
   the same build differ from each other: a mean of 0.09 levels in 255, on about 11% of pixels,
   from the live sea and sky. The reactive mask is cleared, and TAA is unchanged there.

   **In a browser** (Chromium's WebGPU), the sorted mist and solid drops draw with no GPU
   errors: the sort's 16 KiB of workgroup memory and the solid pass's R32Float and Rg32Float
   attachments, written without blending, are accepted.

   **Cost** at 1280×720 in the cove, with the same method as before. The figures are
   simulate, light and sort, then draw:

   | Case | RTX 5080, High | Integrated, Low |
   |---|---|---|
   | The fountain, its mist now sorted | 0.07, 0.06 ms | 0.47, 0.21 ms |
   | 100,000 drops (35,000 on Low) | 0.17, 0.08 ms | 2.0, 0.53 ms |
   | 4,096 large sorted smoke puffs (1,425 on Low) | 0.14, 0.95 ms | 0.47, 2.7 ms |
   | 10,000 solid sparks (3,300 on Low) | 0.04, 0.03 ms | 0.12, 0.18 ms |

   - **The sort's cost on the integrated GPU is mostly waiting.** About 0.3 ms, even for a few
     hundred particles: a bitonic network of n keys takes about log²(n)/2 stages, each ending
     at a workgroup barrier.
     - Letting each lane own a run of neighbouring keys, so that the stages within a run need
       no barrier, would cut that.
     - So would sorting every other frame, since the order changes slowly.
   - **The smoke's draw is overdraw:** large puffs covering the same pixels many times. That is
     what step 7's half-resolution drawing is for.
4. **Forces and collision,** in two parts. The cover map draws the scene from above through the
   shared culling and rasterising, which another session was changing, so it lands on its own.
   - **4a, forces, collisions and events.** *Built; in review.*
     - Wind from the weather, brought down to the particle's height with a log profile, plus
       gusts; curl noise, buoyancy, attract and vortex.
     - Collision against depth, the distance field and the water, with bounce, stick and kill.
     - Events between emitters.
     - Tests, each mutation-checked: 33 bugs were put back in turn, and each made a test fail. No test reaches two other paths:
       a wall merely beside a particle not holding it up, and a corner's blur not being taken for
       the surface.
       - Under every force at once, the GPU's particles are where the CPU reference puts them,
         to within 0.1 mm (2 µm measured).
       - Curl noise has no divergence, and the noise's gradient matches finite differences.
       - Wind carries a particle with drag up to its speed.
       - Particles thrown at a floor in view never fall through it, by depth or by the field.
       - A dropped particle rebounds to its restitution squared of its fall.
       - Drops landing on the sea die there, and each sends three splash particles.
       - Sticking particles stay where they landed.
       - A landing particle slides on, by depth or by the field, until friction stops it.
       - A resting particle sends no more events, at a shallow view or pressed by other forces.
       - What is born on a surface can leave it, and an event's child knows the open side.
       - A sliding particle still stops at a wall, and a slow one doesn't jump to it.
       - A particle born near a surface still hits the next, one falling from just above a floor
         lands, and a wall beside a particle holds nothing up.
       - Events past the limit count as dropped.
       - What is already behind a surface, or under the water, stays there.
       - An effect with events lasts until its last child could have died.

     The built-in fountain's drops now die where they land, and its mist drifts on the
     weather's wind and swirls.

     **Cost** at 1280×720 in the cove, with the same method as before, simulate (now with
     collisions), light and sort, then draw:

     | Case | RTX 5080, High | Integrated, Low |
     |---|---|---|
     | The fountain | 0.08, 0.08 ms | 0.52, 0.28 ms |
     | 100,000 drops in wind and curl noise, dying where they land (58,000 alive; 20,000 on Low) | 0.12, 0.05 ms | 1.7, 0.4 ms |

     The same drops on the integrated GPU, at Low, three runs each:
     - with neither forces nor collisions, 35,000 alive: 2.2 ms;
     - with the forces alone, 35,000 alive: 2.4 ms;
     - with the collisions alone, 20,000 alive: 1.6 ms.

     So forces add about a tenth to a particle's cost there, and collisions about a third.
     Lighting is still most of it.
   - **4b, the cover map.** Rain and snow stopped by the first surface above them.
     - Test: particles under the gate's arch stay dry of rain.
5. **Rain and snow.**
   - The renderer's own rain and snow, driven by the weather.
   - A new weather setting, `snow`, in the weather files and in `Observation`.
   - Streak rendering, splashes and rings on the sea.
   - Tuned against photographs and against the rain-rate figures.
   - Tests:
     - Drop counts and speeds match Marshall and Palmer, and Gunn and Kinzer.
     - No rain falls beneath cover.
     - `rain 0` costs nothing, as no emitter runs.
6. **Sea spray.**
   - Spawning from folding crests and from surf against faces.
   - Backlit phase, and wetting the rocks through the water-line map.
   - Tests:
     - A calm sea throws no spray, and a gale throws plenty.
     - Spray lands where it should, and the rocks it lands on are wet, then dry.
7. **Smoke, fire, sparks and dust.**
   - Flipbooks with normals and motion vectors.
   - Fire by temperature with its light; sparks; dust over dry ground.
   - Half-resolution drawing of large translucent particles on Low, upsampled by depth.
   - Example effects placed in the cove.
8. **Particle shadows.** The sun's opacity map, cast by marked emitters and received by the
   lighting pass, the water and particles. Tests: smoke's shadow darkens the ground under it, by
   its opacity.
9. **README, screenshots and costs.**
   - Measured in a heavy storm with spray and a fire, on the RTX 5080, on the integrated GPU,
     and in Chromium, Edge and Firefox.
   - Viewer controls for rain and effects.

Mesh particles, such as debris and splinters, forward-shaded with their own depth and motion,
come after, if you want them.

## Costs

Targets, at 1280×720, measured each step against them. The integrated GPU already takes about
30 ms a frame at Low, so particles must take only a small share of that.

| Scene | RTX 5080, High | Integrated, Low |
|---|---|---|
| No effects running | 0 ms: nothing is dispatched | 0 ms |
| Heavy rain: about 40k drops, with splashes | 0.25 ms | 1.2 ms |
| Storm: rain, spray and a fire with smoke | 0.6 ms | 2.0 ms |
| Simulation alone, per 100k particles | 0.1 ms | 0.8 ms |

Each step reports, separately:
- plan, spawn and update;
- lighting;
- sort;
- the depth copy;
- the draw;
- overdraw, for the draw;

plus memory. Timings come from the profiler natively. In browsers, only the frame time can be
measured.

## Decisions

Taken on 8 November 2026, when you approved the plan with the recommendations below as they
stand.

1. **Which first use to build first, after the core.** I recommend rain. The weather already has
   `rain`, and it proves cover collision, water collision, events, streaks and the reactive mask
   in one go, in the cove, where it shows. Spray is the alternative: it ties into the sea the
   most.
2. **Snow in the weather.** I recommend a `snow` setting from 0 to 1 beside `rain`, set from
   `Observation`'s snow codes. It changes the weather files and the weather type that the web
   page and gameplay read.
3. **Sorting or order-independent blending.** I recommend sorting only the emitters that need it,
   and ordering emitters by distance, as Unreal does. Weighted blended OIT needs no sort at all,
   but it muddies dark smoke in front of bright fire, which is the case that matters most.
4. **Where smoke and fire textures come from.** I recommend baking our own flipbooks: a small 2D
   smoke and fire simulation in `solstice-procgen`, written out as frames with normals and
   motion vectors. There is then no question over licences, and they can be retuned. The
   alternatives are noise shaded in the shader, which is cheaper but looks procedural, or CC0
   flipbooks from elsewhere, if suitable ones can be found.
5. **The budget on integrated GPUs.** I propose the targets in [Costs](#costs): about 1.2 ms for
   heavy rain and 2 ms for a storm at Low, on a frame that is already about 30 ms. Do you want
   tighter, or effects held back further on Low?
6. **Particle shadows (step 8) and mesh particles.** Both are optional. I recommend shadows,
   because smoke without them looks pasted on, but off on Low. Mesh particles can wait until
   gameplay needs debris.

Publishing the cove with effects is a separate question, for when there is something worth
showing.

## Risks

- **Particles behind the water.** The water's refraction sees the lit scene without particles,
  so spray behind a wave's surface vanishes rather than showing dimly through. It is rarely
  visible, as spray is above the water. The fix, drawing particles into the lit target before
  the water pass, is possible later.
- **TAA and thin streaks.** If the reactive mask isn't enough, rain streaks can be drawn wider
  and fainter, or kept out of history altogether.
- **Overdraw on integrated GPUs.** Smoke near the camera can cost more than all the simulation.
  The defences are near fades, on-screen size caps and half-resolution drawing; the overdraw
  view shows whether they are enough.
- **WebGPU limits.** The update pass binds the pool, both alive lists, the free lists, the
  emitter table, events and counters: 7 storage buffers, within the 8 required. Collision adds
  textures, not buffers. Everything counts with `u32` atomics only.
- **Light per particle, not per pixel.** Large sprites could look flatly lit. Four samples over
  large particles, flipbook normals, and later six-way lighting are the answers, in that order.
- **The GI volume's reach.** Bounce light and field collision only work within it: its coarsest
  level is about 300 m across, centred on the camera. Beyond it, particles get sky light and the cover map.
- **Determinism.** Atomics hand out slots in any order. Tests check invariants and compare
  trajectories by spawn index, never by slot. Seeding by spawn index keeps the look the same
  from run to run.
- **Shared code.** Lighting, TAA, the water-line map and the weather all gain small changes.
  Each change is in the step that needs it, with the tests of the system it touches still passing.
