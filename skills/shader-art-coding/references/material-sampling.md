# Stochastic material sampling and local masks

Use the [patinated brass template](../templates/patinated-brass.glsl) for surfaces that combine repeating stock material with placed details. Timber, plaster and glazed ceramic can use the same separation. The example uses an analytic tile so the standalone shader needs no textures.

## Inspect the panel

Switch **Stochastic grain** between 0 and 1 in the offline viewer. At 0 the same brush marks recur across the panel. At 1 each lattice vertex supplies a rotation and offset, and the containing triangle blends its three samples. Adjacent triangles share the two seeds along their common edge. Squared and renormalized weights keep more local contrast. The engraving stays in panel coordinates as the grain changes.

Select **Grain / recess / patina**. Red is the grain-derived deposit mask, green is the engraved recess, and blue is their product. Blue must vanish on the flat brass wherever green is zero. To diagnose a lost mask during a material port, temporarily substitute the red channel for the product and compare the same flat patch. Play the animation to move the light. All three masks stay attached to the panel.

Select **Relief normals** to see how the same grain shapes the lit surface. The template differentiates the final blended height, including changes in blend weights. Keep derivative evaluation outside divergent control flow.

## Using image textures

For a texture-backed version, reuse each vertex's UV transform for the color, normal and roughness samples. Transform the pixel gradients along with the UVs and supply them to `textureGrad` to select the mip from the continuous footprint. Hash offsets and chart changes can corrupt an implicit footprint at boundaries. Rotate sampled tangent-space normal offsets back into the common surface frame before blending and normalizing. Filter color in linear light and treat masks as linear scalar data.

Keep lettering, joints and trim in their own authored coordinates. If baking a repeating primary-UV mask into a secondary atlas, use the mesh triangles to map corresponding points. Two faces can share the same secondary UV while requiring different primary-mask values. Give those faces distinct output charts before baking and pad their borders for filtering.

The example trades three evaluations of a small periodic pattern for less repetition. Compare the aligned surface at a fixed output size and under a grazing light before applying the same cost to several texture layers. [SmartImposter ground sampler](https://github.com/SamG-Coder/SmartImposter/blob/34d4cb1/shaders/tri.rchit) · [KingdomComeGlueMapper two-UV audit](https://github.com/SamG-Coder/KingdomComeGlueMapper/blob/47f031a/docs/building-material-audit.md)
