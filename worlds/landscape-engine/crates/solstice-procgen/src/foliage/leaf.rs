//! Kinds of leaf: the outline each leaf is built with, and the atlas that colours it.
//!
//! An atlas has [`LEAF_VARIANTS`] columns side by side, each a leaf from stalk (v = 0) to tip
//! (v = 1) and across it (u), with the midrib down the middle. Each leaf picks a column, so one
//! material gives a crown of many shades. Atlases do not tile, so they are listed here rather
//! than in the surface library.

use crate::field::{fbm, smoothstep, Field};
use crate::surface::{Surface, Texel};
use glam::Vec3;
use std::f32::consts::TAU;

/// Colour variants in each leaf atlas, side by side.
pub const LEAF_VARIANTS: u32 = 4;

/// The kinds of leaf.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LeafKind {
    /// An oval leaf, widest a third of the way along, as on an oak or a beech.
    Broadleaf,
    /// A small leaf, widest near its base and drawn to a point, bright yellow-green.
    Birch,
    /// Needles splayed either side of a shoot, dark blue-green, as on a fir. They are built as
    /// fans of needles; see `tree::add_needle_spray`.
    Needles,
    /// A long, narrow leaf drawn to a fine point, widest below its middle, a fresh yellow-green
    /// that is silvery beneath, as on a willow.
    Willow,
    /// An oak's leaf: leathery, deep green and glossy above, with a strong pale midrib, widest
    /// beyond its middle.
    Oak,
    /// A eucalypt's adult leaf: a long, narrow sickle that hangs edge-on to the sun, leathery,
    /// a dull grey-green the same on both faces, with the new growth bronze.
    Gum,
    /// A wattle's: feathery sprays of tiny grey-green leaflets, with its blossom among them,
    /// balls of gold. Built as sprays, as needles are.
    Wattle,
    /// A fern's frond: leaflets either side of a long arching stalk, fresh green. Built as a
    /// spray, the whole frond one.
    Fern,
    /// A grass tree's: a blade as long as an arm and thin as a skewer, arching out of the
    /// crown, grey-green, with the old ones straw.
    GrassTree,
}

impl LeafKind {
    pub const ALL: [Self; 9] = [
        Self::Broadleaf,
        Self::Birch,
        Self::Needles,
        Self::Willow,
        Self::Oak,
        Self::Gum,
        Self::Wattle,
        Self::Fern,
        Self::GrassTree,
    ];

    /// The name of this kind's atlas.
    pub fn name(self) -> &'static str {
        match self {
            Self::Broadleaf => "broadleaf_leaves",
            Self::Birch => "birch_leaves",
            Self::Needles => "fir_needles",
            Self::Willow => "willow_leaves",
            Self::Oak => "oak_leaves",
            Self::Gum => "gum_leaves",
            Self::Wattle => "wattle_leaves",
            Self::Fern => "fern_fronds",
            Self::GrassTree => "grass_tree_leaves",
        }
    }

    /// Whether this kind is built as a spray: small leaves splayed either side of a shoot, as
    /// a fir's needles are, rather than as one leaf.
    pub fn is_spray(self) -> bool {
        matches!(self, Self::Needles | Self::Wattle | Self::Fern)
    }

    pub fn by_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.name() == name)
    }

    /// Half the leaf's width at `s` of the way from stalk to tip, as a fraction of its full
    /// width: zero at both ends, and 0.5 where it is widest.
    pub fn half_width(self, s: f32) -> f32 {
        // s^a (1 - s)^b peaks at a / (a + b).
        let (a, b): (f32, f32) = match self {
            Self::Broadleaf => (0.6, 0.9),
            Self::Birch => (0.45, 1.2),
            // Nearly as wide along its whole length, rounding off at each end.
            Self::Needles => (0.2, 0.25),
            // A lance: widest two fifths of the way along, and long in the point.
            Self::Willow => (0.6, 0.95),
            // Widest beyond the middle, where an oak leaf's largest lobes are.
            Self::Oak => (0.75, 0.6),
            // A lance: widest a quarter of the way along, then a long, fine point.
            Self::Gum => (0.5, 1.35),
            Self::Wattle | Self::Fern => (0.2, 0.25),
            // A blade: as wide all the way, drawn to a point only at its very end.
            Self::GrassTree => (0.1, 0.45),
        };
        let peak = a / (a + b);
        let max = peak.powf(a) * (1.0 - peak).powf(b);
        0.5 * s.clamp(0.0, 1.0).powf(a) * (1.0 - s.clamp(0.0, 1.0)).powf(b) / max
    }

    /// Fractions of a leaf's length at which it has a full cross-section, between its stalk and
    /// its tip. A narrow leaf needs one; a broad one two, to round its outline.
    pub fn rows(self) -> &'static [f32] {
        match self {
            Self::Willow | Self::Gum => &[0.42],
            Self::GrassTree => &[0.35, 0.7],
            _ => &[0.32, 0.68],
        }
    }

    /// How far the leaf curls down at its tip, as a fraction of its length.
    pub fn curl(self) -> f32 {
        match self {
            Self::Broadleaf => 0.18,
            Self::Birch => 0.12,
            Self::Needles => 0.06,
            // Long leaves arc over under their own weight.
            Self::Willow => 0.3,
            Self::Oak => 0.14,
            Self::Gum => 0.34,
            Self::Wattle => 0.12,
            // A frond arches right over, and a grass tree's blade further still.
            Self::Fern => 0.42,
            Self::GrassTree => 0.6,
        }
    }

    /// Deepest of the fold along the midrib, as a fraction of half the width.
    pub fn fold(self) -> f32 {
        match self {
            Self::Broadleaf | Self::Birch | Self::Oak => 0.25,
            Self::Willow | Self::Gum => 0.35,
            // A shoot's needles splay out in a shallow V on either side.
            Self::Needles | Self::Wattle | Self::Fern => 0.45,
            Self::GrassTree => 0.6,
        }
    }

    pub fn atlas(self, seed: u32) -> Surface {
        match self {
            Self::Broadleaf => veined(seed, BROADLEAF, 7.0, 0.45),
            Self::Birch => veined(seed, BIRCH, 9.0, 0.4),
            Self::Needles => needles(seed, NEEDLES),
            Self::Willow => veined(seed, WILLOW, 13.0, 0.42),
            Self::Oak => veined(seed, OAK, 6.0, 0.36),
            // A gum leaf's side veins are fine and close, and run at a slant to a vein inside
            // its edge; its surface is waxy.
            Self::Gum => veined(seed, GUM, 22.0, 0.48),
            Self::Wattle => needles(seed, WATTLE),
            Self::Fern => needles(seed, FERN),
            Self::GrassTree => veined(seed, GRASS_TREE, 1.0, 0.55),
        }
    }
}

/// Linear albedo of each broadleaf variant: deep green to the first yellow of autumn.
const BROADLEAF: [Vec3; LEAF_VARIANTS as usize] = [
    Vec3::new(0.040, 0.085, 0.016),
    Vec3::new(0.060, 0.110, 0.020),
    Vec3::new(0.095, 0.135, 0.025),
    Vec3::new(0.200, 0.150, 0.030),
];
/// Birch leaves are lighter and yellower, and turn gold.
const BIRCH: [Vec3; LEAF_VARIANTS as usize] = [
    Vec3::new(0.070, 0.120, 0.020),
    Vec3::new(0.095, 0.145, 0.022),
    Vec3::new(0.120, 0.160, 0.030),
    Vec3::new(0.300, 0.220, 0.030),
];
/// Fir needles are dark, and blue-green on the young shoots.
const NEEDLES: [Vec3; LEAF_VARIANTS as usize] = [
    Vec3::new(0.022, 0.050, 0.020),
    Vec3::new(0.030, 0.062, 0.030),
    Vec3::new(0.040, 0.075, 0.045),
    Vec3::new(0.060, 0.085, 0.030),
];

/// Willow leaves are a light, fresh yellow-green, the youngest nearly lime, and the first to
/// turn pale gold.
const WILLOW: [Vec3; LEAF_VARIANTS as usize] = [
    Vec3::new(0.062, 0.115, 0.024),
    Vec3::new(0.085, 0.140, 0.028),
    Vec3::new(0.115, 0.165, 0.036),
    Vec3::new(0.230, 0.205, 0.045),
];
/// Oak leaves in high summer: deep, slightly blue green, the newer growth brighter, and a few
/// touched with the tan they turn.
const OAK: [Vec3; LEAF_VARIANTS as usize] = [
    Vec3::new(0.032, 0.075, 0.018),
    Vec3::new(0.046, 0.095, 0.020),
    Vec3::new(0.070, 0.120, 0.024),
    Vec3::new(0.150, 0.125, 0.035),
];

/// Gum leaves are a dull grey-green, olive in shade and bluer in the open, and the new growth
/// at the shoots' ends is bronze-red.
const GUM: [Vec3; LEAF_VARIANTS as usize] = [
    Vec3::new(0.040, 0.066, 0.034),
    Vec3::new(0.056, 0.084, 0.044),
    Vec3::new(0.075, 0.098, 0.060),
    Vec3::new(0.150, 0.085, 0.045),
];
/// A wattle's leaflets are a soft blue-grey green, and its blossom is gold.
const WATTLE: [Vec3; LEAF_VARIANTS as usize] = [
    Vec3::new(0.038, 0.070, 0.045),
    Vec3::new(0.052, 0.088, 0.058),
    Vec3::new(0.070, 0.105, 0.062),
    Vec3::new(0.620, 0.420, 0.030),
];
/// Fern fronds are a fresh, clear green, the old ones going bronze.
const FERN: [Vec3; LEAF_VARIANTS as usize] = [
    Vec3::new(0.030, 0.080, 0.018),
    Vec3::new(0.045, 0.105, 0.022),
    Vec3::new(0.062, 0.125, 0.028),
    Vec3::new(0.160, 0.110, 0.040),
];
/// A grass tree's blades are grey-green, and the dead ones in its skirt are straw.
const GRASS_TREE: [Vec3; LEAF_VARIANTS as usize] = [
    Vec3::new(0.050, 0.085, 0.034),
    Vec3::new(0.066, 0.100, 0.040),
    Vec3::new(0.085, 0.115, 0.045),
    Vec3::new(0.300, 0.240, 0.110),
];

/// Column and position across it at `u`: (variant, -1..1 across the leaf).
fn column(u: f32) -> (usize, f32) {
    let variant = ((u * LEAF_VARIANTS as f32) as usize).min(LEAF_VARIANTS as usize - 1);
    (variant, (u * LEAF_VARIANTS as f32).fract() * 2.0 - 1.0)
}

/// A leaf with a midrib and side veins leaving it towards the tip, `vein_count` pairs along it.
fn veined(
    seed: u32,
    colors: [Vec3; LEAF_VARIANTS as usize],
    vein_count: f32,
    roughness: f32,
) -> Surface {
    let mottle = fbm(24, 4, seed + 1);
    let blotches = fbm(8, 3, seed + 2);
    Surface::new(0.004, move |u, v| {
        let (variant, across) = column(u);
        let d = across.abs();
        let phase = v * vein_count - d * 2.2;
        let side_veins = (-((phase - phase.round()) * 9.0).powi(2)).exp() * (1.0 - d);
        let midrib = (-(across * 18.0).powi(2)).exp();
        let veins = side_veins.max(midrib);

        let mut color = colors[variant];
        color *= 1.0 + 0.18 * mottle.at(u, v) + 0.1 * blotches.at(u, v);
        // Veins are paler, and the edge is a little darker.
        color = color.lerp(color * Vec3::new(1.6, 1.5, 1.3), veins * 0.5);
        color *= 1.0 - 0.18 * smoothstep(0.75, 1.0, d);
        Texel {
            color,
            // Veins sit in grooves on a leaf's upper face.
            height: -veins,
            roughness,
            cavity: 1.0 - veins * 0.15,
        }
    })
}

/// Fans of needles, each from where it leaves the shoot (v = 0) to its tip (v = 1): several
/// needles converging on the tip, dark in the gaps between them and browner at the base, with
/// a paler, waxier tip on young growth.
fn needles(seed: u32, colors: [Vec3; LEAF_VARIANTS as usize]) -> Surface {
    let shade = fbm(16, 3, seed + 1);
    Surface::new(0.004, move |u, v| {
        let (variant, across) = column(u);
        // Five needles across the fan, closing up towards its tip.
        let needle = 0.5 + 0.5 * (across * 2.5 * TAU / (1.0 - v * 0.85)).cos();
        let gap = smoothstep(0.35, 0.0, needle) * (1.0 - smoothstep(0.75, 1.0, v));
        let mut color = colors[variant] * (1.0 + 0.2 * shade.at(u, v));
        color *= 0.8 + 0.35 * needle;
        // Browner where it leaves the shoot, but for blossom, which is gold all through.
        let woody = if colors[variant].x > 0.4 { 0.0 } else { 0.5 };
        color = color.lerp(
            Vec3::new(0.05, 0.045, 0.025),
            (1.0 - smoothstep(0.0, 0.25, v)) * woody,
        );
        color *= 1.0 - 0.55 * gap;
        Texel {
            color,
            height: needle * 0.5,
            roughness: 0.65,
            // Deep between needles and inside the spray, where little light gets in.
            cavity: (1.0 - 0.45 * gap) * (0.65 + 0.35 * smoothstep(0.0, 0.6, v)),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outlines_close_at_both_ends_and_are_half_wide_at_most() {
        for kind in LeafKind::ALL {
            assert!(kind.half_width(0.0) < 1.0e-6);
            assert!(kind.half_width(1.0) < 1.0e-6);
            let widest = (1..100)
                .map(|i| kind.half_width(i as f32 / 100.0))
                .fold(0.0, f32::max);
            assert!((0.49..=0.5001).contains(&widest), "{kind:?}: {widest}");
        }
    }

    #[test]
    fn birch_leaves_are_widest_nearer_their_stalk_than_broadleaves() {
        let widest = |kind: LeafKind| {
            (1..100)
                .map(|i| i as f32 / 100.0)
                .max_by(|&a, &b| kind.half_width(a).total_cmp(&kind.half_width(b)))
                .unwrap()
        };
        assert!(widest(LeafKind::Birch) < widest(LeafKind::Broadleaf));
    }

    #[test]
    fn leaves_are_as_dark_as_real_leaves() {
        for kind in LeafKind::ALL {
            let layer = kind.atlas(0).bake(64);
            let top = layer.color_mips.last().unwrap();
            // Real leaves reflect about 0.05 to 0.2 in green. A wattle's gold blossom, a
            // quarter of its atlas, lifts its mean.
            let green = (top[1] as f32 / 255.0).powf(2.2);
            let most = if kind == LeafKind::Wattle { 0.3 } else { 0.22 };
            assert!(
                (0.03..most).contains(&green),
                "{kind:?}: green albedo {green}"
            );
        }
    }

    #[test]
    fn kinds_are_found_by_name() {
        for kind in LeafKind::ALL {
            assert_eq!(LeafKind::by_name(kind.name()), Some(kind));
        }
    }
}
