# Building a world around a shot

Start at the camera and choose what needs to survive movement. Each of the following studies has a working template or pair of passes, an offline viewer control and a diagnostic view.

| Shot | Representation and controls | A concrete review |
|---|---|---|
| Swim between kelp | `kelp-passage` layers blade silhouettes, wildlife and suspended particles. Increase Current strength and then Sun shafts independently. | Hold the frame on a blade base. The tip should travel farther than the base. Use the kelp/wildlife/shafts view to check that the manta still reads through overlapping fronds. |
| Drift over sponges | `sponge-shelf` gives each barrel an open mouth, wall, lip and pores. Rim fluorescence controls the lip response. | Switch to normals. The inner wall should turn away from the outer wall. If the mouth looks painted on, tune its radius and thickness before increasing fluorescence. |
| Inspect a submerged wreck | `ribbed-wreck` traces a keel, transverse ribs and mast. Orbit angle changes the silhouette, Dive depth changes the view and Water attenuation controls the medium. | Orbit until ribs overlap. Use Trace cost to locate slow rays, particularly at grazing angles. Compare the structure against open water and against the seabed before tuning caustics. |
| Grade a coast over a day | `tide-observatory` combines sun, cloud cover, a phase-lit moon, stars and a lighthouse. | Compare twilight and midnight at fixed cloud cover. Sweep lunar phase while paused. Check the limb silhouette separately from the terminator. |
| Cross a forest detail boundary | `cloudline-forest` layers seven ridges. A projected branch error blends six explicit boughs into a crown proxy. | Lower Branch error to reveal more boughs. In the detail view, inspect adjacent trees across the transition, then resize and compare coverage at the same camera position. |
| Examine color bleeding | `pigment-chamber` evaluates an area light and a one-bounce approximation on two colored walls. Move the lamp with Wall bounce at zero, then restore bounce. | Compare the channels in Direct / left / right bounce. The sphere can occlude a lighting segment even where both endpoints face each other. |
| Retain diffuse transport | `room-radiosity` stores 80 wall patches in a 20 × 4 floating-point texture. Frame zero seeds direct light. Three further Jacobi passes gather from the previous texture. | Move the lamp while paused. Wall colors should update immediately. Set Indirect contribution to zero to isolate direct illumination. Keep ping-pong textures separate and sample only the previous iteration. |

## Representation and cost

**Keep density and shading separate.** Establish large empty passages before distributing plants, silt or rocks. Give depth cues different scales of motion. A nearby blade can swing through the frame while a distant ridge shifts little. If turning fog off makes the scene unreadable, adjust silhouettes and placement first.

**Choose where to pay for light transport.** A one-bounce analytic shader works in a single Image pass. With feedback available, hold outgoing diffuse radiance at each patch. Weight transfer by the emitter area, facing cosines, visibility and inverse squared distance. Clamp the sum of approximate weights to conserve energy on nearby patches. An emissive panel moving across the ceiling invalidates the solution. Exposure and camera motion can reuse it when the geometry and lighting remain fixed. The coarse grid smooths small contact detail, which still needs direct visibility at the shaded point.

**Budget geometry in screen space.** Estimate the pixel size of the branch that a simpler crown would omit. Cross-fade the two silhouettes over an error interval. Apply wind to their shared parent coordinate before evaluating either shape so the transition follows the same motion. Inspect the proxy view at two resolutions.

**Separate dimensions of quality.** For a GPU-limited dive, compare shaft steps, shadow resolution, geometry density and output resolution separately with the same camera. In a streaming landscape, profile the first visit to a region separately from the return visit. Record viewport size, warm-up frames and camera path with frame time.

## Working at engine scale

In a full shader-skills checkout, open `worlds/README.md` for a subsystem map and `worlds/catalog.md` for the complete file index. The dive includes world construction, WebGPU materials, camera collision, quality levels and procedural audio. The landscape includes clustered geometry, streaming, weather, GI, water, a bake pipeline and a browser editor. Installed skill users can use all seven studies directly from this package.
