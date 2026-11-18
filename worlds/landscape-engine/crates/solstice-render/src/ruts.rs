//! The ruts tyres have pressed into the ground, as the resolve pass draws them
//! (`ground/ruts.wgsl`, `docs/ruts.md`).
//!
//! Whoever keeps the map of them, a game's physics, keeps it as squares of cells, made only
//! where something has been driven, and hands them over through [`RutSource`]. This holds
//! the squares round the camera in one texture, which the resolve pass samples for every
//! pixel of ground: the height the ground has been pressed to, the tread's print, and how
//! wet and how churned it is.
//!
//! * **The texture wraps.** A square of ground always goes to the same place in it, its
//!   own numbers taken round the texture's width, so that nothing moves when the camera
//!   does: squares coming into reach are copied in over the ones that went out of it on the
//!   far side. The shader reads it with a repeating sampler, straight from the world's x
//!   and z.
//! * **Only what changed is copied.** Each place in the texture remembers which square it
//!   holds and how many times that square had changed when it was copied; a frame copies
//!   the few that differ, and none on a map nobody has driven on.
//! * **Two layers of four bytes a cell,** as the map keeps them, and each square at six
//!   sizes, so that ruts far off thin out and do not sparkle.

use crate::gpu::Gpu;
use glam::{Vec2, Vec4};

/// Cells a side in each square of the map, and how many sizes each square is kept at.
pub const TILE: u32 = 64;
pub const LEVELS: u32 = 6;
/// Squares a side that are held round the camera: with cells of 4 cm, 61 m of ground on a
/// desktop and 31 m in a browser or on a phone.
#[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
pub const TILES: u32 = 24;
#[cfg(any(target_arch = "wasm32", target_os = "android"))]
pub const TILES: u32 = 12;
/// The most squares copied to the GPU in one frame: the rest wait for the next.
const COPIES_A_FRAME: usize = 24;
/// A cell nothing has touched, in each layer: twins of `Tile::new` in solstice-vehicle's
/// `rut.rs`.
const UNTOUCHED: [[u8; 4]; 2] = [[128, 0, 128, 0], [128, 128, 0, 0]];

/// Who keeps the map of ruts: `solstice_vehicle::Ruts`, through whoever joins the two.
pub trait RutSource {
    /// The width of a cell, in metres.
    fn cell(&self) -> f32;
    /// Whether nothing has been pressed anywhere.
    fn is_empty(&self) -> bool;
    /// A count that changes whenever a square does: 0 for one nothing has pressed.
    fn version(&self, tile: (i32, i32)) -> u32;
    /// One layer of a square at every size, the largest first: [`TILE`] rows of [`TILE`]
    /// cells of four bytes, then half that a side, [`LEVELS`] in all.
    fn levels(&self, tile: (i32, i32), layer: usize) -> Option<Vec<Vec<u8>>>;
}

/// The squares of ruts round the camera, on the GPU.
///
/// Until something is first pressed it holds a stand-in of one square, which the shader never
/// reads (the window is empty), so a scene nobody drives on does not pay for the whole of it:
/// tens of megabytes on a desktop. The first press makes the whole texture; whoever bound the
/// stand-in binds [`RutMap::view`] again when [`RutMap::generation`] changes.
pub struct RutMap {
    device: wgpu::Device,
    queue: wgpu::Queue,
    texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    /// Whether `texture` is the whole window, not the stand-in.
    whole: bool,
    /// Which square of ground each place in the texture holds, and that square's count of
    /// changes when it was copied. `None` is untouched ground.
    held: Vec<Option<((i32, i32), u32)>>,
    /// The width of a cell, in metres, as the source last said.
    cell: f32,
    /// The square the window is centred on.
    middle: (i32, i32),
    /// How many places hold a square.
    filled: usize,
}

impl RutMap {
    pub fn new(gpu: &Gpu) -> Self {
        let (texture, view) = make(&gpu.device, &gpu.queue, TILE);
        Self {
            device: gpu.device.clone(),
            queue: gpu.queue.clone(),
            texture,
            view,
            whole: false,
            held: vec![None; (TILES * TILES) as usize],
            cell: 0.04,
            middle: (0, 0),
            filled: 0,
        }
    }

    /// Counts the textures made: 0 for the stand-in, 1 once the whole window is.
    pub fn generation(&self) -> u32 {
        u32::from(self.whole)
    }

    /// Brings the texture up to date with the map, round `centre`: the world's x and z of
    /// what is looked at. Returns how many squares were copied.
    pub fn sync(&mut self, centre: Vec2, source: &dyn RutSource) -> usize {
        // Nothing driven on, and nothing held: nothing to do, which is most frames of most
        // maps.
        if source.is_empty() && self.filled == 0 {
            return 0;
        }
        if !self.whole {
            (self.texture, self.view) = make(&self.device, &self.queue, TILES * TILE);
            self.whole = true;
        }
        self.cell = source.cell();
        let metres = TILE as f32 * self.cell;
        self.middle = (
            (centre.x / metres).round() as i32,
            (centre.y / metres).round() as i32,
        );
        let half = (TILES / 2) as i32;
        let mut copied = 0;
        // The squares nearest the middle first, so that what is underfoot never waits.
        let mut order: Vec<(i32, i32)> = (-half..half)
            .flat_map(|dj| (-half..half).map(move |di| (di, dj)))
            .collect();
        order.sort_by_key(|(di, dj)| (2 * di + 1).pow(2) + (2 * dj + 1).pow(2));
        for (di, dj) in order {
            let tile = (self.middle.0 + di, self.middle.1 + dj);
            let slot = (
                tile.0.rem_euclid(TILES as i32) as u32,
                tile.1.rem_euclid(TILES as i32) as u32,
            );
            let held = &mut self.held[(slot.1 * TILES + slot.0) as usize];
            let version = source.version(tile);
            let wanted = (version != 0).then_some((tile, version));
            if *held == wanted {
                continue;
            }
            if copied >= COPIES_A_FRAME {
                continue;
            }
            let levels = [0, 1].map(|layer| wanted.and_then(|_| source.levels(tile, layer)));
            for (layer, levels) in levels.iter().enumerate() {
                for level in 0..LEVELS {
                    let side = TILE >> level;
                    let at = (slot.0 * side, slot.1 * side);
                    match levels
                        .as_ref()
                        .and_then(|levels| levels.get(level as usize))
                    {
                        Some(texels) if texels.len() == (side * side * 4) as usize => {
                            write(
                                &self.queue,
                                &self.texture,
                                level,
                                layer as u32,
                                at,
                                side,
                                texels,
                            );
                        }
                        _ => {
                            let texels = UNTOUCHED[layer].repeat((side * side) as usize);
                            write(
                                &self.queue,
                                &self.texture,
                                level,
                                layer as u32,
                                at,
                                side,
                                &texels,
                            );
                        }
                    }
                }
            }
            let now = wanted.filter(|_| levels.iter().all(Option::is_some));
            self.filled = self.filled + usize::from(now.is_some()) - usize::from(held.is_some());
            *held = now;
            copied += 1;
        }
        copied
    }

    /// What the shader is told of it (`rut_window` in `common/frame.wgsl`): the middle of
    /// the square of ground that is held, in the world's x and z; half its width, in
    /// metres, or 0 when nothing is held, which skips all of it; and one over the width of
    /// the ground the texture wraps round in.
    pub fn window(&self) -> Vec4 {
        let metres = TILE as f32 * self.cell;
        let half = if self.filled == 0 {
            0.0
        } else {
            // Half a square short of the edge: the window moves a square at a time.
            ((TILES / 2) as f32 - 0.5) * metres
        };
        Vec4::new(
            self.middle.0 as f32 * metres,
            self.middle.1 as f32 * metres,
            half,
            1.0 / (TILES as f32 * metres),
        )
    }

    /// The width of a cell, in metres.
    pub fn cell(&self) -> f32 {
        self.cell
    }

    /// How many squares of ruts are held.
    pub fn filled(&self) -> usize {
        self.filled
    }
}

/// A texture of ruts `side` texels a side, at every size, all of it untouched ground.
fn make(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    side: u32,
) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("ruts"),
        size: wgpu::Extent3d {
            width: side,
            height: side,
            depth_or_array_layers: 2,
        },
        mip_level_count: LEVELS,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    // Untouched ground is not all zeros: its height is the middle of the range.
    for level in 0..LEVELS {
        let side = side >> level;
        for (layer, cell) in UNTOUCHED.iter().enumerate() {
            let texels = cell.repeat((side * side) as usize);
            write(queue, &texture, level, layer as u32, (0, 0), side, &texels);
        }
    }
    let view = texture.create_view(&wgpu::TextureViewDescriptor {
        dimension: Some(wgpu::TextureViewDimension::D2Array),
        ..Default::default()
    });
    (texture, view)
}

/// Copies a square of texels into one size of one layer.
fn write(
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    level: u32,
    layer: u32,
    at: (u32, u32),
    side: u32,
    texels: &[u8],
) {
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: level,
            origin: wgpu::Origin3d {
                x: at.0,
                y: at.1,
                z: layer,
            },
            aspect: wgpu::TextureAspect::All,
        },
        texels,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(side * 4),
            rows_per_image: Some(side),
        },
        wgpu::Extent3d {
            width: side,
            height: side,
            depth_or_array_layers: 1,
        },
    );
}
