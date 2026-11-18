# Research studies for procedural art

Choose a technique by the change that should remain visible in motion. The offline viewer exposes the camera elevation, wingbeat, optical period, side light, cylinder radius and reaction rates for the relevant studies. Build it from the installed skill with `python3 scripts/build_viewer.py studies.html` and open the file in a WebGL2 browser.

## A black hole with a visible far side

Start with `templates/lensed-disk.glsl`. A null geodesic stays in a plane through the black hole. In units of the Schwarzschild radius, set (u=1/r) and integrate (u''=1.5u^2-u) with respect to orbital angle. A radial basis and its perpendicular in that plane reconstruct the ray in 3D. Disk-plane crossings recur every π radians. Evaluate the radius at the crossing angle, accept emitting material between 3 and 13 Schwarzschild radii, and continue tracing through the empty inner gap.

Lower the camera elevation until the direct face becomes a narrow band. The back of the disk should still arch above the shadow. In diagnostic view 3, blue marks an emitting crossing after at least half a turn. The GPU tests bracket the capture threshold on both sides of the critical impact parameter. The material uses an artistic radial temperature profile and orbiting filaments. A spinning black hole requires a different spacetime. Bruneton uses precomputed beam tracing and pixel integration for stellar images. This example uses fixed-step RK4, one ray per pixel and procedural stars. [Bruneton, 2020](https://arxiv.org/abs/2010.08735)

**Animate the gas at a fixed camera.** In the viewer, enable **Cache light paths**. A full-resolution RGBA32F texture retains either the disk intersection and frequency shift, the escape direction, or a capture tag. New animation times only evaluate emission and background shading. Editing camera elevation or resolution invalidates the texture. A 100-frame fixed-camera shot integrates each pixel’s orbital path once. Pixel shading, sampling and display still run on every frame. In a 100-frame check on Chromium SwiftShader at 480 × 270, the direct and cached outputs matched in all 8-bit RGB channels. Warm median frame time was 9.4 ms direct and 2.5 ms cached, including readback. The bake took 103 ms in that run. Reproduce with `python tests/check_viewer.py --disk-frames 100` from the repository. For continuous camera motion use direct tracing or a camera-independent transfer table.

**Try another shot.** Keep the camera nearly edge-on, reduce the disk’s outer radius and use diagnostic view 3 to isolate the secondary image before grading it. For a high-contrast starfield, supersample the background where the lens compresses it into arcs.

## A butterfly that flashes blue

Start with `templates/morpho.glsl`. The leading and trailing wings have separate outlines. Veins and the dark border belong to the wing coordinates. The blue return responds to the tilt of each wing, local curvature, and variation among scale orientations. Its peak wavelength follows an effective periodic-stack model. The sample maps that peak to a restricted blue–cyan palette.

Pause the viewer and change the layer period. The blue membrane should change while the border stays dark. Restart the wingbeat and watch the two sides flash at different times. A full Morpho scattering model would include the distribution of lamellar heights and directions. Scale curvature also changes how widely a flash is visible. [Kinoshita et al., 2002](https://forma.katachi-jp.com/abstract/1702/17020103.html) · [Morpho wing-scale curvature study](https://doi.org/10.1098/rsbl.2024.0358)

**Try another shot.** Make a close crop of one wingtip at grazing incidence. Broaden the distribution of scale tilts while keeping the dark apical spots fixed. Compare images at the same point in the wingbeat.

## A comb jelly under side light

Start with `templates/comb-jelly.glsl`. An analytical ellipsoid gives both interfaces and the length of the underwater light path. Evaluate eight meridians at each interface. Vary plate orientation along latitude in a traveling wave. Composite the far rows through the gel first and add the near rows last. The study uses a procedural spectral palette to represent the color change caused by the beating cilia.

Set **Side light** to zero while paused. The colored rows disappear and the surrounding water and body rim remain visible. In view 3, red and green distinguish the two interfaces. The rim models scatter and the offset background approximates weak refraction. The measured organ has a coherent ciliary structure. The shader resolves the row pattern at body scale. [Optical properties of the iridescent organ of Beroë cucumis](https://doi.org/10.1103/PhysRevE.73.041916)

**Try another shot.** Rotate toward the long axis and track one wave from pole to pole with a narrow moving light. Keep rear-side contrast lower so the direction of travel stays legible.

## An engraving made from a velocity field

Start with `templates/streamline-press.glsl`. A uniform stream plus a doublet gives tangential flow at the boundary of a cylinder. Integrate short streamlines forward and backward from each pixel. Average paper noise along those paths with a tapered kernel. This produces marks aligned with the flow even where it turns sharply. Use physical speed separately from tracing direction: direction determines the engraving strokes and the potential controls moving accents.

Change the radius while watching the stagnation points. Strokes should bend around the obstacle and realign downstream. The symmetric field describes steady inviscid flow. To depict alternating shed vortices, supply a time-dependent velocity field or a simulated velocity texture. `QUALITY 1` halves the tap count in both tracing directions. [Cabral and Leedom, 1993](https://www.osti.gov/servlets/purl/10185520-hIb0EP/) · [MIT cylinder-flow derivation](https://web.mit.edu/fluids-modules/www/potential_flows/Lectures/lec10/lecture10.pdf)

**Try another shot.** Replace the velocity field with wind sampled from a map, then paint paths crossing a single mountain pass. Keep the source noise stable over time and use the kernel length to control the stroke length.

## Grow a pattern with feedback

Use `multipass/reaction-diffusion/`. Buffer A stores concentrations U and V in red and green. One Euler step adds diffusion from a four-neighbor Laplacian, the reaction UV², feed and removal. The image pass derives normals from concentration differences and lights the pattern like raised ink. The viewer advances five fixed steps per displayed frame. Reset replays the deterministic initial field. A pointer stroke seeds new material.

In Shadertoy, bind **Buffer A to its own channel 0**, then bind **Buffer A to Image channel 0**. Use floating-point buffers, integer fetches for the solver and linear filtering for the display. Enter the two bundled files in their corresponding passes. A single displayed frame advances one solver step on Shadertoy. Preserve separate read and write textures in WebGL. The viewer uses a 320 × 180 simulation grid, independent of display resolution.

Change feed or removal while running. Give the field several hundred steps to respond. The test compares the initial state with step 400, checks that the concentrations remain finite and checks reset against the original field. Parameter values depend on the time step, diffusion coefficients and stencil. [Pearson, Complex Patterns in a Simple System](https://arxiv.org/abs/patt-sol/9304003)

**Try another shot.** Keep the same initial seed and render the field once as flat pigment and once with raking light. This isolates pattern formation from the material used to show it.

## Cache a landscape across camera motion

Use `python3 scripts/build_alpine_cache.py output-directory` to generate the two passes from the alpine scene. Bind Buffer A to its own channel 0 and to Image channel 0. Use linear filtering on Image, clamp wrapping, floating-point storage and no mipmaps. Buffer A saves the world-space height field and copies it on subsequent frames. The offline viewer retains a single texture and skips the copy.

This example uses one fixed rectangle. Inside it, bilinear interpolation changes fine ridges and consequently some normal and shadow samples. Outside it, the image shader evaluates the original terrain. Elevation textures are also the underlying representation in GPU geometry clipmaps, where nested levels update around the camera. For a much longer route, that moving representation would avoid falling back to procedural evaluation. [Asirvatham and Hoppe, GPU-based geometry clipmaps](https://hhoppe.com/proj/gpugcm/)

To compare a change, run `python tests/compare_alpine_cache.py` from the repository root. It records bake time, feedback-copy time, paired warm timings, absolute RGB error and side-by-side images at three camera positions. Compare on the target GPU at the intended display and atlas resolutions.
