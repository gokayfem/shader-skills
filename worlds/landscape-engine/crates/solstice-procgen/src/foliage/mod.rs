//! Vegetation as real geometry: grass, trees and bushes.
//!
//! Every blade and leaf is a few triangles of its own shape, not a picture on a transparent card.
//! That is what Unreal does for Nanite foliage, and it suits a cluster renderer for the same
//! reasons: nothing is drawn and then thrown away by an alpha test, edges stay sharp at any
//! distance, and the cluster builder thins the pieces out with distance while keeping the area
//! they cover, so a meadow stays as dense from far away as it is close up.
//!
//! Materials using these meshes should be [`Material::FOLIAGE`](solstice_asset::Material::FOLIAGE)
//! and two-sided, so that light from behind shines through.
//!
//! Barks are surfaces of the procedural library, found by name like any other, and each kind of
//! leaf has its own atlas. A [`species::Species`] names one of each and is plain data with a text
//! form, so a new tree, or a change to one, is a matter of description rather than code.
//!
//! Each leaf and blade texture here is an atlas of variants side by side, and each blade or leaf picks one, so a
//! single material gives a field of many shades.

pub mod bark;
pub mod grass;
pub mod leaf;
pub mod species;
mod text;
pub mod tree;
pub mod weed;

/// A small, fast, deterministic random number generator (PCG32), so a seed always grows the same
/// plant on every platform.
#[derive(Clone, Debug)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u32, stream: u32) -> Self {
        let mut rng =
            Self((u64::from(seed) << 32 | u64::from(stream)).wrapping_add(0x853c_49e6_748f_ea9b));
        rng.next_u32();
        rng
    }

    pub fn next_u32(&mut self) -> u32 {
        let old = self.0;
        self.0 = old
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        let shifted = (((old >> 18) ^ old) >> 27) as u32;
        shifted.rotate_right((old >> 59) as u32)
    }

    /// A float in `0..1`.
    pub fn unit(&mut self) -> f32 {
        (self.next_u32() >> 8) as f32 / 16_777_216.0
    }

    /// A float in `low..high`.
    pub fn range(&mut self, low: f32, high: f32) -> f32 {
        low + (high - low) * self.unit()
    }

    /// A float in `-1..1`.
    pub fn signed(&mut self) -> f32 {
        self.unit() * 2.0 - 1.0
    }

    /// An index in `0..count`.
    pub fn index(&mut self, count: usize) -> usize {
        ((self.unit() * count as f32) as usize).min(count.saturating_sub(1))
    }
}
