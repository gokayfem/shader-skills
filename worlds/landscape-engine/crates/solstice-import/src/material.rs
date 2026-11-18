//! Converts glTF materials and images to the renderer's texture format.
//!
//! The renderer's layers hold, per texel:
//! * colour: RGB base colour (sRGB), A cavity from the occlusion map, or coverage for a material
//!   that is cut out (`docs/cutouts.md`).
//! * normal: RG tangent-space normal.
//! * material: R roughness multiplier, G metallic multiplier.

use glam::Vec2;
use gltf::image::{Data as ImageData, Format};
use gltf::Document;
use rayon::prelude::*;
use solstice_asset::{Material, TextureLayer};
use solstice_builder::{masked_mip_chain, mip_chain, move_alpha_threshold};
use std::collections::HashMap;

/// A `KHR_texture_transform`, applied to UVs while reading geometry.
///
/// The renderer has one UV set per mesh, so all of a material's maps must share a transform.
/// glTF allows them to differ, which [`MaterialTable`] warns about.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UvTransform {
    pub offset: Vec2,
    pub rotation: f32,
    pub scale: Vec2,
}

impl UvTransform {
    pub const IDENTITY: Self = Self {
        offset: Vec2::ZERO,
        rotation: 0.0,
        scale: Vec2::ONE,
    };

    /// The extension's definition: scale, then rotate clockwise in texture space, then offset.
    pub fn apply(&self, uv: Vec2) -> Vec2 {
        let (sin, cos) = self.rotation.sin_cos();
        let scaled = uv * self.scale;
        Vec2::new(
            self.offset.x + cos * scaled.x + sin * scaled.y,
            self.offset.y - sin * scaled.x + cos * scaled.y,
        )
    }
}

fn to_uv_transform(transform: Option<gltf::texture::TextureTransform<'_>>) -> UvTransform {
    transform.map_or(UvTransform::IDENTITY, |t| UvTransform {
        offset: Vec2::from(t.offset()),
        rotation: t.rotation(),
        scale: Vec2::from(t.scale()),
    })
}

/// The transforms of a material's maps. Only colour and metallic-roughness can report one
/// through the glTF crate, so normal and occlusion maps are assumed to match them.
pub(crate) fn map_transforms(material: &gltf::Material<'_>) -> Vec<UvTransform> {
    let pbr = material.pbr_metallic_roughness();
    let mut transforms = Vec::new();
    if let Some(t) = pbr.base_color_texture() {
        transforms.push(to_uv_transform(t.texture_transform()));
    }
    if let Some(t) = pbr.metallic_roughness_texture() {
        transforms.push(to_uv_transform(t.texture_transform()));
    }
    transforms
}

/// The transform to apply to a primitive's UVs.
pub fn uv_transform(material: &gltf::Material<'_>) -> UvTransform {
    map_transforms(material)
        .first()
        .copied()
        .unwrap_or(UvTransform::IDENTITY)
}

/// Which source images make up one texture layer.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct LayerKey {
    base_color: Option<usize>,
    normal: Option<usize>,
    metallic_roughness: Option<usize>,
    occlusion: Option<usize>,
    /// Where alpha holds coverage rather than cavity: the source alpha that becomes 0.5.
    coverage: Option<u16>,
}

/// A threshold no texture alpha reaches.
const NEVER_PASSES: u16 = 256;

/// The texture alpha, of 255, at which a `MASK` material's surface becomes solid: its cut-off,
/// allowing for the factor glTF multiplies alpha by. `None` for any other alpha mode.
fn mask_threshold(material: &gltf::Material<'_>) -> Option<u16> {
    if material.alpha_mode() != gltf::material::AlphaMode::Mask {
        return None;
    }
    let cutoff = material.alpha_cutoff().unwrap_or(0.5);
    let factor = material.pbr_metallic_roughness().base_color_factor()[3];
    Some(if cutoff <= 0.0 {
        0
    } else if factor < cutoff {
        NEVER_PASSES
    } else {
        // Solid is at or above the cut-off, so this rounds up.
        ((cutoff / factor * 255.0).ceil() as u16).min(255)
    })
}

/// Whether a material can never show, because its factor is below its own cut-off.
pub fn is_invisible(material: &gltf::Material<'_>) -> bool {
    mask_threshold(material) == Some(NEVER_PASSES)
}

/// Collects materials and texture layers, sharing them where the source does.
pub struct MaterialTable<'a> {
    images: &'a [ImageData],
    texture_size: u32,
    materials: Vec<Material>,
    slots: HashMap<Option<usize>, u32>,
    layer_slots: HashMap<LayerKey, i32>,
    layers: Vec<TextureLayer>,
    /// The base colour images `MASK` materials cut out of, each with the first such threshold.
    /// An atlas can hold masked and opaque cells, so a layer made from one of these keeps its
    /// coverage for every material on it, and opaque cells read it as a cavity of 1.
    mask_images: HashMap<usize, u16>,
}

impl<'a> MaterialTable<'a> {
    pub fn new(document: &Document, images: &'a [ImageData], texture_size: u32) -> Self {
        let mut mask_images = HashMap::new();
        for material in document.materials() {
            let base = material.pbr_metallic_roughness().base_color_texture();
            if let (Some(threshold), Some(base)) = (mask_threshold(&material), base) {
                if threshold < NEVER_PASSES {
                    mask_images
                        .entry(base.texture().source().index())
                        .or_insert(threshold);
                }
            }
        }
        Self {
            images,
            texture_size,
            materials: Vec::new(),
            slots: HashMap::new(),
            layer_slots: HashMap::new(),
            layers: Vec::new(),
            mask_images,
        }
    }

    /// Returns the slot for a glTF material index, where `None` is glTF's default material.
    pub fn slot(
        &mut self,
        document: &Document,
        index: Option<usize>,
        warnings: &mut Vec<String>,
    ) -> u32 {
        if let Some(&slot) = self.slots.get(&index) {
            return slot;
        }
        let material = match index.and_then(|i| document.materials().nth(i)) {
            Some(source) => self.convert(&source, warnings),
            None => Material {
                base_color: [1.0; 4],
                roughness: 1.0,
                metallic: 1.0,
                ..Default::default()
            },
        };
        let slot = self.materials.len() as u32;
        self.materials.push(material);
        self.slots.insert(index, slot);
        slot
    }

    pub fn finish(self) -> (Vec<Material>, Vec<TextureLayer>) {
        (self.materials, self.layers)
    }

    fn convert(&mut self, source: &gltf::Material<'_>, warnings: &mut Vec<String>) -> Material {
        let name = source.name().unwrap_or("unnamed");
        if source.emissive_texture().is_some() {
            warnings.push(format!(
                "material '{name}': emissive texture is ignored, only its factor is used"
            ));
        }

        if source.alpha_mode() == gltf::material::AlphaMode::Blend && !is_glass(source) {
            warnings.push(format!(
                "material '{name}': alpha mode Blend with an opaque colour is drawn as opaque"
            ));
        }

        let pbr = source.pbr_metallic_roughness();
        let mut uses_other_uv_set = false;
        let mut image_of = |set: u32, index: usize| {
            uses_other_uv_set |= set != 0;
            index
        };
        let base_color = pbr
            .base_color_texture()
            .map(|t| image_of(t.tex_coord(), t.texture().source().index()));
        // With no texture to cut out of, a mask is solid: `is_invisible` has dealt with the
        // alternative.
        let masked = mask_threshold(source).filter(|_| base_color.is_some());
        let mut occlusion = source
            .occlusion_texture()
            .map(|t| image_of(t.tex_coord(), t.texture().source().index()));
        if masked.is_some() && occlusion.take().is_some() {
            warnings.push(format!(
                "material '{name}': its occlusion map is ignored, as it is cut out"
            ));
        }
        // An opaque material on a masked atlas shares the layer, unless it has a cavity of its own
        // to keep there.
        let shared = base_color
            .filter(|_| occlusion.is_none())
            .and_then(|image| self.mask_images.get(&image).copied());
        let key = LayerKey {
            base_color,
            normal: source
                .normal_texture()
                .map(|t| image_of(t.tex_coord(), t.texture().source().index())),
            metallic_roughness: pbr
                .metallic_roughness_texture()
                .map(|t| image_of(t.tex_coord(), t.texture().source().index())),
            occlusion,
            coverage: masked.or(shared),
        };
        let transforms = map_transforms(source);
        if transforms.windows(2).any(|pair| pair[0] != pair[1]) {
            warnings.push(format!(
                "material '{name}': its maps use different texture transforms, only the first is applied"
            ));
        }
        if uses_other_uv_set {
            warnings.push(format!(
                "material '{name}': only the first UV set is supported"
            ));
        }

        let has_textures = key.base_color.is_some()
            || key.normal.is_some()
            || key.metallic_roughness.is_some()
            || key.occlusion.is_some();
        let texture_layer = if has_textures {
            self.layer_slot(key)
        } else {
            -1
        };

        let [er, eg, eb] = source.emissive_factor();
        Material {
            base_color: pbr.base_color_factor(),
            emissive: [er, eg, eb],
            flags: named_flags(source.name())
                | if source.double_sided() {
                    Material::TWO_SIDED
                } else {
                    0
                }
                | if masked.is_some() {
                    Material::MASKED
                } else {
                    0
                }
                | if is_glass(source) { Material::GLASS } else { 0 },
            roughness: pbr.roughness_factor(),
            metallic: pbr.metallic_factor(),
            uv_scale: 1.0,
            normal_strength: source.normal_texture().map_or(0.0, |t| t.scale()),
            texture_layer,
            ..Default::default()
        }
    }

    fn layer_slot(&mut self, key: LayerKey) -> i32 {
        if let Some(&slot) = self.layer_slots.get(&key) {
            return slot;
        }
        let layer = compose_layer(self.images, key, self.texture_size);
        let slot = self.layers.len() as i32;
        self.layers.push(layer);
        self.layer_slots.insert(key, slot);
        slot
    }
}

fn compose_layer(images: &[ImageData], key: LayerKey, size: u32) -> TextureLayer {
    let image = |index: Option<usize>| index.map(|i| &images[i]);
    let (base, normal, metallic_roughness, occlusion) = (
        image(key.base_color),
        image(key.normal),
        image(key.metallic_roughness),
        image(key.occlusion),
    );
    let n = size as usize;
    let texels: Vec<([u8; 4], [u8; 2], [u8; 2])> = (0..n * n)
        .into_par_iter()
        .map(|i| {
            let (u, v) = (
                ((i % n) as f32 + 0.5) / size as f32,
                ((i / n) as f32 + 0.5) / size as f32,
            );
            // Through alpha when cutting out, so shrinking doesn't blend a hole's colour into the
            // strands beside it.
            let base = base.map_or([255; 4], |im| {
                if key.coverage.is_some() {
                    sample_masked(im, u, v, size)
                } else {
                    sample(im, u, v, size)
                }
            });
            let alpha = match key.coverage {
                Some(threshold) => move_alpha_threshold(base[3], threshold),
                None => occlusion.map_or(255, |im| sample(im, u, v, size)[0]),
            };
            let normal = normal.map_or([128, 128, 255, 255], |im| sample(im, u, v, size));
            // glTF stores roughness in G and metallic in B. Roughness is halved because the
            // layer format lets a texture raise roughness as well as lower it.
            let orm = metallic_roughness.map_or([255; 4], |im| sample(im, u, v, size));
            (
                [base[0], base[1], base[2], alpha],
                // glTF's green points towards decreasing V, and the renderer's derives its
                // frame from the direction of increasing V.
                [normal[0], 255 - normal[1]],
                [orm[1].div_ceil(2), orm[2]],
            )
        })
        .collect();

    let mut color = Vec::with_capacity(n * n * 4);
    let mut normals = Vec::with_capacity(n * n * 2);
    let mut material = Vec::with_capacity(n * n * 2);
    for (c, nor, mat) in &texels {
        color.extend_from_slice(c);
        normals.extend_from_slice(nor);
        material.extend_from_slice(mat);
    }
    TextureLayer {
        color_mips: if key.coverage.is_some() {
            masked_mip_chain(color, size)
        } else {
            mip_chain(color, size, 4)
        },
        normal_mips: mip_chain(normals, size, 2),
        material_mips: mip_chain(material, size, 2),
    }
}

/// Reads a source image at output resolution `size`, averaging when shrinking so that a
/// large image does not alias. Wraps at the borders, as glTF's default sampler does.
pub(crate) fn sample(image: &ImageData, u: f32, v: f32, size: u32) -> [u8; 4] {
    sample_weighted(image, u, v, size, false)
}

/// [`sample`] with colour averaged by each tap's alpha, so holes don't darken a mask's edges.
pub(crate) fn sample_masked(image: &ImageData, u: f32, v: f32, size: u32) -> [u8; 4] {
    sample_weighted(image, u, v, size, true)
}

fn sample_weighted(image: &ImageData, u: f32, v: f32, size: u32, by_alpha: bool) -> [u8; 4] {
    let taps = (image.width.max(image.height) / size).clamp(1, 8);
    let mut colour = [0.0f32; 3];
    let (mut colour_weight, mut alpha) = (0.0, 0.0);
    for ty in 0..taps {
        for tx in 0..taps {
            let du = (tx as f32 + 0.5) / taps as f32 - 0.5;
            let dv = (ty as f32 + 0.5) / taps as f32 - 0.5;
            let (tap_u, tap_v) = (u + du / size as f32, v + dv / size as f32);
            let texel = if by_alpha {
                bilinear_by_alpha(image, tap_u, tap_v)
            } else {
                bilinear(image, tap_u, tap_v)
            };
            let weight = if by_alpha { texel[3] } else { 1.0 };
            for channel in 0..3 {
                colour[channel] += texel[channel] * weight;
            }
            colour_weight += weight;
            alpha += texel[3];
        }
    }
    let count = (taps * taps) as f32;
    let [r, g, b] = colour.map(|value| {
        if colour_weight > 0.0 {
            (value / colour_weight + 0.5) as u8
        } else {
            0
        }
    });
    [r, g, b, (alpha / count + 0.5) as u8]
}

fn bilinear(image: &ImageData, u: f32, v: f32) -> [f32; 4] {
    let x = u * image.width as f32 - 0.5;
    let y = v * image.height as f32 - 0.5;
    let (x0, y0) = (x.floor(), y.floor());
    let (fx, fy) = (x - x0, y - y0);
    let at = |dx: i64, dy: i64| texel(image, x0 as i64 + dx, y0 as i64 + dy);
    let (a, b, c, d) = (at(0, 0), at(1, 0), at(0, 1), at(1, 1));
    std::array::from_fn(|channel| {
        let top = a[channel] + (b[channel] - a[channel]) * fx;
        let bottom = c[channel] + (d[channel] - c[channel]) * fx;
        top + (bottom - top) * fy
    })
}

/// [`bilinear`] with colour averaged by alpha as well, so a hole's colour doesn't reach a mask's
/// edge. Alpha is filtered as usual.
fn bilinear_by_alpha(image: &ImageData, u: f32, v: f32) -> [f32; 4] {
    let x = u * image.width as f32 - 0.5;
    let y = v * image.height as f32 - 0.5;
    let (x0, y0) = (x.floor(), y.floor());
    let (fx, fy) = (x - x0, y - y0);
    let corners = [
        (0, 0, (1.0 - fx) * (1.0 - fy)),
        (1, 0, fx * (1.0 - fy)),
        (0, 1, (1.0 - fx) * fy),
        (1, 1, fx * fy),
    ];
    let mut colour = [0.0f32; 3];
    let (mut colour_weight, mut alpha) = (0.0, 0.0);
    for (dx, dy, weight) in corners {
        let texel = texel(image, x0 as i64 + dx, y0 as i64 + dy);
        alpha += texel[3] * weight;
        colour_weight += texel[3] * weight;
        for channel in 0..3 {
            colour[channel] += texel[channel] * texel[3] * weight;
        }
    }
    let [r, g, b] = if colour_weight > 0.0 {
        colour.map(|value| value / colour_weight)
    } else {
        [0.0; 3]
    };
    [r, g, b, alpha]
}

/// One texel as RGBA in `0.0..=255.0`, whatever the source format.
fn texel(image: &ImageData, x: i64, y: i64) -> [f32; 4] {
    let x = x.rem_euclid(i64::from(image.width)) as usize;
    let y = y.rem_euclid(i64::from(image.height)) as usize;
    let index = y * image.width as usize + x;
    let p = &image.pixels;
    let u8_at = |offset: usize| f32::from(p[offset]);
    // 16-bit data is little-endian, so its high byte holds the most significant bits.
    let u16_at = |offset: usize| f32::from(p[offset * 2 + 1]);
    let f32_at = |offset: usize| {
        let bytes = [
            p[offset * 4],
            p[offset * 4 + 1],
            p[offset * 4 + 2],
            p[offset * 4 + 3],
        ];
        f32::from_le_bytes(bytes).clamp(0.0, 1.0) * 255.0
    };
    match image.format {
        Format::R8 => [u8_at(index), u8_at(index), u8_at(index), 255.0],
        Format::R8G8 => [u8_at(index * 2), u8_at(index * 2 + 1), 0.0, 255.0],
        Format::R8G8B8 => [
            u8_at(index * 3),
            u8_at(index * 3 + 1),
            u8_at(index * 3 + 2),
            255.0,
        ],
        Format::R8G8B8A8 => [
            u8_at(index * 4),
            u8_at(index * 4 + 1),
            u8_at(index * 4 + 2),
            u8_at(index * 4 + 3),
        ],
        Format::R16 => [u16_at(index), u16_at(index), u16_at(index), 255.0],
        Format::R16G16 => [u16_at(index * 2), u16_at(index * 2 + 1), 0.0, 255.0],
        Format::R16G16B16 => [
            u16_at(index * 3),
            u16_at(index * 3 + 1),
            u16_at(index * 3 + 2),
            255.0,
        ],
        Format::R16G16B16A16 => [
            u16_at(index * 4),
            u16_at(index * 4 + 1),
            u16_at(index * 4 + 2),
            u16_at(index * 4 + 3),
        ],
        Format::R32G32B32FLOAT => [
            f32_at(index * 3),
            f32_at(index * 3 + 1),
            f32_at(index * 3 + 2),
            255.0,
        ],
        Format::R32G32B32A32FLOAT => [
            f32_at(index * 4),
            f32_at(index * 4 + 1),
            f32_at(index * 4 + 2),
            f32_at(index * 4 + 3),
        ],
    }
}

/// Whether a material is glass: blended, and seen through by its colour's own alpha. Glass is
/// drawn as a share of its pixels, by that alpha alone, so a blended material whose colour is
/// opaque stays opaque, as it was before glass, and keeps off the masked path it would not use.
pub(crate) fn is_glass(material: &gltf::Material<'_>) -> bool {
    material.alpha_mode() == gltf::material::AlphaMode::Blend
        && material.pbr_metallic_roughness().base_color_factor()[3] < 1.0
}

/// What a material's name says of it: one whose name begins `paint` is the paint of
/// something driven, and one whose name begins `inside` is in its cab or under its bonnet
/// (`Material::PAINT`, `Material::INSIDE`). A model is given such names when it is made ready.
pub(crate) fn named_flags(name: Option<&str>) -> u32 {
    let name = name.unwrap_or("").to_ascii_lowercase();
    if name.starts_with("paint") {
        Material::PAINT
    } else if name.starts_with("inside") {
        Material::INSIDE
    } else {
        0
    }
}
