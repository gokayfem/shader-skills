// The layout of the light tiles, shared by the pass that builds them and the passes that read
// them. Twins of the constants in `lighting.rs`.

// Pixels along each side of a tile.
const TILE_SIZE: u32 = 16u;
// Lights a tile can list. Any beyond are left out, silently, and which ones depends on the
// order the threads found them in, so a tile that overflows would flicker: lights overlap
// this deeply only where more than a hundred stand within one another's range, which no
// scene has yet. A counter, as the cull lists keep, would be the next step if one did.
const TILE_MAX_LIGHTS: u32 = 127u;
// Words per tile: the count, then the indices.
const TILE_WORDS: u32 = 128u;

// Where tile `tile` of a `size` viewport starts in the tile buffer.
fn light_tile_base(tile: vec2u, size: vec2u) -> u32 {
    let tiles_across = (size.x + TILE_SIZE - 1u) / TILE_SIZE;
    return (tile.y * tiles_across + tile.x) * TILE_WORDS;
}
