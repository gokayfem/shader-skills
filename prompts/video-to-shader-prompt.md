# Prompt: recreate a reference video as a GLSL shader

Use the **shader-art-coding** skill (read its `SKILL.md` first and follow its workflow) to recreate the reference clip
below as a single, paste-ready Shadertoy fragment shader: `mainImage`, GLSL ES 3.00, single pass, no textures, no
custom uniforms, driven only by `iTime`, `iResolution` and `iFrame`.

Fill these in before sending (leave a line blank to let the agent measure it from the video):

- **Reference video:** `<URL or path>`
- **Output name:** `<name>` (files become `<name>.glsl` and `<name>.out.glsl`)
- **Must keep:** `<the beats or objects you care about most, or blank>`
- **Allowed to skip:** `<anything you already know you do not want, or blank>`
- **Target machine for fps:** `<e.g. Apple M1, 1280×720>`

The video is the spec. Nothing in this document describes its content; you derive all of it from the frames
(section 1) and get the breakdown approved before building.

During the reference breakdown, use the skill’s [example guide](../skills/shader-art-coding/references/example-selection.md). For each distinct visible mechanism, record the closest source, what can be adapted to this single pass, and the frame or motion that will test it. Look beyond the main subject to its environment, materials, illumination, camera and transitions. Reuse these observations in the milestone comparisons.

Use targeted search first. From the skill directory, run `python3 scripts/glsl_tools.py search <terms>` to find a helper, then `python3 scripts/glsl_tools.py examples <exactName>` for direct uses and available debug views. Open the matching source at the reported line. For host-side wiring, run `rg -n -w <symbol>` inside the relevant world’s source directory and follow its definition and consumers. For a visual brief without a known symbol, start with the example guide.

## 0. Quality bar (read first; overrides anything below that conflicts)

This is a showcase piece. **Visual and motion quality outrank performance.** Report fps, but do not trade detail or
smoothness for speed without asking. Nothing is simplified without approval (section 9).

Apply the checks to what appears in the clip. Identify the relevant rows once during the breakdown. For a locked-off mineral study, for example, track the band contours, highlight motion and exposure. For a swimming animal, track the body silhouette, stroke phase, parallax and attenuation. A failed applicable check stays visible in the review until corrected.

- **Motion** (apply continuity within continuous action and preserve observed cuts, impacts and locked cameras)
  - During continuous action, check pose, speed and camera for unintended jumps. Prove it with 10-frame strips at 1/60 s spacing around each
    one: every change of gait or state (walk, run, stop, hop takeoff and landing, fall, turn), every camera move,
    and the loop seam.
  - Planted feet (or any contact: wheels, paws, hands) never slide. Prove it with a close-up strip of one full gait
    cycle per gait.
  - Gait phase is integrated (phase = ∫ speed / stride dt, computed analytically), so changing speed never jumps the
    phase.
  - For smooth gait changes, ease the weights for walk, run, stand and jump over the observed transition interval.
  - Secondary motion (hair, ears, cloth, tails, antennae, foliage) is spring-damped: it lags, overshoots and settles.
    It is analytic (damped responses to the body's acceleration events), since a single pass has no state.
  - Jumps and thrown objects are ballistic under one gravity consistent with the scene's scale: anticipation squash,
    stretch on the rise, squash on contact, a settle afterwards. Bouncing objects lose height (restitution below 1),
    and their spin matches the distance they roll.
  - For a continuously tracking camera with a damped response, evaluate the follow analytically and check for jitter at speed changes.
- **Detail**
  - At 1280×720, anything covering more than 10 % of the frame shows three levels: silhouette, secondary forms
    (folds, seams, ledges, lobes, clumps), and surface detail.
  - Close-ups at 4× zoom of every hero object: characters at 8 gait phases plus mid-action, and props, terrain and
    vegetation at near and mid distance.
- **Reference**
  - Every object is localized: list its visible parts (a flower has a head, stem and leaves; a tree has roots, trunk,
    limbs and a crown) and build each part. An object reduced to its most obvious part fails.
  - Colors are measured, not guessed: crop the same region in the reference and the render and compare mean RGB and
    spread (section 3). Report the numbers.
  - Every timeline row (section 1) has a side-by-side at the same time, with the remaining differences written down.

**Process.** Work in milestones (section 7). After each milestone, critique your own renders against the reference,
fix the 5 biggest differences and re-render, at least 3 passes. Show the strips, close-ups and side-by-sides before
moving on. Judge motion as frame sequences; stills hide motion bugs. Do not stop or summarize while a check fails,
and do not invent content the video does not show.

---

## 1. Reference breakdown (do this first, then stop for approval)

Extract frames before writing any shader code. Draw the video into a canvas at exact times; upscaling first helps
inspection. Then write `REFERENCE.md` with the items below and show it for approval.

1. **Clip facts:** duration, resolution and aspect, cuts or one continuous shot, whether it loops.
2. **Shot timeline.** Sample every 0.5 s (finer around fast action). For each row: time, beat name, what the frame
   shows, and where the subject is (on what, doing what). Beats must later land within ±0.25 s.
3. **Object inventory.** For every object that is ever larger than about 2 % of the frame: its parts, shape language,
   size relative to a known reference (the main character's height is usually the best ruler), position relative to
   other objects, and the frames where it is best seen. Measure sizes and positions from several frames, and check
   that they agree.
4. **Character or subject sheet:** parts, proportions, colors, materials, and how each part moves (primary motion,
   secondary motion).
5. **Motion log:** speeds, gait (stride, cadence), pauses, jumps (takeoff, apex, landing times), and the props it
   interacts with, with times. Note where the timing is uneven (pauses, settling), since that is easy to miss.
6. **Camera log per beat:** position relative to the subject (behind, above, side; distance as subject heights),
   framing (subject size and position in frame), tilt, how it moves between beats (follows, holds, pulls back,
   rushes in), and an estimate of the field of view.
7. **Light and palette:** sun direction in frame terms, key, fill and shadow colors, sky gradient, fog or haze, and
   sampled sRGB values for the main surfaces (character parts, ground, vegetation, rock, sky top and horizon).
8. **Style notes:** the look (toon, stylized PBR, clay), edge softness, saturation, grade, bloom, depth of field.
9. **What you cannot reproduce exactly,** and the substitute you propose for each (section 9 decides).

For each difficult mechanism, add one row to the breakdown. Describe a measurement someone else could repeat in the same frame.

| Moment in the supplied clip | Observation to record | Implementation to inspect | Comparison to make |
|---|---|---|---|
| First clear view of a solid entering another layer | Screen position of the overlap and which edge disappears | Nearest-hit selection, silhouette or layer masks | Flat IDs beside the reference crop immediately before and after entry |
| Highlight, glow or apparent color change | Whether it follows the subject, light or screen as the camera moves | Local material mask, optical response or post pass | Freeze pose, vary the suspected driver, then compare the real time sequence |
| Repeating movement, hold, cut or loop seam | Frame of onset, peak, release and any intentional discontinuity | Time mapping and the affected object's transform or field | Short strip centered on that transition with each render paired to its reference timestamp |
| Background that changes with camera travel | Relative shift of one near and one far landmark | Camera projection, procedural coordinates and depth ordering | Overlay both paths through the shot and inspect where they diverge |

Use rows matching the footage. Record uncertain causes as hypotheses. A single projected image can support several different depths, scales or lighting setups. Choose a consistent reconstruction and test it against another frame when available.

## 2. Goal and priorities

- **Goal:** a real-time interpretation that reads as the same shot: the same beats at the same times, the same
  composition and camera moves, the same palette and light, believable motion.
- **Not the goal:** a pixel match. Offline-rendered references have millions of blades and leaves. Match the richness
  with clumped geometry near the camera and detailed shading farther away, not by dropping detail.
- **When you must trade off:** (1) smooth, physical subject and camera motion, (2) composition and timing,
  (3) object detail, (4) light and palette, (5) particles and atmosphere effects. Performance comes after all of
  these.

## 3. Comparison tooling (build it before the first milestone)

Keep it in a scratch folder (not in the deliverable). It costs an hour and saves days.

- **Frame viewer:** renders the bundle at chosen times, a crop or zoom of them, and a live mode with an fps readout.
- **Side-by-side:** the reference frame and the render at the same time, full frame or the same crop.
- **Pixel measurement:** mean RGB and 10/50/90 percentiles over a cropped region in both images, filtered by a simple
  class test (green-dominant for grass and foliage, red over green over blue for soil).
- **Probe:** raw animation signals (subject root, feet, camera, props) sampled densely over time, flagging jumps and
  kinks, so every motion change can be shown to be smooth, or to have only the kinks that are physical (takeoff,
  contact).
- **Position readout:** world positions of the subject, camera and key props at given times, read back as floats.
  Use it to reason about framing in numbers instead of by eye.
- **Diff and bench:** before/after image difference per beat, and a median frame time per beat, for any change that
  should not alter the look.

**Test the comparison before trusting it.** Mean colour per region can match while the picture is wrong: a flat grey
frame with the measured patch colours painted on passes a colour-only check.

- **Score renders that must fail.** Before milestone 2, run the comparison on a flat frame painted with the measured
  colours, a single frozen reference frame, and the reference frames shuffled in time. Any check that passes one of
  them is not measuring what it claims.
- **Add a layout signal.** Classify a coarse grid of cells (water, rock, foliage, …) by colour and report the share of
  cells that match the reference at each timeline row. Measure it on the reference itself first (two different
  shots; the same shot 1 s apart) so the number has a scale.
- **Optional: a fresh reviewer.** When your tool can start a subagent, have one with no access to the conversation
  or the source judge the side-by-sides against the section 0 checklist.

## 4. Scene construction

- **Model.** Choose from the measured shot. For sculpted solids, start with `templates/raymarch-sdf.glsl` and its distance/material contract. For a graphic animation, separate the 2D masks and composite in layer order. For a landscape, follow a heightfield example and add solid or volume intersections as required. For a cloud shot, trace the density interval. State which intersections, fields or layers determine each major occlusion. Keep the final implementation single-pass.
- **Scale:** the main subject's height is 1 unit unless the breakdown says otherwise. Place everything from the
  measured relative positions, then tune against the side-by-sides.
- **Bounds and LOD:** a bounding volume around every object group; evaluate detailed SDFs only inside it, and blend
  the bound into the detail near its margin so the field stays continuous (soft shadows read the bound). Reduce noise
  octaves with distance. Cull distant set pieces per ray with `iSphere` or `iBox`.
- **Vegetation and ground cover:** near the camera, real clumps (a traced shell over the ground, for example);
  farther out, shading with patches and streaks. Flowers, bushes and trees are built from their parts (section 0).
- **Static data:** precompute anything constant per frame or per object (heights props stand on, rotations, hashes)
  as `const` arrays, regenerated by a debug mode, never recomputed per marching step.
- Assemble from the library by name (`python3 scripts/glsl_tools.py search <words>`). Never paste or re-derive
  library code, and respect the skill's hard rules (no sine hashes, float literals, NaN guards, no `dFdx` in loops).

## 5. Animation

- Use the reference timebase. For looping clips use `T = mod(iTime + START_TIME, LOOP)`. For one-shot clips clamp to the shot duration. Preserve intentional cuts, holds and fixed cameras. Where the subject calls for analytic motion, use keyed speeds integrated exactly,
  foot planting by phase, closed-form springs for secondary motion, ballistic arcs for jumps and props, and a camera
  follow solved in closed form. Evaluate the subject state once per pixel before marching, not inside `map`.
- Key the timeline to the breakdown: speed keys (time, speed, cadence), event times (jumps, contacts, interactions),
  and camera holds per beat. A pause on the reference (settling on a ledge, gathering for a jump) is part of the
  timing; reproduce it.
- Props that interact with the subject (balls, doors, ropes) are keyed relative to the subject, so retiming the
  subject keeps them in place.

## 6. Camera, light and atmosphere

- **Camera:** take framing, holds, cuts and motion from the camera log. Use a damped follow for tracked shots that show that behavior. Check framing numerically: subject height in frame and horizon line
  against the reference at every timeline row.
- **Light:** key direction and color from the breakdown, sky and bounce fill, soft shadows that never go black, AO.
- **Sky, clouds, haze, god rays, particles:** as the breakdown shows. Cheap 2D or analytic versions are fine where
  the reference treats them as background (section 9).
- **Pipeline:** linear lighting, exposure, a tonemapper chosen by comparing measured saturation with the reference,
  `linearToSrgb`, vignette if the reference has one, and `ditherTPDF`. For looping footage, the loop seam is clean (fade or matched end
  state).

## 7. Milestones

1. **Breakdown and tooling** (sections 1 and 3). Stop for approval.
2. **Blockout and timeline:** stand-in shapes for everything, the subject moving through every beat, the camera
   following. Check every timeline row with `FREEZE_TIME` against the reference.
3. **Motion:** final gait, states, secondary motion, jumps and prop interactions. Prove smoothness with strips and the
   probe.
4. **Subject detail:** the sculpted character or subject, close-ups at 8 phases plus mid-action.
5. **Environment detail:** every object built from its parts, matched by side-by-side and measured color.
6. **Light, color and atmosphere:** matched by sampled pixels.
7. **Reference pass:** walk every timeline row and every object again, and fix the biggest remaining differences.
8. **Performance pass:** keep the look. First exact changes (precomputation, tighter culling, skipping detail that
   cannot win a min or smax), verified by the diff tool to change nothing; then near-exact ones with their measured
   difference reported. Report per-beat times before and after.

At every milestone, render the timeline times next to the reference, plus the strips and close-ups from section 0;
then critique, fix and re-render, at least 3 passes, before moving on.

## 8. Controls and deliverables

- **Controls at the top:** `DEBUG_MODE` (skill conventions, plus a timeline overlay, motion curves and raw motion
  signals), `AA`, toggles for expensive features, `START_TIME`, `FREEZE_TIME` (≥ 0 freezes on that time), and named
  constants for every art control (timing keys, speeds, camera offsets and holds, FOV, sun, palette, fog).
- **Deliverables:**
  - `<name>.glsl` (readable source calling library functions) and the bundled `<name>.out.glsl` from
    `python3 scripts/glsl_tools.py bundle <name>.glsl -o <name>.out.glsl`;
  - `python3 scripts/glsl_tools.py check <name>.out.glsl` passing, saying whether glslangValidator ran;
  - `REFERENCE.md` (section 1) and a README with a tuning guide, the simplifications and why, the section 0
    checklist marked pass or fail with its evidence, measured color matches, and fps per beat.

## 9. Simplifications

Allowed without asking: no depth of field, no motion blur; background clouds and birds as 2D layers; faces left
unmodelled when the reference never shows them.

Everything else needs approval first. That includes shading-only ground cover, decal-only props, fewer or simpler
background objects, dropping shadows or detail for speed, and any change to the timeline or camera that departs from
the reference.
