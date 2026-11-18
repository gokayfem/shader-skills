// Specular antialiasing. Where a surface's direction changes from one pixel to the next, as on
// relief finer than a pixel, one pixel shows one of the many facets under it, and a smooth
// material turns each into a mirror: single pixels flash with whatever bright thing their facet
// happens to face, and flash differently every frame. Spread over the pixel, those facets
// reflect as a rougher surface does, so roughness is raised by how much the normals around a
// pixel disagree (Toksvig 2005; Kaplanyan et al. 2016, whose limit this takes).
//
// Needs `oct_decode` from common/math.wgsl.

// The most the squared GGX width is raised by. It keeps relief from turning fully diffuse.
const SPECULAR_AA_MOST: f32 = 0.18;
// Neighbours whose depth differs by more than this share of the pixel's are another surface.
const SPECULAR_AA_DEPTH_SHARE: f32 = 0.03;

// `roughness` at `pixel`, raised where the normals of the pixels beside it differ from its own.
// `depth` is the pixel's, reversed, as the depth buffer holds it.
fn antialiased_roughness(
    normals: texture_2d<f32>,
    depths: texture_2d<f32>,
    pixel: vec2i,
    normal: vec3f,
    depth: f32,
    roughness: f32,
) -> f32 {
    let last = vec2i(textureDimensions(normals)) - 1;
    var sum = normal;
    var count = 1.0;
    for (var i = 0; i < 4; i++) {
        let offset = vec2i((i & 1) * (1 - (i & 2)), (1 - (i & 1)) * (1 - (i & 2)));
        let beside = clamp(pixel + offset, vec2i(0), last);
        let other = textureLoad(depths, beside, 0).r;
        // Depth is the near plane over the distance, so equal shares are equal shares of distance.
        if other > 0.0 && abs(other - depth) < depth * SPECULAR_AA_DEPTH_SHARE {
            sum += oct_decode(textureLoad(normals, beside, 0).xy);
            count += 1.0;
        }
    }
    // The mean of unit vectors is shorter the more they disagree, which gives their variance.
    let mean = min(length(sum / count), 1.0);
    let variance = (1.0 - mean) / max(mean, 1.0e-4);
    let alpha = roughness * roughness;
    return sqrt(sqrt(alpha * alpha + min(2.0 * variance, SPECULAR_AA_MOST)));
}
