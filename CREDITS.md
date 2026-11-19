# Credits and Sources

All GLSL in this repository is an original implementation, released under the MIT license (see LICENSE). The
techniques come from the published work below. Numeric constants (matrices, fit coefficients, physical
coefficients) are facts reproduced from their sources. No third-party shader source is copied; Shadertoy shaders
default to CC BY-NC-SA 3.0, which is incompatible with MIT.

| Technique | Source | License or terms of the source |
|---|---|---|
| SDF primitives, operators, smooth minimum, domain repetition, normals, soft shadows, intersectors, fog, palettes, filtered patterns, Voronoi borders, noise derivatives, domain warping | Inigo Quilez, iquilezles.org/articles | articles state no license; ideas reimplemented |
| Chamfer, stairs and engrave combinators, polar and mirrored repetition | Mercury demogroup, hg_sdf (mercury.sexy/hg_sdf) | MIT OR CC BY-NC 4.0 |
| PCG hashes (pcgHash, pcg3d, pcg4d) and hash quality data | Jarzynski and Olano, "Hash Functions for GPU Rendering", JCGT 9(3), 2020 | paper CC BY-ND 3.0; arithmetic reimplemented |
| Interleaved gradient noise | Jorge Jimenez, "Next Generation Post Processing in Call of Duty: Advanced Warfare", SIGGRAPH 2014 | constants are facts |
| R2 low-discrepancy sequence | Martin Roberts, "The Unreasonable Effectiveness of Quasirandom Sequences" | constants are facts |
| Habitat-based stand placement and canopy-driven understory | [SmartImposter forest generation](https://github.com/SamG-Coder/SmartImposter/blob/34d4cb1/src/world.rs) | MIT; original stateless shader study |
| Curl noise | Bridson, Hourihan and Nordenstam, SIGGRAPH 2007 | math |
| Enhanced (over-relaxed) sphere tracing (documented, not shipped) | Keinert et al., STAG 2014 | math |
| AgX tonemapping (minimal polynomial version) | Troy Sobotka (AgX); Benjamin Wrensch / iolite (polynomial fit) | MIT |
| Khronos PBR Neutral tonemapping | Khronos Group, github.com/KhronosGroup/ToneMapping | Apache-2.0 (constants) |
| ACES fitted (RRT+ODT) | Stephen Hill, via MJP's BakingLab | MIT |
| Oklab / OkLCh | Bjorn Ottosson, bottosson.github.io/posts/oklab | public domain / MIT |
| Blackbody chromaticity | Krystek 1985 Planckian-locus fit | math |
| GGX, height-correlated Smith visibility, Schlick Fresnel | Filament documentation (Google) | Apache-2.0 (formulas) |
| Environment BRDF approximation | Brian Karis, "Physically Based Shading on Mobile" (after Lazarov) | constants are facts |
| Energy-conserving wrap lighting | Stephen Hill, "Righting Wrap" (2011) | math |
| Fast translucency | Barre-Brisebois and Bouchard, GDC 2011 | math |
| Energy-conserving volume integration, sky constants | Sebastien Hillaire, Frostbite (SIGGRAPH 2015/2016) and EGSR 2020 | math, constants are facts |
| Beer-Powder cloud lighting, dual-lobe phase | Andrew Schneider, Horizon Zero Dawn clouds (SIGGRAPH 2015) | math |
| Sun limb darkening | Neckel 1996 | constants are facts |
| Cornette-Shanks phase function | Cornette and Shanks 1992 | math |
| Simplex noise and its analytic gradient | Stefan Gustavson, "Simplex noise demystified" (2005); Gustavson and McEwan, "Tiling simplex noise and flow noise in two and three dimensions", JCGT 11(1), 2022 | papers; math reimplemented, the psrdnoise reference code is not used |
| Flow noise (rotating gradients) | Perlin and Neyret, "Flow Noise", SIGGRAPH 2001 technical sketch | math |
| Footprint-filtered fBm | [Pharr, Jakob and Humphreys, PBRT 3e, Noise](https://pbr-book.org/3ed-2018/Texture/Noise) | octave clamping concept, original implementation |
| Turbulence | Ken Perlin, "An Image Synthesizer", SIGGRAPH 1985 | math |
| Gradient-noise learning reference | [Ken Perlin, improved noise](https://cs.nyu.edu/~perlin/noise/) | reference only; fixed-permutation Java code not copied |
| Noise progression: fBm, ridges, cellular construction, domain warping | [Gonzalez Vivo and Lowe, The Book of Shaders, chapters 12](https://thebookofshaders.com/12/) and [13](https://thebookofshaders.com/13/) | explanatory sources; no source code copied |
| External noise code catalog | [Gustavson / Ashima, webgl-noise](https://github.com/stegu/webgl-noise), [psrdnoise](https://github.com/stegu/psrdnoise), [Auburn, FastNoise Lite](https://github.com/Auburn/FastNoiseLite) | MIT alternatives linked in the use-case guide; not vendored |
| Billow terminology | [libnoise Billow documentation](https://libnoise.sourceforge.net/docs/classnoise_1_1module_1_1Billow.html) | reference only; code not copied; local weighted [0,1] billow differs |
| Hybrid terrain height and designed river composition | this repository, `hybridTerrainHeight` / `noise-terrain-map.glsl` | original MIT composition; not a claim to implement Musgrave's hybrid multifractal or hydrological erosion |
| Sum-of-sines water with sharpened crests, deep-water dispersion | Mark Finch, "Effective Water Simulation from Physical Models", GPU Gems (2004), chapter 1 | math |
| Filtering vanished normal detail into roughness | Toksvig, "Mipmapping Normal Maps" (2005); Olano and Baker, "LEAN Mapping" (I3D 2010) | math |
| Four-arc (Moss) egg | classical compass construction | geometry |
| Procedural building massing hierarchy | [Müller et al., Procedural Modeling of Buildings, SIGGRAPH 2006](https://doi.org/10.1145/1141911.1141931); [SIGGRAPH 2011 urban modeling course](https://www.cs.purdue.edu/cgvlab/urban/sg_2011_course/contents.html) | concepts reimplemented; not a CGA engine or copied grammar/code |
| Filtered facade window coverage | Quilez, filterable procedurals / analytic footprint integration | original periodic rectangle integral; no source copied |
| Terrain representation and density/meshing distinction | [Geiss, GPU Gems 3 chapter 1](https://developer.nvidia.com/gpugems/gpugems3/part-i-geometry/chapter-1-generating-complex-procedural-terrains-using-gpu) | explanatory source; local heightfield shaders do not implement the chapter's meshing pipeline |
| Finite-depth gravity-wave dispersion | [MIT Marine Hydrodynamics lecture 20](https://ocw.mit.edu/courses/2-20-marine-hydrodynamics-13-021-spring-2005/5d48a5937971d973fd8ca90c051a83f8_lecture20.pdf) | mathematical relation, original implementation |
| Exact dielectric Fresnel and Beer-Lambert attenuation | [PBRT reflection/transmission](https://www.pbr-book.org/4ed/Reflection_Models/Specular_Reflection_and_Transmission), [transmittance](https://www.pbr-book.org/4ed/Volume_Scattering/Transmittance) | equations reimplemented; no PBRT source copied |
| Coastal refraction pipeline comparison | [GPU Gems 2 chapter 19, Generic Refraction Simulation](https://developer.nvidia.com/gpugems/gpugems2/part-ii-shading-lighting-and-shadows/chapter-19-generic-refraction-simulation) | reference for scene-buffer alternatives; local coastal shader traces a procedural heightfield instead |


## Research studies

| Study | Primary source | Local implementation |
|---|---|---|
| Morpho structural color | [Kinoshita et al.](https://forma.katachi-jp.com/abstract/1702/17020103.html), [wing-scale curvature study](https://doi.org/10.1098/rsbl.2024.0358) | Original cubic outlines, procedural scale response and effective optical-period model |
| Comb-row color | [Optical properties of the iridescent organ of Beroë cucumis](https://doi.org/10.1103/PhysRevE.73.041916) | Eight animated rows on two analytic interfaces, artistic RGB diffraction palette |
| Schwarzschild disk | [Bruneton](https://arxiv.org/abs/2010.08735) | Null-ray ODE, original RK4 tracer, disk material and fixed-camera transfer texture |
| Streamline engraving | [Cabral and Leedom](https://www.osti.gov/servlets/purl/10185520-hIb0EP/), [MIT cylinder flow](https://web.mit.edu/fluids-modules/www/potential_flows/Lectures/lec10/lecture10.pdf) | Bidirectional midpoint integration, tapered noise convolution and procedural print |
| Reaction–diffusion | [Pearson](https://arxiv.org/abs/patt-sol/9304003) | Original finite-difference ping-pong simulation and relief display |
| Elevation texture reuse | [Asirvatham and Hoppe](https://hhoppe.com/proj/gpugcm/) | Fixed rectangular cache for the existing alpine scene |

## World studies

The patch lighting studies use the classical diffuse interreflection formulation of Goral, Torrance, Greenberg and Battaile, [Modeling the Interaction of Light Between Diffuse Surfaces](https://doi.org/10.1145/800031.808601). The templates approximate patch transfer with center-to-center visibility and bounded diffuse weights. The complete world distributions carry their original notices in their respective LICENSE files under worlds/.
