// Skinned positions and normals. A shader that includes this after common/scene.wgsl declares
// `bone_texture`, a texture_2d<f32> of this frame's skinning matrices in its top half and last
// frame's in its bottom half.
//
// Every pass that places a vertex goes through skin_rows, so a shared vertex comes out the same
// to the last bit wherever it is skinned: both rasterizers, resolve and the shadow passes alike.

// Twins of NO_BONES and BONE_TEXTURE_WIDTH in skin.rs.
const NO_BONES: u32 = 0xffffffffu;
// False in the shaders of a scene with nothing skinned, so that it pays nothing for skinning: see
// `shaders::module_for`.
const SKINNING: bool = true;
const BONE_TEXTURE_WIDTH: u32 = 1024u;

// A vertex's blended skinning matrix, its top three rows.
struct Skinning {
    row0: vec4f,
    row1: vec4f,
    row2: vec4f,
}

fn instance_skinned(i: Instance) -> bool {
    return SKINNING && i.bones != NO_BONES;
}

fn bone_texel(address: u32, rows_down: u32) -> vec4f {
    return textureLoad(
        bone_texture,
        vec2u(address % BONE_TEXTURE_WIDTH, address / BONE_TEXTURE_WIDTH + rows_down),
        0,
    );
}

// The matrix of one bone of the skeleton whose matrices start at `base`, this frame's or last.
fn bone_rows(base: u32, bone: u32, previous: bool) -> Skinning {
    let rows_down = select(0u, textureDimensions(bone_texture).y / 2u, previous);
    let address = base + bone * 3u;
    return Skinning(
        bone_texel(address, rows_down),
        bone_texel(address + 1u, rows_down),
        bone_texel(address + 2u, rows_down),
    );
}

// Blends up to four bones' matrices by a vertex's weights: `skin` holds its bones and its weights,
// four bytes each. Used bones come first and sum to 255, so unused slots end the loop.
fn skin_rows(base: u32, skin: vec2u, previous: bool) -> Skinning {
    var out = Skinning(vec4f(0.0), vec4f(0.0), vec4f(0.0));
    for (var slot = 0u; slot < 4u; slot++) {
        let weight = (skin.y >> (slot * 8u)) & 0xffu;
        if weight == 0u {
            break;
        }
        let share = f32(weight) / 255.0;
        let bone = bone_rows(base, (skin.x >> (slot * 8u)) & 0xffu, previous);
        out.row0 += bone.row0 * share;
        out.row1 += bone.row1 * share;
        out.row2 += bone.row2 * share;
    }
    return out;
}

fn skin_point(s: Skinning, p: vec3f) -> vec3f {
    let h = vec4f(p, 1.0);
    return vec3f(dot(s.row0, h), dot(s.row1, h), dot(s.row2, h));
}

fn skin_direction(s: Skinning, d: vec3f) -> vec3f {
    return vec3f(dot(s.row0.xyz, d), dot(s.row1.xyz, d), dot(s.row2.xyz, d));
}
