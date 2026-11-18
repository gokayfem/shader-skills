//! Texture preparation: mip chains and block compression.
//!
//! Colour goes to BC7 through the `block_compression` crate, a pure Rust port of Intel's ISPC
//! Texture Compressor. Its slower settings measured no better on scanned textures, so this
//! uses its "basic" one. Normals and material data go to BC5 through [`encode_bc4`] here, as
//! that crate's BC4 stops at the channel's extremes, which measured 1 dB worse on normals.

use block_compression::{encode::compress_rgba8, BC7Settings, CompressionVariant};
use rayon::prelude::*;
use solstice_asset::{SceneAsset, TextureEncoding};

/// Box-filters a square image down to 1x1. The result includes the input as mip 0.
pub fn mip_chain(base: Vec<u8>, size: u32, channels: usize) -> Vec<Vec<u8>> {
    let mut mips = vec![base];
    let mut edge = size as usize;
    while edge > 1 {
        let previous = mips.last().expect("chain starts non-empty");
        let half = edge / 2;
        let mut next = Vec::with_capacity(half * half * channels);
        for y in 0..half {
            for x in 0..half {
                for channel in 0..channels {
                    let at = |px: usize, py: usize| {
                        u32::from(previous[(py * edge + px) * channels + channel])
                    };
                    let sum = at(x * 2, y * 2)
                        + at(x * 2 + 1, y * 2)
                        + at(x * 2, y * 2 + 1)
                        + at(x * 2 + 1, y * 2 + 1);
                    next.push((sum / 4) as u8);
                }
            }
        }
        mips.push(next);
        edge = half;
    }
    mips
}

/// The largest grid of tiles [`masked_mip_chain`] keeps coverage over, per side.
const COVERAGE_TILES: usize = 4;
/// The least edge of such a tile, in texels: with fewer, a tile's texels are too few to hold a
/// fraction of coverage, and thin detail would round away.
const MIN_TILE_EDGE: usize = 8;

/// How many tiles per side a level of this edge is split into.
fn tiles_for(edge: usize) -> usize {
    (edge / MIN_TILE_EDGE).clamp(1, COVERAGE_TILES)
}

/// Moves the point where `alpha` crosses 0.5 from `threshold` to 128 (of 255), so that what passes
/// a test at `threshold` is exactly what passes a test at 0.5 afterwards. 255 stays where it is,
/// and so does 0, unless a `threshold` of 0 means everything passes. A `threshold` of 256 passes
/// nothing.
pub fn move_alpha_threshold(alpha: u8, threshold: u16) -> u8 {
    let (alpha, threshold) = (u32::from(alpha), u32::from(threshold));
    if alpha < threshold {
        (alpha * 128 / threshold) as u8
    } else if threshold >= 255 {
        255
    } else {
        (128 + (alpha - threshold) * 127 / (255 - threshold)) as u8
    }
}

/// Mip chain of an RGBA8 image whose alpha is a cut-out mask at 0.5.
///
/// * Alpha keeps its coverage (Castaño 2010): a plain box filter pulls thin strands below 0.5 and
///   they vanish with distance, so each level moves its own threshold until as much of it passes
///   as passes in the largest level. This is done per tile, as an atlas's cells differ, with ties
///   parted by a fixed pattern in the texture, so it holds still under the camera.
/// * Colour is averaged by alpha, and texels with no alpha at all take the colour of the level
///   above, so bilinear filtering at an edge never reaches a texel's unrelated colour.
pub fn masked_mip_chain(base: Vec<u8>, size: u32) -> Vec<Vec<u8>> {
    let mut levels = alpha_weighted_pyramid(base, size);
    fill_transparent_colour(&mut levels);

    let targets = passing_per_tile(&levels[0], size as usize);
    let mut edge = size as usize;
    for level in &mut levels[1..] {
        edge /= 2;
        keep_coverage(level, edge, size as usize, &targets);
    }
    levels
}

/// Halves an RGBA8 image down to 1x1, with alpha box-filtered and colour weighted by alpha.
fn alpha_weighted_pyramid(base: Vec<u8>, size: u32) -> Vec<Vec<u8>> {
    let mut levels = vec![base];
    let mut edge = size as usize;
    while edge > 1 {
        let previous = levels.last().expect("chain starts non-empty");
        let half = edge / 2;
        let mut next = Vec::with_capacity(half * half * 4);
        for y in 0..half {
            for x in 0..half {
                let mut colour = [0u32; 3];
                let mut alpha = 0u32;
                for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                    let texel = &previous[((y * 2 + dy) * edge + x * 2 + dx) * 4..][..4];
                    let weight = u32::from(texel[3]);
                    alpha += weight;
                    for channel in 0..3 {
                        colour[channel] += u32::from(texel[channel]) * weight;
                    }
                }
                for value in colour {
                    next.push((value + alpha / 2).checked_div(alpha).unwrap_or(0) as u8);
                }
                next.push((alpha / 4) as u8);
            }
        }
        levels.push(next);
        edge = half;
    }
    levels
}

/// Gives every texel with no alpha the colour of the texel above it, from the top level down.
fn fill_transparent_colour(levels: &mut [Vec<u8>]) {
    for index in (0..levels.len() - 1).rev() {
        let (lower, upper) = levels.split_at_mut(index + 1);
        let (finer, coarser) = (&mut lower[index], &upper[0]);
        let edge = (finer.len() / 4).isqrt();
        for (i, texel) in finer.chunks_exact_mut(4).enumerate() {
            if texel[3] == 0 {
                let (x, y) = (i % edge / 2, i / edge / 2);
                texel[..3].copy_from_slice(&coarser[(y * (edge / 2) + x) * 4..][..3]);
            }
        }
    }
}

/// The tile of a `tiles`-per-side grid that a texel lies in.
fn tile_of(x: usize, y: usize, edge: usize, tiles: usize) -> usize {
    y * tiles / edge * tiles + x * tiles / edge
}

/// How many texels of each tile pass the test, and how many there are.
fn passing_per_tile(level: &[u8], edge: usize) -> Vec<(u64, u64)> {
    let tiles = tiles_for(edge);
    let mut counts = vec![(0, 0); tiles * tiles];
    for (i, texel) in level.chunks_exact(4).enumerate() {
        let count: &mut (u64, u64) = &mut counts[tile_of(i % edge, i / edge, edge, tiles)];
        count.0 += u64::from(texel[3] >= 128);
        count.1 += 1;
    }
    counts
}

/// Moves each tile's threshold so as much of `level` passes as `targets` say does in the largest
/// level, of edge `base_edge`.
fn keep_coverage(level: &mut [u8], edge: usize, base_edge: usize, targets: &[(u64, u64)]) {
    let base_tiles = tiles_for(base_edge);
    let tiles = tiles_for(edge);
    // A level too small for the whole grid merges its tiles.
    let mut wanted = vec![(0, 0); tiles * tiles];
    for (tile, count) in targets.iter().enumerate() {
        let (x, y) = (tile % base_tiles, tile / base_tiles);
        let merged: &mut (u64, u64) = &mut wanted[tile_of(x, y, base_tiles, tiles)];
        merged.0 += count.0;
        merged.1 += count.1;
    }

    let mut histograms = vec![vec![0u64; RANKS as usize]; tiles * tiles];
    for (i, texel) in level.chunks_exact(4).enumerate() {
        let (x, y) = (i % edge, i / edge);
        histograms[tile_of(x, y, edge, tiles)][rank(texel[3], x, y) as usize] += 1;
    }
    let thresholds: Vec<u32> = histograms
        .iter()
        .zip(&wanted)
        .map(|(histogram, &(passing, total))| {
            let texels: u64 = histogram.iter().sum();
            let wanted = (passing as f64 / total as f64 * texels as f64).round() as u64;
            rank_passing(histogram, wanted)
        })
        .collect();
    for (i, texel) in level.chunks_exact_mut(4).enumerate() {
        let (x, y) = (i % edge, i / edge);
        let threshold = thresholds[tile_of(x, y, edge, tiles)];
        let alpha = texel[3];
        let passes = rank(alpha, x, y) >= threshold;
        texel[3] = if u32::from(alpha) == threshold / RANKS_PER_ALPHA {
            // Texels parted only by the pattern would all filter to about 0.5, and so to nearly
            // nothing passing between them.
            if passes {
                255
            } else {
                0
            }
        } else {
            let moved = move_alpha_threshold(alpha, threshold.div_ceil(RANKS_PER_ALPHA) as u16);
            if passes {
                moved.max(128)
            } else {
                moved.min(127)
            }
        };
    }
}

/// Ranks each alpha value is split into, by the pattern in [`rank`].
const RANKS_PER_ALPHA: u32 = 16;
/// Ranks in all.
const RANKS: u32 = 256 * RANKS_PER_ALPHA;

/// Orders texels by alpha, and those of equal alpha by a fixed 4x4 pattern. A level of a fine
/// regular pattern, such as a net, can be one alpha throughout, where no threshold alone could
/// let some of it pass and not the rest: it would turn solid or vanish.
fn rank(alpha: u8, x: usize, y: usize) -> u32 {
    const BAYER: [[u32; 4]; 4] = [[0, 8, 2, 10], [12, 4, 14, 6], [3, 11, 1, 9], [15, 7, 13, 5]];
    u32::from(alpha) * RANKS_PER_ALPHA + BAYER[y % 4][x % 4]
}

/// The rank that lets `wanted` texels of a histogram pass, as near as its ranks allow. Never one
/// of alpha 0: a texel with no alpha at all must stay a hole.
fn rank_passing(histogram: &[u64], wanted: u64) -> u32 {
    if wanted == 0 {
        return RANKS;
    }
    let mut passing = 0;
    for value in (RANKS_PER_ALPHA..RANKS).rev() {
        let with_this = passing + histogram[value as usize];
        if with_this >= wanted {
            // The nearer of stopping above this rank or below it.
            return if passing > 0 && wanted - passing <= with_this - wanted {
                value + 1
            } else {
                value
            };
        }
        passing = with_this;
    }
    RANKS_PER_ALPHA
}

/// Converts a scene's textures to BC7 and BC5, unless they are already compressed.
/// Textures below 4x4 cannot be, since blocks are 4x4, and those stay as they are.
pub fn compress_textures(scene: &mut SceneAsset) {
    if scene.texture_encoding != TextureEncoding::Raw || scene.texture_size < 4 {
        return;
    }
    let size = scene.texture_size;
    for layer in &mut scene.textures {
        // A layer made ready and compressed already, as the editor's bake is given
        // (`solstice-bake`'s textures), is a quarter the size of a raw one and stays as it is.
        let raw_bytes = size as usize * size as usize * 4;
        if layer
            .color_mips
            .first()
            .is_some_and(|mip| mip.len() != raw_bytes)
        {
            continue;
        }
        for (level, mip) in layer.color_mips.iter_mut().enumerate() {
            *mip = encode_bc7(mip, size >> level);
        }
        for (level, mip) in layer.normal_mips.iter_mut().enumerate() {
            *mip = encode_bc5(mip, size >> level);
        }
        for (level, mip) in layer.material_mips.iter_mut().enumerate() {
            *mip = encode_bc5(mip, size >> level);
        }
    }
    scene.texture_encoding = TextureEncoding::Block;
}

/// Encodes a square RGBA8 mip level, of any edge, to BC7.
fn encode_bc7(texels: &[u8], edge: u32) -> Vec<u8> {
    let (texels, edge) = fill_blocks(texels, edge, 4);
    let edge = edge as usize;
    let blocks_wide = edge / 4;
    let mut out = vec![0; blocks_wide * blocks_wide * 16];
    let variant = CompressionVariant::BC7(BC7Settings::alpha_basic());
    out.par_chunks_mut(blocks_wide * 16)
        .zip(texels.par_chunks(edge * 4 * 4))
        .for_each(|(row_out, row_in)| {
            let (width, stride) = (edge as u32, (edge * 4) as u32);
            compress_rgba8(variant, row_in, row_out, width, 4, stride);
        });
    out
}

/// Encodes a square RG8 mip level, of any edge, to BC5.
fn encode_bc5(texels: &[u8], edge: u32) -> Vec<u8> {
    let (texels, edge) = fill_blocks(texels, edge, 2);
    let edge = edge as usize;
    let blocks_wide = edge / 4;
    let mut out = vec![0; blocks_wide * blocks_wide * 16];
    out.par_chunks_mut(blocks_wide * 16)
        .zip(texels.par_chunks(edge * 2 * 4))
        .for_each(|(row_out, row_in)| {
            for (x, block_out) in row_out.chunks_exact_mut(16).enumerate() {
                for (channel, half) in block_out.chunks_exact_mut(8).enumerate() {
                    let values = std::array::from_fn(|i| {
                        row_in[((i / 4) * edge + x * 4 + i % 4) * 2 + channel]
                    });
                    half.copy_from_slice(&encode_bc4(&values));
                }
            }
        });
    out
}

/// A mip smaller than one block repeats to fill it, as sampling only reads the part in use.
fn fill_blocks(texels: &[u8], edge: u32, channels: usize) -> (std::borrow::Cow<'_, [u8]>, u32) {
    if edge >= 4 {
        return (texels.into(), edge);
    }
    let n = edge as usize;
    let mut filled = Vec::with_capacity(16 * channels);
    for i in 0..16 {
        let at = ((i / 4) % n * n + (i % 4) % n) * channels;
        filled.extend_from_slice(&texels[at..at + channels]);
    }
    (filled.into(), 4)
}

/// How far inside a block's range to look for better endpoints. Noise-like blocks do best
/// with endpoints inside the range, and searching further than this measured no better.
const BC4_SEARCH: i32 = 8;

/// Encodes 16 values to a BC4 block by testing endpoint pairs near the block's extremes.
fn encode_bc4(values: &[u8; 16]) -> [u8; 8] {
    let min = i32::from(*values.iter().min().expect("16 values"));
    let max = i32::from(*values.iter().max().expect("16 values"));
    if min == max {
        // Equal endpoints select the six-value mode, whose index 0 is the first endpoint.
        return [min as u8, min as u8, 0, 0, 0, 0, 0, 0];
    }

    let mut best = (i64::MAX, min, max);
    for low in min..=(min + BC4_SEARCH).min(max - 1) {
        for high in (max - BC4_SEARCH).max(low + 1)..=max {
            let palette = bc4_palette(low, high);
            let mut error = 0;
            for &value in values {
                let difference = palette
                    .iter()
                    .map(|entry| (entry - i32::from(value)).abs())
                    .min()
                    .expect("8 entries");
                error += i64::from(difference * difference);
                if error >= best.0 {
                    break;
                }
            }
            if error < best.0 {
                best = (error, low, high);
            }
        }
    }

    let (_, low, high) = best;
    let palette = bc4_palette(low, high);
    let mut indices = 0u64;
    for (i, &value) in values.iter().enumerate() {
        let (nearest, _) = palette
            .iter()
            .enumerate()
            .min_by_key(|(_, entry)| (*entry - i32::from(value)).abs())
            .expect("8 entries");
        // Position 0 of the palette is the first endpoint, 7 the second, and the rest run
        // between them, which is the order of index values 0, 2, 3, 4, 5, 6, 7, 1.
        let code = match nearest {
            0 => 0,
            7 => 1,
            between => between as u64 + 1,
        };
        indices |= code << (3 * i);
    }
    let mut block = [0; 8];
    block[0] = high as u8;
    block[1] = low as u8;
    block[2..].copy_from_slice(&indices.to_le_bytes()[..6]);
    block
}

/// The eight values a block can hold when its first endpoint is the larger, from `high` to `low`.
fn bc4_palette(low: i32, high: i32) -> [i32; 8] {
    std::array::from_fn(|i| {
        let i = i as i32;
        ((7 - i) * high + i * low + 3) / 7
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use block_compression::decode::{decode_block_bc5, decode_block_bc7};

    #[test]
    fn mip_chain_reaches_one_texel_with_correct_sizes() {
        for channels in [2, 4] {
            let mips = mip_chain(vec![200; 16 * 16 * channels], 16, channels);
            assert_eq!(mips.len(), 5);
            for (level, mip) in mips.iter().enumerate() {
                let edge = 16usize >> level;
                assert_eq!(mip.len(), edge * edge * channels);
            }
            assert_eq!(mips[4], vec![200; channels]);
        }
    }

    /// Texture-like data: a slope with repeating noise on top.
    fn test_value(x: usize, y: usize, seed: usize) -> u8 {
        let noise = ((x * 7 + y * 13 + seed * 5).wrapping_mul(2654435761) >> 7) & 15;
        (60 + (x + y) * 3 + noise + seed * 20).min(255) as u8
    }

    #[test]
    fn bc5_stays_close_to_its_source() {
        let texels: Vec<u8> = (0..16 * 16)
            .flat_map(|i| [test_value(i % 16, i / 16, 0), test_value(i % 16, i / 16, 1)])
            .collect();
        let encoded = encode_bc5(&texels, 16);
        assert_eq!(encoded.len(), 4 * 4 * 16);
        let mut out = [0u8; 32];
        for (index, block) in encoded.chunks_exact(16).enumerate() {
            decode_block_bc5(block, &mut out, 8);
            let (bx, by) = (index % 4 * 4, index / 4 * 4);
            for y in 0..4 {
                for x in 0..8 {
                    let source = texels[((by + y) * 16 + bx) * 2 + x];
                    let error = (i32::from(out[y * 8 + x]) - i32::from(source)).abs();
                    assert!(error <= 4, "block {index}: {error} at {x},{y}");
                }
            }
        }
    }

    #[test]
    fn bc5_represents_flat_blocks_exactly() {
        let encoded = encode_bc5(&[77, 200].repeat(16), 4);
        let mut out = [0u8; 32];
        decode_block_bc5(&encoded, &mut out, 8);
        assert_eq!(out[..], [[77, 200]; 16].concat()[..]);
    }

    #[test]
    fn bc7_stays_close_to_its_source_and_keeps_alpha_separate() {
        let texels: Vec<u8> = (0..16 * 16)
            .flat_map(|i| {
                let (x, y) = (i % 16, i / 16);
                [
                    test_value(x, y, 0),
                    test_value(x, y, 0) / 2,
                    test_value(x, y, 0) / 3,
                    test_value(y, x, 4),
                ]
            })
            .collect();
        let encoded = encode_bc7(&texels, 16);
        let mut out = [0u8; 64];
        for (index, block) in encoded.chunks_exact(16).enumerate() {
            decode_block_bc7(block, &mut out, 16);
            let (bx, by) = (index % 4 * 4, index / 4 * 4);
            for y in 0..4 {
                for x in 0..16 {
                    let source = texels[((by + y) * 16 + bx) * 4 + x];
                    let error = (i32::from(out[y * 16 + x]) - i32::from(source)).abs();
                    assert!(error <= 12, "block {index}: {error} at {x},{y}");
                }
            }
        }
    }

    /// A fraction of a level's texels at or above 0.5, for a tile of it.
    fn coverage(level: &[u8]) -> f64 {
        let passing = level.chunks_exact(4).filter(|t| t[3] >= 128).count();
        passing as f64 / (level.len() / 4) as f64
    }

    /// One-texel strands every fourth column, a quarter of the image.
    fn strands(edge: usize) -> Vec<u8> {
        (0..edge * edge)
            .flat_map(|i| {
                let on = (i % edge).is_multiple_of(4);
                [if on { 200 } else { 0 }, 50, 20, if on { 255 } else { 0 }]
            })
            .collect()
    }

    /// One solid texel in each 4x4 block, a sixteenth of the image.
    fn dots(edge: usize) -> Vec<u8> {
        (0..edge * edge)
            .flat_map(|i| {
                let on = (i % edge).is_multiple_of(4) && (i / edge).is_multiple_of(4);
                [200, 50, 20, if on { 255 } else { 0 }]
            })
            .collect()
    }

    #[test]
    fn a_threshold_of_zero_lets_everything_pass() {
        assert!((0..=255).all(|alpha| move_alpha_threshold(alpha, 0) >= 128));
    }

    #[test]
    fn moving_the_threshold_keeps_what_passes_and_the_ends() {
        for threshold in [1u16, 40, 128, 200, 254, 255] {
            for alpha in 0..=255u8 {
                let moved = move_alpha_threshold(alpha, threshold);
                assert_eq!(
                    moved >= 128,
                    u16::from(alpha) >= threshold,
                    "{alpha} at {threshold} gave {moved}"
                );
            }
            assert_eq!(move_alpha_threshold(0, threshold), 0);
            assert_eq!(move_alpha_threshold(255, threshold), 255);
        }
        assert!(move_alpha_threshold(255, 256) < 128, "nothing passes");
    }

    /// Scattered solid texels, on about a quarter of the image: fine detail with no pattern, so
    /// each level holds a spread of alphas for its threshold to fall between.
    fn speckle(edge: usize) -> Vec<u8> {
        (0..edge * edge)
            .flat_map(|i| {
                let h = (i as u32 ^ 0x9e37_79b9).wrapping_mul(0x85eb_ca6b);
                let on = (h ^ (h >> 13)).wrapping_mul(0xc2b2_ae35) >> 30 == 0;
                [120, 80, 40, if on { 255 } else { 0 }]
            })
            .collect()
    }

    #[test]
    fn fine_detail_keeps_its_coverage_down_the_chain() {
        let source = coverage(&speckle(64));
        let plain = mip_chain(speckle(64), 64, 4);
        let masked = masked_mip_chain(speckle(64), 64);
        assert!(coverage(&plain[2]) < 0.05, "box filtering loses the detail");
        for (level, mip) in masked.iter().enumerate().take(5) {
            let found = coverage(mip);
            assert!(
                (found - source).abs() < 0.1,
                "level {level} covers {found}, the source {source}"
            );
        }
    }

    #[test]
    fn a_regular_pattern_neither_turns_solid_nor_vanishes() {
        // Every level below the first few holds one alpha throughout.
        for (source, name) in [(strands(64), "strands"), (dots(64), "dots")] {
            let wanted = coverage(&source);
            let chain = masked_mip_chain(source, 64);
            for (level, mip) in chain.iter().enumerate().take(5) {
                let found = coverage(mip);
                assert!(
                    (found - wanted).abs() < 0.1,
                    "{name}: level {level} covers {found}, the source {wanted}"
                );
            }
        }
    }

    #[test]
    fn each_cell_of_an_atlas_keeps_its_own_coverage() {
        // Fine detail on the left half, solid on the right.
        let edge = 64;
        let base: Vec<u8> = speckle(edge)
            .chunks_exact(4)
            .enumerate()
            .flat_map(|(i, texel)| {
                if i % edge >= edge / 2 {
                    [90, 90, 90, 255]
                } else {
                    [texel[0], texel[1], texel[2], texel[3]]
                }
            })
            .collect();
        let chain = masked_mip_chain(base, edge as u32);
        let level = &chain[2];
        let side = edge / 4;
        let half = |right: bool| -> Vec<u8> {
            level
                .chunks_exact(4)
                .enumerate()
                .filter(|(i, _)| (i % side >= side / 2) == right)
                .flat_map(|(_, texel)| texel.to_vec())
                .collect()
        };
        assert!((coverage(&half(false)) - 0.25).abs() < 0.1);
        assert_eq!(coverage(&half(true)), 1.0);
        let solid = half(true);
        assert!(
            solid.chunks_exact(4).all(|t| t[3] == 255),
            "an opaque cell stays fully opaque, as materials read it as cavity"
        );
    }

    #[test]
    fn holes_take_the_colour_of_what_surrounds_them() {
        let chain = masked_mip_chain(strands(16), 16);
        // A transparent texel's colour was black in red; the strands are 200.
        let hole = &chain[0][4..8];
        assert_eq!(hole[3], 0);
        assert!(hole[0] > 150, "got {hole:?}");
        // Averaged by alpha, a level's strand colour is not darkened by the black around it.
        assert!(chain[1][0] > 150, "got {:?}", &chain[1][..4]);
        // Nothing but zero-alpha texels moved.
        assert_eq!(chain[0][..4], [200, 50, 20, 255]);
    }

    #[test]
    fn nothing_covered_stays_uncovered() {
        let chain = masked_mip_chain(vec![10; 16 * 16 * 4], 16);
        // Alpha 10 of 255 is below the test everywhere.
        assert!(chain.iter().all(|mip| coverage(mip) == 0.0));
        let empty = masked_mip_chain(vec![0; 16 * 16 * 4], 16);
        assert!(empty.iter().all(|mip| coverage(mip) == 0.0));
    }

    #[test]
    fn bc7_keeps_a_mask_close_to_its_coverage() {
        let texels = speckle(64);
        let encoded = encode_bc7(&texels, 64);
        let (mut out, mut decoded) = ([0u8; 64], vec![0u8; texels.len()]);
        for (index, block) in encoded.chunks_exact(16).enumerate() {
            decode_block_bc7(block, &mut out, 16);
            let (bx, by) = (index % 16 * 4, index / 16 * 4);
            for y in 0..4 {
                let at = ((by + y) * 64 + bx) * 4;
                decoded[at..at + 16].copy_from_slice(&out[y * 16..y * 16 + 16]);
            }
        }
        assert!((coverage(&decoded) - coverage(&texels)).abs() < 0.03);
    }

    #[test]
    fn mips_below_a_block_fill_one_whole_block() {
        assert_eq!(encode_bc7(&[9; 4], 1).len(), 16);
        assert_eq!(encode_bc7(&[9; 16], 2).len(), 16);
        assert_eq!(encode_bc5(&[9; 2], 1).len(), 16);
        let two_by_two = [10, 11, 40, 41, 40, 41, 10, 11];
        let mut out = [0u8; 32];
        decode_block_bc5(&encode_bc5(&two_by_two, 2), &mut out, 8);
        assert_eq!(&out[..4], &[10, 11, 40, 41]);
        assert_eq!(&out[8..12], &[40, 41, 10, 11]);
    }

    #[test]
    fn compressing_a_scene_fills_every_level_and_is_idempotent() {
        let chain = |channels| mip_chain(vec![90; 16 * 16 * channels], 16, channels);
        let mut scene = SceneAsset {
            texture_size: 16,
            textures: vec![solstice_asset::TextureLayer {
                color_mips: chain(4),
                normal_mips: chain(2),
                material_mips: chain(2),
            }],
            ..Default::default()
        };
        compress_textures(&mut scene);
        assert_eq!(scene.texture_encoding, TextureEncoding::Block);
        let layer = &scene.textures[0];
        let sizes: Vec<usize> = layer.normal_mips.iter().map(Vec::len).collect();
        assert_eq!(sizes, [256, 64, 16, 16, 16]);
        assert_eq!(layer.color_mips[4].len(), 16);

        let before = layer.color_mips[0].clone();
        compress_textures(&mut scene);
        assert_eq!(scene.textures[0].color_mips[0], before);
    }
}
