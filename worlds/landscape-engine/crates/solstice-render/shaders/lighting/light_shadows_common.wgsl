// Each local light's shadow map: how far its light reaches in every direction, traced through
// the GI volume once a frame by lighting/light_shadows.wgsl. A pass lighting a point then looks
// its shadow up, one read per light, instead of tracing the volume once per pixel per light.
//
// Needs common/math.wgsl, and, declared by whatever includes this:
//   var light_shadows: texture_2d_array<f32>;

#include "common/math.wgsl"

// Texels along each side of a light's octahedral map. Twin of `LIGHT_SHADOW_SIZE` in lighting.rs.
const LIGHT_SHADOW_SIZE: i32 = 64;

// A texel coordinate past the map's edge, folded back the way the octahedron wraps: the far
// side of each edge is the mirror of the near side.
fn light_shadow_texel(texel: vec2i) -> vec2i {
    let last = LIGHT_SHADOW_SIZE - 1;
    var t = texel;
    if t.x < 0 || t.x > last {
        t = vec2i(select(2 * last + 1 - t.x, -1 - t.x, t.x < 0), last - t.y);
    }
    if t.y < 0 || t.y > last {
        t = vec2i(last - t.x, select(2 * last + 1 - t.y, -1 - t.y, t.y < 0));
    }
    return clamp(t, vec2i(0), vec2i(last));
}

// How much of light `index` reaches the point `from_light` away from it, `distance` off, on a
// surface with `normal` that the volume resolves with voxels `voxel` across: 1 in full view, 0
// behind something, in between across a texel of the map. Each of the four texels around the
// point's direction is compared where its own ray meets the point's surface, taken as a
// plane, or at the point's own distance if that is nearer, so that a surface seen at a slant
// does not shadow itself where a neighbouring texel reaches it nearer the light. The point's own surface stands in the volume, and the trace
// stops most of a voxel short of any surface, so the light must fall two voxels short of the
// plane before the point counts as shadowed. Lights beyond the map's layers are never shadowed.
fn light_shadow(index: u32, from_light: vec3f, distance: f32, voxel: f32, normal: vec3f) -> f32 {
    if index >= textureNumLayers(light_shadows) {
        return 1.0;
    }
    let size = f32(LIGHT_SHADOW_SIZE);
    let p = (oct_encode(from_light / distance) * 0.5 + 0.5) * size - 0.5;
    let base = vec2i(floor(p));
    let f = p - vec2f(base);
    // The plane's distance from the light along its normal, which faces the light. It is
    // negative for a lamp nearer its surface than the lookup's lift off it. The volume cannot
    // resolve so close a lamp's rays towards the surface, so they count as lit, even behind a
    // wall. The lift is 1.5 of the receiver's voxels, so more lamps qualify further from the
    // camera.
    let plane = -dot(from_light, normal);
    var lit = 0.0;
    for (var i = 0; i < 4; i++) {
        let corner = vec2i(i % 2, i / 2);
        let weight = select(1.0 - f.x, f.x, corner.x == 1) * select(1.0 - f.y, f.y, corner.y == 1);
        let texel = light_shadow_texel(base + corner);
        let ray = oct_decode((vec2f(texel) + 0.5) / size * 2.0 - 1.0);
        // Where this texel's ray meets the plane, but never past the point itself: at a slant
        // that meeting can lie far beyond it, behind whatever stands there. The point's own
        // distance where the ray runs along the plane or away from it.
        let towards = -dot(ray, normal);
        let along = select(distance, min(distance, plane / towards), towards > 0.001);
        let far = textureLoad(light_shadows, texel, i32(index), 0).r;
        lit += weight * select(0.0, 1.0, far >= along - voxel * 2.0);
    }
    return lit;
}
