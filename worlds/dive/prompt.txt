Build a real-time fantasy underwater scene that runs in the browser. It only needs to look beautiful and let me fly a free camera through it.

PRIORITIES (in this order when they conflict)
1. Visual quality
2. Stable frame pacing at the target spec
3. Small bundle size

CONSTRAINTS
- No Three.js/Babylon.js or other engines.
- WebGPU rendered.
- Everything procedural: no texture/model files. Deliverable is a single self-contained index.html. Report minified size.
- Zero per-frame JS allocations (no GC stutter). Preallocate all buffers.

TARGET
- Windows 11 25H2, Chrome 157, RTX 5070 Ti, Ryzen 7800X3D, 180 Hz monitor
- 2560×1440 at full resolution. GPU frame time ≤ 11 ms average, no frame > 16.7 ms during a scripted 60 s fly-through.

SCENE
- Dense, towering kelp columns with sparkling green/gold specks, swaying
- Strong depth fog/absorption: cyan near the surface → deep blue in the distance
- God rays, animated caustics, wavy surface visible from below
- Rock canyons, arches and overhangs; floating boulders
- Iridescent purple/pink foliage, magenta ground cover, barrel sponges, coral
- Animated life: manta rays, fish schools, rising bubbles, drifting particles
- Set pieces: a sunken ship hull, a stone whale-fluke sculpture
- World: bounded, but the boundaries can't be visible from the starting area

CONTROLS
WASD move, mouse look (pointer lock), Q/E up/down, Shift to accelerate

VERIFICATION
- Visual critic: a separate subagent that sees ONLY the your renders, never your code or reasoning. Give it 6 fixed camera poses, rendered at 2560×1440 in GPU-accelerated Chrome, plus a short recording of the fly-through to catch shimmer, aliasing, popping and banding. It returns prioritized, concrete defects. Iterate until it reports nothing above "minor".
- Performance: GPU timestamp queries plus a frame-time log from the scripted fly-through. Report average, 99th percentile and max.

DONE WHEN: both verification gates pass. Deliver index.html, the bundle size, the perf report and the critic's final review.