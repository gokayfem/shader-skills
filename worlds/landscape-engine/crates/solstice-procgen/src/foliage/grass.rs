//! Grass: tufts of curved, folded, tapering blades, and the atlas that colours them.
//!
//! A real blade is not a flat strip. It is folded along its midrib into a shallow V, which catches
//! light differently on each half, it tapers to a point, and it arcs over under its own weight
//! towards the face it is folded around. Grass also grows in tufts, with blades fanning outward
//! from a crown, rather than as an even scatter, and that clumping is much of what makes a field
//! read as grass rather than as green noise.
//!
//! A breed of grass is plain data with a text form, like a tree [`Species`](super::species):
//!
//! ```text
//! # A mown lawn.
//! size 1
//! tufts per_m2=200 blades=4..8 height=0.05..0.09 width=0.003..0.005 lean=0.05..0.4
//! undergrowth per_m2=900 height=0.02..0.05
//! variants 0.5,0.3,0.2,0
//! dry 0.02
//! colors 0.05,0.11,0.02 0.08,0.13,0.022 0.045,0.1,0.035 0.2,0.17,0.08
//! weed kind=clover per_m2=12 length=0.01..0.015 width=0.9 flower=0.6,0.6,0.55
//! ```
//!
//! Each `weed` line adds a kind of [`Weed`] among the grass, such as dandelions or clover.
//!
//! Every setting has a default, so a file names only what differs. [`Meadow::parse`] reads this
//! form and [`Meadow::to_text`] writes it back. The built-in breeds are files of exactly this
//! kind, in `grass/` beside this crate.

use super::text::{color, color_text, lines, number, pairs, range, range_text, single};
use super::weed::{flower_texel, leaf_texel, Weed};
use super::Rng;
use crate::field::smoothstep;
use crate::noise::tiling_fbm;
use crate::surface::{Surface, Texel};
use glam::{Vec2, Vec3};
use solstice_builder::{Mesh, Vertex};
use std::f32::consts::TAU;

/// Colour variants in the blade atlas, side by side. The last is dry.
pub const VARIANTS: u32 = 4;
/// The atlas variant of dead, dry blades.
pub const DRY: u32 = 3;

/// Heights along a blade, as fractions of its length, at which it has a full cross-section. The
/// tip is a single point beyond the last.
const ROWS: [f32; 3] = [0.0, 0.4, 0.75];
/// How far below the ground blades start, so they never float where the ground is uneven.
const ROOT_DEPTH: f32 = 0.03;

/// The built-in breeds, by name, as text.
const BUILT_IN: [(&str, &str); 5] = [
    ("meadow", include_str!("../../grass/meadow.grass")),
    ("lawn", include_str!("../../grass/lawn.grass")),
    ("dry", include_str!("../../grass/dry.grass")),
    ("tall", include_str!("../../grass/tall.grass")),
    ("verge", include_str!("../../grass/verge.grass")),
];

/// A square patch of one breed of grass, which lays edge to edge with copies of itself.
#[derive(Clone, Debug, PartialEq)]
pub struct Meadow {
    /// Edge of the patch, in metres.
    pub size: f32,
    pub tufts_per_square_metre: f32,
    pub blades_per_tuft: (u32, u32),
    /// Range of tuft heights, in metres. Blades vary around their tuft's height.
    pub height: (f32, f32),
    /// Range of blade widths at the base, in metres.
    pub width: (f32, f32),
    /// How far a tuft's blades arc over, as a fraction of their length at the tip: the first for
    /// blades at its centre, the second for those at its edge.
    pub lean: (f32, f32),
    /// How deep the fold along each blade's midrib is, relative to its width.
    pub fold: f32,
    /// How far blade normals lean towards straight up. Grass in Unreal is usually lit this way, as
    /// light through a whole field rather than off each blade, which keeps a meadow from turning to
    /// glitter where blades face away from the sun.
    pub normals_up: f32,
    /// How likely a tuft is to be each variant, before dry blades are added.
    pub variants: [f32; VARIANTS as usize],
    /// Fraction of blades that are dead and dry, in any tuft.
    pub dry: f32,
    /// Short single blades between the tufts, per square metre, which hide the soil as the thatch
    /// and young shoots of a real meadow do.
    pub undergrowth_per_square_metre: f32,
    /// Range of undergrowth heights, in metres.
    pub undergrowth_height: (f32, f32),
    /// Linear albedo at mid-blade of each atlas variant, the last being dry.
    pub colors: [Vec3; VARIANTS as usize],
    /// Weeds and wild flowers among the grass.
    pub weeds: Vec<Weed>,
    /// Which patch of this breed: different seeds lay out different blades.
    pub seed: u32,
}

impl Default for Meadow {
    /// An unmown summer meadow.
    fn default() -> Self {
        Self::by_name("meadow").unwrap()
    }
}

impl Meadow {
    /// Names of the built-in breeds.
    pub fn names() -> impl Iterator<Item = &'static str> {
        BUILT_IN.iter().map(|(name, _)| *name)
    }

    /// A built-in breed, or `None` if there is none of that name.
    pub fn by_name(name: &str) -> Option<Self> {
        let (_, text) = BUILT_IN.iter().find(|(n, _)| *n == name)?;
        Some(Self::parse(text).unwrap_or_else(|e| panic!("built-in grass {name}: {e}")))
    }

    /// The settings a breed file starts from: an unmown meadow.
    fn base() -> Self {
        Self {
            size: 2.0,
            tufts_per_square_metre: 50.0,
            blades_per_tuft: (8, 20),
            height: (0.22, 0.55),
            width: (0.005, 0.009),
            lean: (0.12, 0.82),
            fold: 0.18,
            normals_up: 0.3,
            variants: [0.45, 0.3, 0.25, 0.0],
            dry: 0.08,
            undergrowth_per_square_metre: 450.0,
            undergrowth_height: (0.05, 0.16),
            colors: [
                Vec3::new(0.050, 0.105, 0.018),
                Vec3::new(0.085, 0.130, 0.022),
                Vec3::new(0.042, 0.090, 0.034),
                Vec3::new(0.230, 0.180, 0.080),
            ],
            weeds: Vec::new(),
            seed: 0,
        }
    }

    /// Reads a breed from its text form. Errors name the line at fault.
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut meadow = Self::base();
        for (line, words) in lines(text) {
            let at = |e: String| format!("line {line}: {e}");
            let (keyword, rest) = (words[0], &words[1..]);
            match keyword {
                "size" => meadow.size = single(rest).map_err(at)?,
                "dry" => meadow.dry = single(rest).map_err(at)?,
                "fold" => meadow.fold = single(rest).map_err(at)?,
                "normals_up" => meadow.normals_up = single(rest).map_err(at)?,
                "tufts" => meadow.read_tufts(rest).map_err(at)?,
                "weed" => meadow.weeds.push(Weed::parse(rest).map_err(at)?),
                "undergrowth" => {
                    for (key, value) in pairs(rest).map_err(at)? {
                        match key {
                            "per_m2" => {
                                meadow.undergrowth_per_square_metre = number(value).map_err(at)?
                            }
                            "height" => meadow.undergrowth_height = range(value).map_err(at)?,
                            _ => return Err(at(format!("unknown undergrowth setting {key}"))),
                        }
                    }
                }
                "variants" => {
                    let [weights] = rest else {
                        return Err(at("variants takes weights separated by commas".into()));
                    };
                    let values: Vec<f32> = weights
                        .split(',')
                        .map(number)
                        .collect::<Result<_, _>>()
                        .map_err(at)?;
                    meadow.variants = values
                        .try_into()
                        .map_err(|_| at(format!("variants needs {VARIANTS} weights")))?;
                }
                "colors" => {
                    let values: Vec<Vec3> = rest
                        .iter()
                        .map(|c| color(c))
                        .collect::<Result<_, _>>()
                        .map_err(at)?;
                    meadow.colors = values
                        .try_into()
                        .map_err(|_| at(format!("colors needs {VARIANTS} colours")))?;
                }
                _ => return Err(at(format!("unknown setting {keyword:?}"))),
            }
        }
        if meadow.size <= 0.0 {
            return Err("size must be more than 0".into());
        }
        Ok(meadow)
    }

    fn read_tufts(&mut self, words: &[&str]) -> Result<(), String> {
        for (key, value) in pairs(words)? {
            match key {
                "per_m2" => self.tufts_per_square_metre = number(value)?,
                "blades" => {
                    let (a, b) = range(value)?;
                    if a < 1.0 || b < a {
                        return Err("blades must be a rising range from 1".into());
                    }
                    self.blades_per_tuft = (a as u32, b as u32);
                }
                "height" => self.height = range(value)?,
                "width" => self.width = range(value)?,
                "lean" => self.lean = range(value)?,
                _ => return Err(format!("unknown tufts setting {key}")),
            }
        }
        Ok(())
    }

    /// Writes this breed in the form [`Meadow::parse`] reads. The seed is not part of a breed.
    pub fn to_text(&self) -> String {
        let v = self.variants;
        let colors: Vec<String> = self.colors.iter().map(|&c| color_text(c)).collect();
        let mut text = format!("size {}\n", self.size);
        text += &format!(
            "tufts per_m2={} blades={}..{} height={} width={} lean={}\n",
            self.tufts_per_square_metre,
            self.blades_per_tuft.0,
            self.blades_per_tuft.1,
            range_text(self.height),
            range_text(self.width),
            range_text(self.lean)
        );
        text += &format!(
            "undergrowth per_m2={} height={}\n",
            self.undergrowth_per_square_metre,
            range_text(self.undergrowth_height)
        );
        text += &format!("fold {}\nnormals_up {}\n", self.fold, self.normals_up);
        text += &format!(
            "variants {},{},{},{}\ndry {}\n",
            v[0], v[1], v[2], v[3], self.dry
        );
        text += &format!("colors {}\n", colors.join(" "));
        for weed in &self.weeds {
            text += &format!("weed {}\n", weed.to_text());
        }
        text
    }

    /// Columns in this breed's atlas: [`VARIANTS`] of blades, then a leaf and a flower for each
    /// weed.
    pub fn columns(&self) -> u32 {
        VARIANTS + 2 * self.weeds.len() as u32
    }

    /// The atlas for this breed: its blades in its colours, then its weeds' leaves and flowers.
    pub fn atlas(&self, seed: u32) -> Surface {
        let colors = self.colors;
        let weeds = self.weeds.clone();
        let columns = self.columns();
        Surface::new(0.0006, move |u, v| {
            let column = ((u * columns as f32) as u32).min(columns - 1);
            let across = (u * columns as f32).fract();
            if column < VARIANTS {
                return blade_texel(seed, colors, column, across, v);
            }
            let weed = &weeds[((column - VARIANTS) / 2) as usize];
            if (column - VARIANTS).is_multiple_of(2) {
                leaf_texel((across - 0.5) / 0.45, v, weed.leaf_color, seed + column)
            } else {
                flower_texel(
                    (across - 0.5) / 0.47,
                    v,
                    weed.flower_color,
                    weed.centre_color,
                )
            }
        })
    }

    /// The patch's blades, with the ground at Y = 0 and the patch spanning `0..size` in X and Z.
    pub fn mesh(&self) -> Mesh {
        self.mesh_where(&|_, _| 1.0)
    }

    /// The patch's blades where `vigour`, at a spot's X and Z, is above zero, and shorter where
    /// it is low: grass thinning out and dwindling towards the edge of where it grows, so that
    /// patches of other shapes than a square can be cut from the same breed.
    pub fn mesh_where(&self, vigour: &dyn Fn(f32, f32) -> f32) -> Mesh {
        let mut rng = Rng::new(self.seed, 1);
        let mut mesh = Mesh::default();
        let tufts = (self.size * self.size * self.tufts_per_square_metre).round() as u32;
        let total_weight: f32 = self.variants.iter().sum();
        for _ in 0..tufts {
            let center = Vec3::new(rng.range(0.0, self.size), 0.0, rng.range(0.0, self.size));
            let tuft_height = rng.range(self.height.0, self.height.1);
            let tuft_width = rng.range(self.width.0, self.width.1);
            let mut pick = rng.unit() * total_weight;
            let mut variant = 0;
            while variant + 1 < VARIANTS as usize && pick >= self.variants[variant] {
                pick -= self.variants[variant];
                variant += 1;
            }
            let count = self.blades_per_tuft.0
                + rng.index((self.blades_per_tuft.1 - self.blades_per_tuft.0 + 1) as usize) as u32;
            let strength = vigour(center.x, center.z).min(1.0);
            let (count, tuft_height) = if strength > 0.0 {
                (count, tuft_height * (0.45 + 0.55 * strength))
            } else {
                (0, tuft_height)
            };
            for _ in 0..count {
                // Blades rise from a crown a few centimetres across and fan outward from it,
                // the outer ones leaning furthest.
                let angle = rng.range(0.0, TAU);
                let reach = rng.unit().sqrt();
                let outward = Vec3::new(angle.cos(), 0.0, angle.sin());
                let root = center + outward * reach * 0.035 - Vec3::Y * ROOT_DEPTH;
                let swing = rng.signed() * 0.7;
                let bend = Vec3::new((angle + swing).cos(), 0.0, (angle + swing).sin());
                let lean = (self.lean.0
                    + (self.lean.1 - self.lean.0) * (reach * 0.6 + rng.unit() * 0.4))
                    .min(0.9);
                let blade_variant = if rng.unit() < self.dry {
                    DRY
                } else {
                    variant as u32
                };
                self.add_blade(
                    &mut mesh,
                    root,
                    bend,
                    tuft_height * rng.range(0.65, 1.15),
                    tuft_width * rng.range(0.8, 1.2),
                    lean,
                    blade_variant,
                );
            }
        }
        let columns = self.columns() as f32;
        for (index, weed) in self.weeds.iter().enumerate() {
            let leaf_column = (VARIANTS + 2 * index as u32) as f32;
            let leaf_u = |across: f32| (leaf_column + 0.5 + across * 0.45) / columns;
            let flower_u = |across: f32| (leaf_column + 1.5 + across * 0.47) / columns;
            let plants = (self.size * self.size * weed.per_square_metre).round() as u32;
            for _ in 0..plants {
                let root = Vec3::new(rng.range(0.0, self.size), 0.0, rng.range(0.0, self.size));
                if vigour(root.x, root.z) <= 0.0 {
                    continue;
                }
                weed.add_plant(
                    &mut mesh,
                    &mut rng,
                    root,
                    &leaf_u,
                    &flower_u,
                    self.normals_up,
                );
            }
        }
        let undergrowth =
            (self.size * self.size * self.undergrowth_per_square_metre).round() as u32;
        for _ in 0..undergrowth {
            let root = Vec3::new(
                rng.range(0.0, self.size),
                -ROOT_DEPTH,
                rng.range(0.0, self.size),
            );
            let angle = rng.range(0.0, TAU);
            let variant = if rng.unit() < self.dry * 2.0 {
                DRY
            } else {
                rng.index(DRY as usize) as u32
            };
            let height = rng.range(self.undergrowth_height.0, self.undergrowth_height.1);
            let width = rng.range(self.width.0, self.width.1) * 0.8;
            let lean = rng.range(0.1, 0.7);
            // Short shoots reach a little past the tufts, so the edge frays rather than stops.
            if vigour(root.x, root.z) <= -0.25 {
                continue;
            }
            self.add_blade(
                &mut mesh,
                root,
                Vec3::new(angle.cos(), 0.0, angle.sin()),
                height,
                width,
                lean,
                variant,
            );
        }
        mesh
    }

    /// One blade, arcing over towards `bend` by `lean` of its height at the tip.
    #[allow(clippy::too_many_arguments)]
    fn add_blade(
        &self,
        mesh: &mut Mesh,
        root: Vec3,
        bend: Vec3,
        length: f32,
        width: f32,
        lean: f32,
        variant: u32,
    ) {
        // A quadratic curve, rising straight from the crown and arcing over at the top, sized so
        // the tip is `lean` of the blade's length out and the blade keeps roughly its length.
        let tip =
            root + Vec3::Y * length * (1.0 - lean * lean).sqrt() * 0.97 + bend * length * lean;
        let control = root + Vec3::Y * length * 0.6;
        let point =
            |t: f32| root * (1.0 - t) * (1.0 - t) + control * 2.0 * t * (1.0 - t) + tip * t * t;
        let direction = |t: f32| ((control - root) * (1.0 - t) + (tip - control) * t).normalize();

        // The blade's flat side faces the way it bends, as real blades fold around their upper
        // face.
        let side = Vec3::Y.cross(bend).normalize();
        let columns = self.columns() as f32;
        let u = |across: f32| (variant as f32 + 0.5 + across * 0.45) / columns;
        let lit = |n: Vec3| (n + Vec3::Y * self.normals_up).normalize();

        let base = mesh.vertices.len() as u32;
        for &t in &ROWS {
            let center = point(t);
            let along = direction(t);
            let face = side.cross(along).normalize();
            let half = width * 0.5 * (1.0 - t.powf(2.2));
            let fold = face * (self.fold * half * 2.0);
            // Each half of the V leans in towards the midrib.
            let tilt = side * (self.fold * 2.0);
            mesh.vertices.push(Vertex {
                position: center - side * half,
                normal: lit((face + tilt).normalize()),
                uv: Vec2::new(u(-1.0), t),
            });
            mesh.vertices.push(Vertex {
                position: center - fold,
                normal: lit(face),
                uv: Vec2::new(u(0.0), t),
            });
            mesh.vertices.push(Vertex {
                position: center + side * half,
                normal: lit((face - tilt).normalize()),
                uv: Vec2::new(u(1.0), t),
            });
        }
        let tip_face = side.cross(direction(1.0)).normalize();
        mesh.vertices.push(Vertex {
            position: tip,
            normal: lit(tip_face),
            uv: Vec2::new(u(0.0), 1.0),
        });

        // Counter-clockwise seen from the face.
        for row in 0..ROWS.len() as u32 - 1 {
            let (l0, c0, r0) = (base + row * 3, base + row * 3 + 1, base + row * 3 + 2);
            let (l1, c1, r1) = (l0 + 3, c0 + 3, r0 + 3);
            mesh.indices
                .extend_from_slice(&[l0, c0, c1, l0, c1, l1, c0, r0, r1, c0, r1, c1]);
        }
        let last = base + (ROWS.len() as u32 - 1) * 3;
        let tip_index = last + 3;
        mesh.indices
            .extend_from_slice(&[last, last + 1, tip_index, last + 1, last + 2, tip_index]);
    }
}

/// A blade of the atlas column `variant`, at `across` (0..1) and `v` from root (0) to tip (1),
/// with the given linear albedo at mid-blade. Living grass reflects little: about 0.1 in green.
fn blade_texel(
    seed: u32,
    colors: [Vec3; VARIANTS as usize],
    variant: u32,
    across: f32,
    v: f32,
) -> Texel {
    const SHEATH: Vec3 = Vec3::new(0.11, 0.12, 0.05);
    const WITHERED: Vec3 = Vec3::new(0.20, 0.15, 0.07);
    let base = colors[variant as usize];

    // Parallel veins, and a paler midrib.
    let veins = 0.5 + 0.5 * (across * 9.0 * TAU).cos();
    let midrib = (-((across - 0.5) * 14.0).powi(2)).exp();
    // Slow streaks along the blade.
    let streaks = tiling_fbm(across * 0.25 + variant as f32 * 0.25, v, 8, 3, seed);

    let mut color = base * (0.9 + 0.12 * veins + 0.25 * midrib) * (1.0 + 0.35 * streaks);
    // The pale sheath at the base, and a withered tip on living blades.
    color = color.lerp(SHEATH, (1.0 - smoothstep(0.0, 0.14, v)) * 0.7);
    if variant != DRY {
        let wither = smoothstep(0.82, 1.0, v) * (0.5 + 0.5 * tiling_fbm(across, v, 4, 2, seed + 7));
        color = color.lerp(WITHERED, wither.clamp(0.0, 1.0) * 0.8);
    }
    Texel {
        color,
        height: veins * 0.4 + midrib,
        // Fresh blades have a waxy sheen. Dry ones are matte.
        roughness: if variant == DRY { 0.85 } else { 0.6 },
        // Lower blades sit in the shade of the whole tuft.
        cavity: 0.35 + 0.65 * smoothstep(0.0, 0.55, v),
    }
}

/// The angle between two directions, for tests.
#[cfg(test)]
fn angle_between(a: Vec3, b: Vec3) -> f32 {
    a.normalize()
        .dot(b.normalize())
        .clamp(-1.0, 1.0)
        .acos()
        .to_degrees()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_meadow_has_the_blades_it_asks_for_inside_its_patch() {
        let meadow = Meadow {
            weeds: Vec::new(),
            ..Meadow::default()
        };
        let mesh = meadow.mesh();
        let blades = mesh.vertices.len() / (ROWS.len() * 3 + 1);
        let per_square_metre = blades as f32 / (meadow.size * meadow.size);
        // About 50 tufts of 8 to 20 blades, and 450 short ones between.
        assert!(
            (1000.0..1400.0).contains(&per_square_metre),
            "{per_square_metre} blades per m²"
        );
        for vertex in &mesh.vertices {
            let p = vertex.position;
            assert!(p.x > -0.5 && p.x < meadow.size + 0.5 && p.z > -0.5 && p.z < meadow.size + 0.5);
            assert!(p.y >= -ROOT_DEPTH - 1.0e-4 && p.y < meadow.height.1 * 1.2);
            assert!((vertex.normal.length() - 1.0).abs() < 1.0e-3);
        }
    }

    #[test]
    fn blades_face_the_way_their_triangles_wind() {
        for name in Meadow::names() {
            let mesh = Meadow {
                weeds: Vec::new(),
                ..Meadow::by_name(name).unwrap()
            }
            .mesh();
            for triangle in mesh.indices.chunks_exact(3) {
                let [a, b, c] = [0, 1, 2].map(|i| mesh.vertices[triangle[i] as usize]);
                let face = (b.position - a.position).cross(c.position - a.position);
                if face.length() < 1.0e-9 {
                    continue;
                }
                let normal = a.normal + b.normal + c.normal;
                assert!(
                    angle_between(face, normal) < 80.0,
                    "{name}: a triangle faces {} degrees from its normals",
                    angle_between(face, normal)
                );
            }
        }
    }

    #[test]
    fn the_same_seed_grows_the_same_meadow() {
        let a = Meadow::default().mesh();
        let b = Meadow::default().mesh();
        let c = Meadow {
            seed: 1,
            ..Default::default()
        }
        .mesh();
        assert_eq!(a.vertices, b.vertices);
        assert_ne!(a.vertices, c.vertices);
    }

    #[test]
    fn living_grass_is_as_dark_as_real_grass() {
        for name in Meadow::names() {
            let layer = Meadow::by_name(name).unwrap().atlas(0).bake(64);
            let top = layer.color_mips.last().unwrap();
            // The whole atlas averaged, in sRGB. Real grass is about 0.1 linear albedo in green.
            let green = (top[1] as f32 / 255.0).powf(2.2);
            assert!(
                (0.06..0.25).contains(&green),
                "{name}: green albedo {green}"
            );
        }
    }

    #[test]
    fn every_built_in_breed_reads_and_writes_back_unchanged() {
        for name in Meadow::names() {
            let meadow = Meadow::by_name(name).unwrap();
            assert_eq!(Meadow::parse(&meadow.to_text()), Ok(meadow), "{name}");
        }
        assert!(Meadow::by_name("bamboo").is_none());
    }

    #[test]
    fn breeds_differ_as_their_names_say() {
        let height = |name: &str| {
            let mesh = Meadow::by_name(name).unwrap().mesh();
            let mut tops: Vec<f32> = mesh.vertices.iter().map(|v| v.position.y).collect();
            tops.sort_by(f32::total_cmp);
            tops[tops.len() * 95 / 100]
        };
        assert!(height("lawn") < 0.12, "lawn {}", height("lawn"));
        assert!(height("tall") > height("meadow"));
        let dry = Meadow::by_name("dry").unwrap();
        assert!(dry.dry > Meadow::default().dry);
    }

    #[test]
    fn mistakes_are_reported_with_their_line() {
        let error = |text: &str| Meadow::parse(text).unwrap_err();
        assert!(error("size 2\ntufts hieght=1\n").starts_with("line 2:"));
        assert!(error("colors 1,1\n").contains("colour"));
        assert!(error("tufts blades=9..3\n").contains("blades"));
        assert!(error("size 0\n").contains("size"));
    }
}
