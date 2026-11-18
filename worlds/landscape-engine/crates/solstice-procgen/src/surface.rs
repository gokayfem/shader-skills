//! Turns a description of a material into texture maps.
//!
//! The better way to describe one is [`Surface::shaped`]: build its real shape first, in metres,
//! then decide its colour by looking at that shape. Dirt collects in hollows and wear shows on
//! high points because the colour comes from those very hollows and high points, which is what
//! separates a believable material from coloured noise.

use glam::{Vec2, Vec3};
use rayon::prelude::*;
use solstice_asset::TextureLayer;
use solstice_builder::mip_chain;

/// A material at one point. Colours are linear albedo.
#[derive(Clone, Copy, Debug)]
pub struct Texel {
    pub color: Vec3,
    /// Relative height, from which the normal map is derived.
    pub height: f32,
    pub roughness: f32,
    /// How exposed this point is to ambient light: 0 in a deep crevice, 1 in the open.
    pub cavity: f32,
}

/// How many metres one repeat of a shaped surface covers unless it says otherwise, which gives
/// its heights real meaning. See [`Surface::with_tile_metres`].
pub const TILE_METRES: f32 = 4.0;

/// What a shaped surface's colour can depend on at a point. Heights are in metres, relative to
/// the surroundings at three scales: positive on a rise, negative in a hollow.
#[derive(Clone, Copy, Debug)]
pub struct Relief {
    pub height: f32,
    /// Against about the nearest 2 cm: grain, chips, fine cracks.
    pub fine: f32,
    /// Against about the nearest 10 cm: individual stones and clumps.
    pub medium: f32,
    /// Against about the nearest 50 cm: mounds and dips.
    pub broad: f32,
}

/// What a shaped surface decides from its [`Relief`].
#[derive(Clone, Copy, Debug)]
pub struct Shade {
    pub color: Vec3,
    pub roughness: f32,
    pub cavity: f32,
}

type HeightFn = dyn Fn(f32, f32) -> (f32, f32) + Send + Sync;
type ShadeFn = dyn Fn(f32, f32, f32, Relief) -> Shade + Send + Sync;

enum Source {
    Direct {
        normal_strength: f32,
        sample: Box<dyn Fn(f32, f32) -> Texel + Send + Sync>,
    },
    Shaped {
        height: Box<HeightFn>,
        shade: Box<ShadeFn>,
    },
}

/// A seamlessly tiling material, ready to be generated at any size.
pub struct Surface {
    source: Source,
    tile_metres: f32,
}

impl Surface {
    /// A surface described directly at each point. `normal_strength` scales how steep height
    /// differences appear. `sample` must tile over `0..1`, as anything from
    /// [`field`](crate::field) does.
    pub fn new(
        normal_strength: f32,
        sample: impl Fn(f32, f32) -> Texel + Send + Sync + 'static,
    ) -> Self {
        Self {
            source: Source::Direct {
                normal_strength,
                sample: Box::new(sample),
            },
            tile_metres: TILE_METRES,
        }
    }

    /// A surface described by its shape. `height` gives metres at a point, plus any number the
    /// colour will want, such as which stone this is. `shade` then gets that number and the
    /// [`Relief`] there. Both must tile over `0..1`.
    pub fn shaped(
        height: impl Fn(f32, f32) -> (f32, f32) + Send + Sync + 'static,
        shade: impl Fn(f32, f32, f32, Relief) -> Shade + Send + Sync + 'static,
    ) -> Self {
        Self {
            source: Source::Shaped {
                height: Box::new(height),
                shade: Box::new(shade),
            },
            tile_metres: TILE_METRES,
        }
    }

    /// Sets how many metres one repeat of this surface covers. A smaller tile packs the same
    /// texture size into less of the world, for materials seen close up whose detail is a few
    /// millimetres, such as bark. Materials using it set their UV scale to match.
    pub fn with_tile_metres(mut self, metres: f32) -> Self {
        self.tile_metres = metres;
        self
    }

    /// How many metres one repeat of this surface covers.
    pub fn tile_metres(&self) -> f32 {
        self.tile_metres
    }

    /// The material at one point, with shaped surfaces read as if flat around it. Only for testing
    /// that a surface's formulas agree across the tile's border, since [`Relief`] needs a whole grid.
    #[cfg(test)]
    pub(crate) fn formula_at(&self, u: f32, v: f32) -> Texel {
        match &self.source {
            Source::Direct { sample, .. } => sample(u, v),
            Source::Shaped { height, shade } => {
                let (metres, tag) = height(u, v);
                let relief = Relief {
                    height: metres,
                    fine: 0.0,
                    medium: 0.0,
                    broad: 0.0,
                };
                let shaded = shade(u, v, tag, relief);
                Texel {
                    color: shaded.color,
                    height: metres,
                    roughness: shaded.roughness,
                    cavity: shaded.cavity,
                }
            }
        }
    }

    /// The material at every texel of a `size`x`size` grid, and the normal strength that suits.
    fn texels(&self, size: u32) -> (Vec<Texel>, f32) {
        let n = size as usize;
        let position = |i: usize| {
            (
                ((i % n) as f32 + 0.5) / size as f32,
                ((i / n) as f32 + 0.5) / size as f32,
            )
        };
        match &self.source {
            Source::Direct {
                normal_strength,
                sample,
            } => {
                let texels = (0..n * n)
                    .into_par_iter()
                    .map(|i| {
                        let (u, v) = position(i);
                        sample(u, v)
                    })
                    .collect();
                (texels, *normal_strength)
            }
            Source::Shaped { height, shade } => {
                let shape: Vec<(f32, f32)> = (0..n * n)
                    .into_par_iter()
                    .map(|i| {
                        let (u, v) = position(i);
                        height(u, v)
                    })
                    .collect();
                let heights: Vec<f32> = shape.iter().map(|s| s.0).collect();
                let radius = |metres: f32| {
                    ((metres / self.tile_metres * size as f32).round() as usize).clamp(1, n / 4 + 1)
                };
                let fine = blur(&heights, n, radius(0.01));
                let medium = blur(&heights, n, radius(0.05));
                let broad = blur(&heights, n, radius(0.25));
                let texels = (0..n * n)
                    .into_par_iter()
                    .map(|i| {
                        let (u, v) = position(i);
                        let relief = Relief {
                            height: heights[i],
                            fine: heights[i] - fine[i],
                            medium: heights[i] - medium[i],
                            broad: heights[i] - broad[i],
                        };
                        let shaded = shade(u, v, shape[i].1, relief);
                        Texel {
                            color: shaded.color,
                            height: heights[i],
                            roughness: shaded.roughness,
                            cavity: shaded.cavity,
                        }
                    })
                    .collect();
                // The slope in real units: height change per metre across two texels.
                (texels, 0.5 / self.tile_metres)
            }
        }
    }

    /// Generates texture maps of `size`x`size` with full mip chains, uncompressed.
    pub fn bake(&self, size: u32) -> TextureLayer {
        match self.source {
            Source::Direct { .. } => self.bake_exactly(size),
            // Shapes have edges finer than a texel. Generating at twice the size and averaging
            // down keeps them clean, where sampling once per texel would make them stair-step.
            Source::Shaped { .. } => {
                let mut layer = self.bake_exactly(size * 2);
                layer.color_mips.remove(0);
                layer.normal_mips.remove(0);
                layer.material_mips.remove(0);
                layer
            }
        }
    }

    fn bake_exactly(&self, size: u32) -> TextureLayer {
        let n = size as usize;
        let (texels, normal_strength) = self.texels(size);

        let mut color = Vec::with_capacity(n * n * 4);
        let mut normals = Vec::with_capacity(n * n * 2);
        let mut material = Vec::with_capacity(n * n * 2);
        let to_byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
        for y in 0..n {
            for x in 0..n {
                let texel = &texels[y * n + x];
                let height_at = |dx: i32, dy: i32| {
                    let px = (x as i32 + dx).rem_euclid(n as i32) as usize;
                    let py = (y as i32 + dy).rem_euclid(n as i32) as usize;
                    texels[py * n + px].height
                };
                let slope = Vec2::new(
                    height_at(1, 0) - height_at(-1, 0),
                    height_at(0, 1) - height_at(0, -1),
                ) * normal_strength
                    * size as f32;
                let normal = Vec3::new(-slope.x, -slope.y, 1.0).normalize();

                let srgb = texel.color.powf(1.0 / 2.2);
                color.extend_from_slice(&[
                    to_byte(srgb.x),
                    to_byte(srgb.y),
                    to_byte(srgb.z),
                    to_byte(texel.cavity),
                ]);
                normals.extend_from_slice(&[
                    to_byte(normal.x * 0.5 + 0.5),
                    to_byte(normal.y * 0.5 + 0.5),
                ]);
                material.extend_from_slice(&[
                    to_byte(texel.roughness * 0.5),
                    255, // Metallic multiplier: these materials set metalness by factor alone.
                ]);
            }
        }

        TextureLayer {
            color_mips: mip_chain(color, size, 4),
            normal_mips: mip_chain(normals, size, 2),
            material_mips: mip_chain(material, size, 2),
        }
    }
}

/// A soft blur over a tiling square: two box passes in each axis, which is close to a Gaussian.
fn blur(values: &[f32], n: usize, radius: usize) -> Vec<f32> {
    let mut current = values.to_vec();
    for _ in 0..2 {
        current = box_pass(&current, n, radius, 1, n);
        current = box_pass(&current, n, radius, n, 1);
    }
    current
}

/// Averages along lines, in time that does not depend on `radius`. `step` moves along a line, and
/// `next` moves to the following line.
fn box_pass(values: &[f32], n: usize, radius: usize, step: usize, next: usize) -> Vec<f32> {
    let lines: Vec<Vec<f32>> = (0..n)
        .into_par_iter()
        .map(|line| {
            let at = |i: usize| values[line * next + (i % n) * step];
            let width = 2 * radius + 1;
            let mut sum: f32 = (0..width).map(|i| at(i + n - radius)).sum();
            let mut out = Vec::with_capacity(n);
            for i in 0..n {
                out.push(sum / width as f32);
                sum += at(i + radius + 1) - at(i + n - radius);
            }
            out
        })
        .collect();
    let mut result = vec![0.0; n * n];
    for (line, values) in lines.iter().enumerate() {
        for (i, value) in values.iter().enumerate() {
            result[line * next + i * step] = *value;
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blur_keeps_the_average_and_smooths_peaks() {
        let n = 32;
        let mut values = vec![0.0; n * n];
        values[5 * n + 7] = 1.0;
        let blurred = blur(&values, n, 3);
        let total: f32 = blurred.iter().sum();
        assert!((total - 1.0).abs() < 1.0e-3, "total {total}");
        assert!(blurred[5 * n + 7] < 0.2);
        assert!(blurred[5 * n + 7] > blurred[5 * n + 11]);
    }

    #[test]
    fn blur_wraps_around_the_edges() {
        let n = 16;
        let mut values = vec![0.0; n * n];
        values[0] = 1.0;
        let blurred = blur(&values, n, 2);
        assert!(blurred[n * n - 1] > 0.0, "reaches the opposite corner");
    }

    #[test]
    fn a_shaped_surface_uses_real_slopes() {
        // Rising 1 m per metre is 45 degrees.
        let rise = |u: f32| {
            let t = (u * 2.0).fract();
            (if t < 0.5 { t } else { 1.0 - t }) * TILE_METRES * 0.5
        };
        let surface = Surface::shaped(
            move |u, _| (rise(u), 0.0),
            |_, _, _, _| Shade {
                color: Vec3::ONE,
                roughness: 1.0,
                cavity: 1.0,
            },
        );
        let layer = surface.bake(64);
        let x = layer.normal_mips[0][2 * 8] as f32 / 127.5 - 1.0;
        assert!(
            (x.abs() - std::f32::consts::FRAC_1_SQRT_2).abs() < 0.03,
            "normal x {x}"
        );
    }

    #[test]
    fn layer_has_matching_chains() {
        let surface = Surface::new(0.1, |u, _| Texel {
            color: Vec3::splat(u),
            height: 0.5,
            roughness: 0.5,
            cavity: 1.0,
        });
        let layer = surface.bake(16);
        assert_eq!(layer.color_mips.len(), layer.normal_mips.len());
        assert_eq!(layer.color_mips.len(), layer.material_mips.len());
        assert_eq!(layer.color_mips[0].len(), 16 * 16 * 4);
        assert_eq!(layer.normal_mips[0].len(), 16 * 16 * 2);
    }

    #[test]
    fn flat_height_gives_a_flat_normal() {
        let surface = Surface::new(0.5, |_, _| Texel {
            color: Vec3::ONE,
            height: 0.5,
            roughness: 1.0,
            cavity: 1.0,
        });
        let layer = surface.bake(4);
        assert_eq!(layer.normal_mips[0][0], 128);
        assert_eq!(layer.normal_mips[0][1], 128);
    }
}
