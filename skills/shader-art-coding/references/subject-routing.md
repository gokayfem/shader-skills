# Subject Routing

Start here when the request names a subject ("a jellyfish", "a snowy mountain", "a neon logo"). Each row gives the
rendering model, the library functions that do the heavy lifting, the template to start from, and the reference
section with the formulas. The formula files use geometry-first names; this table maps them to subjects.

## Contents

- [Choosing a rendering model](#choosing-a-rendering-model)
- [Characters and creatures](#characters-and-creatures)
- [Plants and nature objects](#plants-and-nature-objects)
- [Landscapes, sky and weather](#landscapes-sky-and-weather)
- [Built and manufactured things](#built-and-manufactured-things)
- [Materials and surfaces](#materials-and-surfaces)
- [2D graphics and motion design](#2d-graphics-and-motion-design)
- [Formal names to subjects](#formal-names-to-subjects)

## Choosing a rendering model

| The image is... | Model | Template |
|---|---|---|
| flat graphic, poster, icon, logo, UI, pattern | 2D distance fields + `fillAA` | `sdf-2d-poster.glsl`, `pattern-swatches.glsl` |
| sculpted 3D objects, characters, still lifes | SDF raymarching | `raymarch-sdf.glsl` |
| landscape, terrain | heightfield march (scaled bound) | `terrain.glsl` |
| sea, ocean, lake surface | heightfield trace in a wave slab + Fresnel reflection | `ocean.glsl` |
| clouds, smoke, fog volumes, nebulae, fire | volume integration | `volumetric-clouds.glsl` |
| Morpho, iridescent butterfly wing | absorbing wing pigment under angle-dependent scale return | `morpho.glsl` |
| sea gooseberry, comb jelly | analytic optical path, eight moving comb rows, back-to-front compositing | `comb-jelly.glsl` |
| black hole, accretion disk | null-ray orbital integration and disk-plane intersections | `lensed-disk.glsl` |
| flow engraving, wind map, streamline print | line integral convolution of paper noise | `streamline-press.glsl` |
| woods, meadow edges, riparian vegetation | Stable founder groves, moisture-dependent species and a crown-derived understory | `woodland-edge.glsl`; [vegetation habitats](vegetation-habitats.md) |
| growing spots, labyrinth patterns, paintable chemical field | persistent two-channel reaction-diffusion | `multipass/reaction-diffusion/` |
| static terrain on a moving camera | feedback height atlas and procedural fallback | `multipass/alpine-cache/` |
| agate, gemstone cross-section, mineral strata | warped contours, filtered growth bands, Voronoi quartz faces | `banded-agate.glsl` |
| patina, layered wear, breaking up repeated surface grain | Blend three seeded samples on a triangular lattice. Keep object-local wear in a separate coordinate system. | `patinated-brass.glsl`; [material sampling study](material-sampling.md) |
| texture, material swatch, wallpaper | 2D procedural pattern | `pattern-swatches.glsl` |
| comparing noise families / learning noise | 2D scalar-field board | `noise-progression.glsl`; `noise-use-cases.md` |
| game heightmap, biome map, designed river | 2D height + separate masks | `noise-terrain-map.glsl`; `hybridTerrainHeight`; `noise-use-cases.md` |
| city, skyscraper, procedural building facade | SDF massing + filtered material fields | `procedural-city.glsl`; `procedural-buildings.md`; `buildingProfile`, `facadeWindowMask` |
| alpine valley, snow/rock biome, production terrain | designed heightfield + material masks | `alpine-valley.glsl`; `terrain-production.md` |
| clear coast, lagoon, shallow refractive water | water/land heightfields + secondary rays | `coastal-water.glsl`; `water-quality.md`; `waterWaveD`, `fresnelDielectric`, `beerTransmittance` |

## Characters and creatures

| Subject | Build with | Formulas |
|---|---|---|
| head, body, torso | `sdEllipsoid` masses joined with `smin`; `sdRoundCone` for neck and snout | couples-sdf 17 (bilobed carrier), cookbook "Bilateral Organic SDF Composition" |
| eye sockets, nostrils, mouth corners | `sdDeathStar` or `smoothSubtract` of an ellipsoid | couples-sdf 18 (paired concavity) |
| eyes | `sdSphere` + analytic `iSphere` for glossy cornea; iris from `voronoi` / radial `cubicPulse` | couples-materials 18 (radial ring field) |
| eyelids, lips, eyebrows | `sdCappedTorus`, `sdArc` (2D), `sdCutHollowSphere` shells | couples-sdf 19 (aperture curve), 22 (clipped shell) |
| limbs, fingers, tails, tentacles | `sdRoundCone`, `sdCapsule`, 3D `sdQuadraticBezier` tubes | sdf-library-guide "Booleans and smooth blends" |
| hair, fur, whiskers | 3D `sdQuadraticBezier` strands, `curlNoise` for flow, anisotropic highlights | couples-sdf 21 (strand perturbation) |
| skin | `wrapDiffuse`, `translucency` + `sdfThickness`, red-tinted terminator | rendering-pipeline "Lighting" |
| cloth, knit | `fabricDisplace`-style micro displacement (stepScale 0.7) | couples-sdf 23 |
| jellyfish, ghosts, glass creatures | `sdCutHollowSphere`, `opOnion`, `translucency`, additive glow | couples-sdf 22 |

## Plants and nature objects

| Subject | Build with | Formulas |
|---|---|---|
| flower, petals | `domPolar` + `sdUnevenCapsule` / `sdVesica`; neighbor-checked for varied petals | sdf-library-guide "Repetition" |
| leaves | `sdVesica` (2D) extruded with `opExtrude`, veins from `voronoiBorder` | couples-materials 12 |
| trees (stylized) | `sdCone` tiers + `sdCylinder` trunk; forests with neighbor-checked repetition | couples-sdf 24-25 |
| grass, foliage clumps | density fields placed on a grid with suitability masks | couples-terrain 19-21 |
| rocks, pebbles | `sdRoundBox` / `sdEllipsoid` + `fbm` displacement, `voronoi` facets | sdf-library-guide "Deformation" |
| coral, sponge, bone | `sdGyroid` intersected with a bounding shape | library-index (sdGyroid) |
| shells, horns | `sdCappedTorus`, `domTwist` (divide by L), `opOnion` | sdf-library-guide "Deformation" |
| fruit | spheres and ellipsoids, `smoothSubtract` dimples, `voronoi` skin pores | couples-sdf 9 |

## Landscapes, sky and weather

| Subject | Build with | Formulas |
|---|---|---|
| mountains | `erodedFbm` height, `ridgedFbm` crests, slope/altitude masks | couples-terrain 1-12, `terrain.glsl` |
| snow | upward-facing accumulation mask (`n.y`) + altitude + noise breakup | couples-terrain 22 |
| desert dunes | `ridgedFbm` along one axis + `warpedFbm`, low sun | couples-terrain 23 (layered ridge) |
| ocean, lakes | `ocean.glsl`: dispersive exp-sine waves with analytic slope and curvature, `fresnelSchlick` sky reflection, GGX glint with distance roughness, curvature foam; calm lakes: `iPlane` + `simplexNoiseD` ripples | rendering-pipeline "Lighting" |
| sky, sunset | `atmosphereSky` (physical) or `simpleSky`; `sunDisk` | rendering-pipeline "Sky, fog and aerial perspective" |
| clouds | `volumetric-clouds.glsl`: coverage fBm, `beerPowder`, `phaseDualHG`, `volumeStep` | rendering-pipeline "Volumes" |
| fog, mist, haze | `distanceFog`, `heightFog`, aerial perspective with horizon-clamped sky | couples-terrain 13-14 |
| night sky, stars | per-cell `hash32` stars, `blackbody` colors, twinkle with `sin(iTime)` | `sdf-2d-poster.glsl` |
| fire, lava, embers | `turbulence`, `warpedFbm` / `curlNoise` (2D or 3D), `simplexNoiseD` with animated rotation for churning flow, `blackbody(1200-1800 K)`, additive glow, AgX | rendering-pipeline "Volumes" |
| rain, snowfall | repeated streaks with `domRepeat` + per-cell `hash12` phase | sdf-library-guide "Repetition" |

## Built and manufactured things

| Subject | Build with | Formulas |
|---|---|---|
| buildings, rooms | `sdBox` / `sdRoundBox`, `opSubtract` windows with `domRepeatLimited` | sdf-library-guide "Repetition" |
| brick or stone tower | radial row-cell space, staggered rows, interstitial mortar shell | couples-sdf 13-16, cookbook "Cylindrical Modular Cell Lattice" |
| machinery, sci-fi panels | `opChamferUnion`, `opEngrave` panel lines, `sdBoxFrame` | library-index (operators) |
| stairs, terraces, ziggurats | `opStairsUnion` | library-index (operators) |
| columns, pillars | `sdCylinder` + `domPolar` flutes, `domRepeatLimited` colonnades | sdf-library-guide "Repetition" |
| vases, bottles, chess pieces | 2D profile + `opRevolve` | sdf-library-guide "From 2D to 3D" |
| chains, rings, jewelry | `sdLink`, `sdTorus`, metal `shadePBR(metal = 1)` | rendering-pipeline "Color and materials" |
| text-like logos, badges | 2D SDFs + `opExtrude`, `sdStar`, `sdRegularPolygon` | sdf-library-guide "From 2D to 3D" |
| glass objects | analytic `iSphere`/`iBox` + `fresnelSchlick` + refracted sky | rendering-pipeline "Lighting" |

## Materials and surfaces

| Subject | Build with | Formulas |
|---|---|---|
| marble, agate | `warpedFbm` (color from q and r) | `pattern-swatches.glsl` |
| wood | rings: `fract(radius * k + fbm)`, grain stretched along one axis | couples-materials 18 |
| cracked mud, crackle glaze | `voronoiBorder` (constant-width cracks) | `pattern-swatches.glsl` |
| tiles, honeycomb, scales | `hexGrid` + `hexEdgeDistance` bevels, per-tile `hash12` color | `pattern-swatches.glsl` |
| checkerboard floors | `filteredChecker` with ray-footprint width | `raymarch-sdf.glsl` |
| rust, dirt, wear | `fbm` masks biased by gravity and curvature, edge wear from AO | couples-materials 15-16 |
| metal, plastic, ceramic | `shadePBR` roughness/metallic, `envBRDFApprox` sky reflections | rendering-pipeline "Lighting" |
| weaves, mazes, circuits | `truchetArcs`, `domRepeatMirror` | `pattern-swatches.glsl` |

## 2D graphics and motion design

| Subject | Build with | Formulas |
|---|---|---|
| poster, landscape silhouette | ridges from `fbm` lines, layered back to front, `mixOklab` sky | `sdf-2d-poster.glsl` |
| icons, UI shapes | `sdRoundedBox` (per-corner), `sdCircle`, `strokeAA` | couples-materials 3-4 |
| loaders, gauges, rings | `sdArc`, `opOnion`, time-driven aperture | couples-materials 25-26 |
| charts, badges, signage | `sdPie` (pie charts, radar sweeps), `sdHorseshoe` (magnets, U-bends), `sdParabolaSegment` (arches, domes), `sdEgg` (eggs, drops, cartoon heads) | library-index (sdf2d) |
| kaleidoscopes, mandalas | `domPolar`, `domFold`, `cosinePalette` | sdf-library-guide "Repetition" |
| retro sunsets, synthwave | `filteredStripes`, `filteredGrid` in perspective, `tonemapNeutral` | `sdf-2d-poster.glsl` |
| looping animation | integer-frequency `cos(TAU * t)`, `cosinePalette` with integer c | couples-materials 25 |

## Formal names to subjects

The formula files name operators by geometry so they stay reusable. Typical subjects:

| Formal name | Typical subjects |
|---|---|
| bilobed (organic) carrier | head + cranium, torso + pelvis, gourd, snowman, peanut |
| paired concavity carve | eye sockets, nostrils, dimples |
| parametric arc aperture curve | eyelids, mouths, smiles, slits |
| soft parametric groove | lip line, crease, seam, wrinkle |
| anisotropic strand field | hair, fur, grass blades, fibers |
| ellipsoidal shell clipped by half-space | eyelid, helmet, beetle shell, hood |
| radial row-cell space | brick tower, beehive, pinecone, shingled roof |
| interstitial shell | mortar, grout, membrane between cells |
| accumulation mask | snow, dust, moss on top surfaces |
| coverage field | clouds, foliage canopy, fog banks |
| sparse stochastic spot field | freckles, stars, sparkles, pores |
| stratified material channels | rock / grass / snow layering |
