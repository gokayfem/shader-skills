// How software-rasterized depth is kept for a depth map: a drawing's stamp in the top byte of
// each word, over the reversed depth in 24 bits. An atomic max then keeps the nearest sample
// of the latest drawing, whatever earlier drawings left in the word, so the buffer is cleared
// only when the stamps start a new round (shadow.rs); the merge reads only words with its own
// stamp. Twin of the layout in raster.rs.

const DEPTH_STAMP_SHIFT: u32 = 24u;
const DEPTH_STAMP_RANGE: f32 = 16777215.0;

fn stamped(stamp: u32, reversed_depth: f32) -> u32 {
    return (stamp << DEPTH_STAMP_SHIFT) | u32(clamp(reversed_depth, 0.0, 1.0) * DEPTH_STAMP_RANGE);
}

fn stamp_of(word: u32) -> u32 {
    return word >> DEPTH_STAMP_SHIFT;
}

// The depth a word holds, forward again, 0 at the light's side to 1 at the far side.
fn stamped_depth(word: u32) -> f32 {
    return 1.0 - f32(word & 0xffffffu) / DEPTH_STAMP_RANGE;
}
