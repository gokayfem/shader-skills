//! Reads block-compressed KTX2 textures, and fits them to the scene's square texture layers
//! without decoding them.
//!
//! Scenes that ship BC7 and BC5 textures already hold what the renderer stores, so their blocks
//! can be copied. Copying is exact where a level is square, which is nearly every texture of a
//! large scene. Other shapes take the nearest block, a much cheaper resampling than decoding and
//! encoding again, and good enough for the strips and small maps that need it.

use std::io::Read;

/// `VK_FORMAT_BC4_UNORM_BLOCK`
pub const BC4_UNORM: u32 = 139;
/// `VK_FORMAT_BC5_UNORM_BLOCK`
pub const BC5_UNORM: u32 = 141;
/// `VK_FORMAT_BC7_UNORM_BLOCK`
pub const BC7_UNORM: u32 = 145;
/// `VK_FORMAT_BC7_SRGB_BLOCK`
pub const BC7_SRGB: u32 = 146;

const MAGIC: [u8; 12] = [
    0xab, b'K', b'T', b'X', b' ', b'2', b'0', 0xbb, b'\r', b'\n', 0x1a, b'\n',
];
/// The file's fixed header, up to where its level index begins.
const HEADER_BYTES: usize = 80;
const LEVEL_ENTRY_BYTES: usize = 24;
const SUPERCOMPRESSION_NONE: u32 = 0;
const SUPERCOMPRESSION_ZSTD: u32 = 2;
/// A BC5 or BC7 block's size.
pub const BLOCK_BYTES: usize = 16;
/// Texels along a block's edge.
const BLOCK_EDGE: u32 = 4;

/// Reads only a file's format, to check what a long bake will need before starting it.
pub fn read_format(path: &std::path::Path) -> Result<u32, String> {
    let mut header = [0u8; HEADER_BYTES];
    std::fs::File::open(path)
        .and_then(|mut file| file.read_exact(&mut header))
        .map_err(|e| e.to_string())?;
    if header[..MAGIC.len()] != MAGIC {
        return Err("not a KTX2 file".to_owned());
    }
    Ok(u32::from_le_bytes(
        header[12..16].try_into().expect("four bytes"),
    ))
}

/// An opened KTX2 file.
#[derive(Debug)]
pub struct Ktx2 {
    pub format: u32,
    pub width: u32,
    pub height: u32,
    /// Finest first.
    levels: Vec<Level>,
    supercompression: u32,
    bytes: Vec<u8>,
}

#[derive(Clone, Copy, Debug)]
struct Level {
    offset: usize,
    length: usize,
    uncompressed: usize,
}

impl Ktx2 {
    /// Reads a file's header and level index. The levels are decoded when asked for.
    pub fn parse(bytes: Vec<u8>) -> Result<Self, String> {
        if bytes.len() < HEADER_BYTES || bytes[..MAGIC.len()] != MAGIC {
            return Err("not a KTX2 file".to_owned());
        }
        let word =
            |at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().expect("four bytes"));
        let format = word(12);
        let width = word(20);
        let height = word(24);
        let depth = word(28);
        let layers = word(32);
        let faces = word(36);
        let level_count = word(40) as usize;
        let supercompression = word(44);
        if width == 0 || height == 0 || depth > 1 || layers > 1 || faces != 1 || level_count == 0 {
            return Err(format!(
                "not a plain 2D texture: {width}x{height}x{depth}, {layers} layers, {faces} faces, {level_count} levels"
            ));
        }
        if supercompression != SUPERCOMPRESSION_NONE && supercompression != SUPERCOMPRESSION_ZSTD {
            return Err(format!(
                "supercompression {supercompression} is not supported"
            ));
        }
        let index_end = level_count
            .checked_mul(LEVEL_ENTRY_BYTES)
            .and_then(|size| size.checked_add(HEADER_BYTES))
            .filter(|&end| end <= bytes.len())
            .ok_or("the level index runs off the end of the file")?;
        let mut levels = Vec::with_capacity(level_count);
        for entry in bytes[HEADER_BYTES..index_end].chunks_exact(LEVEL_ENTRY_BYTES) {
            let number = |at: usize| {
                u64::from_le_bytes(entry[at..at + 8].try_into().expect("eight bytes")) as usize
            };
            let level = Level {
                offset: number(0),
                length: number(8),
                uncompressed: number(16),
            };
            if level
                .offset
                .checked_add(level.length)
                .is_none_or(|end| end > bytes.len())
            {
                return Err("a level runs off the end of the file".to_owned());
            }
            levels.push(level);
        }
        Ok(Self {
            format,
            width,
            height,
            levels,
            supercompression,
            bytes,
        })
    }

    pub fn level_count(&self) -> usize {
        self.levels.len()
    }

    /// The size of a level, in blocks.
    pub fn level_blocks(&self, level: usize) -> (u32, u32) {
        let blocks = |size: u32| (size >> level).max(1).div_ceil(BLOCK_EDGE);
        (blocks(self.width), blocks(self.height))
    }

    /// The finest level no longer than `edge` on its longer side, or the coarsest level at
    /// least that long if there is no exact match to be had.
    fn level_for_edge(&self, edge: u32) -> usize {
        let longer = |level: usize| (self.width >> level).max(self.height >> level).max(1);
        let mut level = 0;
        while level + 1 < self.levels.len() && longer(level + 1) >= edge {
            level += 1;
        }
        level
    }

    /// A level's blocks, uncompressed.
    pub fn level(&self, level: usize) -> Result<Vec<u8>, String> {
        let entry = self.levels[level];
        let stored = &self.bytes[entry.offset..entry.offset + entry.length];
        let blocks = if self.supercompression == SUPERCOMPRESSION_ZSTD {
            let mut decoder = ruzstd::decoding::StreamingDecoder::new(stored)
                .map_err(|e| format!("level {level}: {e}"))?;
            let mut out = Vec::with_capacity(entry.uncompressed);
            decoder
                .read_to_end(&mut out)
                .map_err(|e| format!("level {level}: {e}"))?;
            out
        } else {
            stored.to_vec()
        };
        let (wide, high) = self.level_blocks(level);
        if blocks.len() != wide as usize * high as usize * BLOCK_BYTES {
            return Err(format!("level {level} is not the size its header says"));
        }
        Ok(blocks)
    }

    /// Every level of a square layer of edge `size`, finest first, as BC5 or BC7 blocks.
    pub fn fit_square(&self, size: u32) -> Result<Vec<Vec<u8>>, String> {
        let mut levels = Vec::with_capacity(size.ilog2() as usize + 1);
        let mut decoded: Option<(usize, Vec<u8>)> = None;
        for target in 0..=size.ilog2() {
            let edge = (size >> target).max(1);
            let source = self.level_for_edge(edge);
            let blocks = match &decoded {
                Some((level, blocks)) if *level == source => blocks.clone(),
                _ => self.level(source)?,
            };
            let (wide, high) = self.level_blocks(source);
            let fitted = fit_blocks(&blocks, wide, high, edge.div_ceil(BLOCK_EDGE));
            decoded = Some((source, blocks));
            levels.push(fitted);
        }
        Ok(levels)
    }
}

/// Resamples a grid of blocks to `target` by `target`, by nearest block.
fn fit_blocks(blocks: &[u8], wide: u32, high: u32, target: u32) -> Vec<u8> {
    if wide == target && high == target {
        return blocks.to_vec();
    }
    let mut out = Vec::with_capacity(target as usize * target as usize * BLOCK_BYTES);
    for y in 0..target {
        let row = (u64::from(y) * u64::from(high) / u64::from(target)) as usize;
        for x in 0..target {
            let column = (u64::from(x) * u64::from(wide) / u64::from(target)) as usize;
            let at = (row * wide as usize + column) * BLOCK_BYTES;
            out.extend_from_slice(&blocks[at..at + BLOCK_BYTES]);
        }
    }
    out
}

/// Inverts a BC4 block's values (`255 - value`) in place, by swapping and remapping what it holds.
///
/// A block's two endpoints and its 3-bit indices together name eight values. Inverting swaps the
/// endpoints, so each index is sent to the place its value moved to. The inverted endpoints keep
/// their order, so the block stays in the same one of BC4's two modes.
fn invert_bc4(block: &mut [u8]) {
    let (e0, e1) = (block[0], block[1]);
    // The two modes interpolate over different numbers of steps, so the remapping differs.
    let map: [u64; 8] = if e0 > e1 {
        [1, 0, 7, 6, 5, 4, 3, 2]
    } else {
        [1, 0, 5, 4, 3, 2, 7, 6]
    };
    block[0] = 255 - e1;
    block[1] = 255 - e0;
    let mut indices = [0u8; 8];
    indices[..6].copy_from_slice(&block[2..8]);
    let indices = u64::from_le_bytes(indices);
    let mut flipped = 0u64;
    for texel in 0..16 {
        let index = (indices >> (texel * 3)) & 7;
        flipped |= map[index as usize] << (texel * 3);
    }
    block[2..8].copy_from_slice(&flipped.to_le_bytes()[..6]);
}

/// Inverts the green channel of a whole BC5 level.
pub fn invert_bc5_green(level: &mut [u8]) {
    for block in level.chunks_exact_mut(BLOCK_BYTES) {
        invert_bc4(&mut block[8..]);
    }
}

/// A BC5 level of `edge` that has this value everywhere.
pub fn constant_bc5(edge: u32, red: u8, green: u8) -> Vec<u8> {
    let blocks = edge.div_ceil(BLOCK_EDGE) as usize;
    let mut level = vec![0u8; blocks * blocks * BLOCK_BYTES];
    for block in level.chunks_exact_mut(BLOCK_BYTES) {
        block[0] = red;
        block[1] = red;
        block[8] = green;
        block[9] = green;
    }
    level
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The eight values of a BC4 block.
    fn palette(block: &[u8]) -> [i32; 8] {
        let (a, b) = (i32::from(block[0]), i32::from(block[1]));
        if a > b {
            std::array::from_fn(|i| match i {
                0 => a,
                1 => b,
                _ => ((8 - i as i32) * a + (i as i32 - 1) * b + 3) / 7,
            })
        } else {
            std::array::from_fn(|i| match i {
                0 => a,
                1 => b,
                6 => 0,
                7 => 255,
                _ => ((6 - i as i32) * a + (i as i32 - 1) * b + 2) / 5,
            })
        }
    }

    fn texels(block: &[u8]) -> [i32; 16] {
        let table = palette(block);
        let mut bytes = [0u8; 8];
        bytes[..6].copy_from_slice(&block[2..8]);
        let indices = u64::from_le_bytes(bytes);
        std::array::from_fn(|t| table[((indices >> (t * 3)) & 7) as usize])
    }

    /// Writes one texel of each index, then repeats, so that every index is exercised.
    fn block(e0: u8, e1: u8) -> [u8; 8] {
        let mut indices = 0u64;
        for texel in 0..16u64 {
            indices |= (texel % 8) << (texel * 3);
        }
        let mut out = [0u8; 8];
        out[0] = e0;
        out[1] = e1;
        out[2..8].copy_from_slice(&indices.to_le_bytes()[..6]);
        out
    }

    #[test]
    fn inverting_a_block_inverts_every_texel() {
        for (e0, e1) in [
            (200, 40),
            (40, 200),
            (255, 0),
            (0, 255),
            (90, 90),
            (101, 100),
        ] {
            let source = block(e0, e1);
            let mut inverted = source;
            invert_bc4(&mut inverted);
            let (before, after) = (texels(&source), texels(&inverted));
            for t in 0..16 {
                // Rounding differs by at most one step between the two palettes.
                assert!(
                    (before[t] + after[t] - 255).abs() <= 1,
                    "endpoints {e0} {e1}, texel {t}: {} then {}",
                    before[t],
                    after[t]
                );
            }
        }
    }

    #[test]
    fn inverting_green_leaves_red_alone() {
        let mut level = constant_bc5(8, 77, 200);
        let red = level[..8].to_vec();
        invert_bc5_green(&mut level);
        assert_eq!(level[..8], red[..]);
        assert_eq!(level[8], 55);
    }

    #[test]
    fn a_grid_is_fitted_by_nearest_block() {
        let grid = |wide: u32, high: u32| -> Vec<u8> {
            (0..wide * high)
                .flat_map(|i| [i as u8; BLOCK_BYTES])
                .collect()
        };
        // Exact copies stay exact.
        assert_eq!(fit_blocks(&grid(4, 4), 4, 4, 4), grid(4, 4));
        // A strip takes each column's block.
        let fitted = fit_blocks(&grid(4, 1), 4, 1, 2);
        let firsts: Vec<u8> = fitted.chunks_exact(BLOCK_BYTES).map(|b| b[0]).collect();
        assert_eq!(firsts, [0, 2, 0, 2]);
        // A small map is stretched.
        let fitted = fit_blocks(&grid(1, 1), 1, 1, 3);
        assert_eq!(fitted.len(), 9 * BLOCK_BYTES);
    }

    /// Wraps levels in a KTX2 file with no supercompression.
    fn file(width: u32, height: u32, levels: &[Vec<u8>]) -> Vec<u8> {
        let mut bytes = MAGIC.to_vec();
        let mut header = [0u32; 17];
        header[0] = BC5_UNORM;
        header[1] = 1;
        header[2] = width;
        header[3] = height;
        header[6] = 1;
        header[7] = levels.len() as u32;
        for value in header {
            bytes.extend(value.to_le_bytes());
        }
        let mut offset = HEADER_BYTES + levels.len() * LEVEL_ENTRY_BYTES;
        let mut payload = Vec::new();
        for level in levels {
            for value in [offset, level.len(), level.len()] {
                bytes.extend((value as u64).to_le_bytes());
            }
            offset += level.len();
            payload.extend_from_slice(level);
        }
        assert_eq!(bytes.len(), HEADER_BYTES + levels.len() * LEVEL_ENTRY_BYTES);
        bytes.extend(payload);
        bytes
    }

    #[test]
    fn levels_are_chosen_by_edge_and_copied_exactly() {
        let level = |edge: u32, value: u8| constant_bc5(edge, value, value);
        let levels = [
            level(16, 1),
            level(8, 2),
            level(4, 3),
            level(2, 4),
            level(1, 5),
        ];
        let texture = Ktx2::parse(file(16, 16, &levels)).unwrap();
        let fitted = texture.fit_square(8).unwrap();
        assert_eq!(fitted.len(), 4, "edges 8, 4, 2 and 1");
        assert_eq!(fitted[0], levels[1]);
        assert_eq!(fitted[1], levels[2]);
        assert_eq!(fitted[2], levels[3]);
        assert_eq!(fitted[3], levels[4]);
    }

    #[test]
    fn damaged_files_are_refused() {
        assert!(Ktx2::parse(vec![0; 40]).is_err());
        let level = constant_bc5(4, 1, 2);
        let mut good = file(4, 4, &[level]);
        good.truncate(good.len() - 4);
        assert!(Ktx2::parse(good).is_err());
    }
}
