//! Weeds and wild flowers among grass: low broadleaf plants, many in flower.
//!
//! A weed is described by one line of a grass file, so a breed of grass can carry any mix:
//!
//! ```text
//! weed kind=rosette per_m2=4 leaves=6..10 length=0.06..0.14 width=0.28 flowers=0.5 flower_height=0.1..0.25 flower_size=0.018 leaf=0.045,0.09,0.02 flower=0.55,0.4,0.01
//! weed kind=clover per_m2=10 leaves=4..9 length=0.012..0.018 width=0.9 stalk=0.03..0.07 flowers=0.3 flower_height=0.06..0.1 flower_size=0.009 leaf=0.035,0.08,0.025 flower=0.6,0.6,0.55
//! ```
//!
//! A rosette is a ring of leaves flat to the ground, as on a dandelion, a daisy or a plantain.
//! Clover carries leaves of three round leaflets on short stalks. Either may flower: a stalk
//! with a head on top, domed for clover, flatter for a dandelion or daisy, whose centre can be
//! another colour.
//!
//! Each weed takes two columns of its breed's atlas, one for its leaves and one for its flowers.

use super::text::{color, color_text, number, pairs, range, range_text};
use super::Rng;
use crate::field::smoothstep;
use crate::noise::tiling_fbm;
use crate::surface::Texel;
use glam::{Quat, Vec2, Vec3};
use solstice_builder::{Mesh, Vertex};
use std::f32::consts::TAU;

/// How a weed's leaves grow.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WeedKind {
    /// A ring of leaves lying nearly flat, as on a dandelion, a daisy or a plantain.
    Rosette,
    /// Leaves of three round leaflets, each on its own short stalk.
    Clover,
}

impl WeedKind {
    fn name(self) -> &'static str {
        match self {
            Self::Rosette => "rosette",
            Self::Clover => "clover",
        }
    }
}

/// One kind of weed in a breed of grass.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Weed {
    pub kind: WeedKind,
    /// Plants per square metre.
    pub per_square_metre: f32,
    /// Leaves on each plant.
    pub leaves: (u32, u32),
    /// Length of each leaf, or for clover of each leaflet, in metres.
    pub length: (f32, f32),
    /// Width of a leaf as a fraction of its length.
    pub width: f32,
    /// Height of clover's leaf stalks, in metres. Rosettes have none.
    pub stalk: (f32, f32),
    /// The chance that a plant is in flower.
    pub flowers: f32,
    /// Height of a flower's head, in metres.
    pub flower_height: (f32, f32),
    /// Radius of a flower's head, in metres.
    pub flower_size: f32,
    /// Linear albedo of the leaves, the petals and the middle of the flower.
    pub leaf_color: Vec3,
    pub flower_color: Vec3,
    pub centre_color: Vec3,
}

impl Weed {
    /// Reads the `key=value` words of a `weed` line.
    pub fn parse(words: &[&str]) -> Result<Self, String> {
        let mut weed = Self {
            kind: WeedKind::Rosette,
            per_square_metre: 4.0,
            leaves: (6, 10),
            length: (0.06, 0.14),
            width: 0.3,
            stalk: (0.03, 0.07),
            flowers: 0.4,
            flower_height: (0.1, 0.25),
            flower_size: 0.015,
            leaf_color: Vec3::new(0.045, 0.09, 0.02),
            flower_color: Vec3::new(0.55, 0.4, 0.01),
            centre_color: Vec3::NAN,
        };
        for (key, value) in pairs(words)? {
            match key {
                "kind" => {
                    weed.kind = match value {
                        "rosette" => WeedKind::Rosette,
                        "clover" => WeedKind::Clover,
                        _ => return Err(format!("unknown weed kind {value:?}")),
                    }
                }
                "per_m2" => weed.per_square_metre = number(value)?,
                "leaves" => {
                    let (a, b) = range(value)?;
                    if a < 1.0 || b < a {
                        return Err("leaves must be a rising range from 1".into());
                    }
                    weed.leaves = (a as u32, b as u32);
                }
                "length" => weed.length = range(value)?,
                "width" => weed.width = number(value)?,
                "stalk" => weed.stalk = range(value)?,
                "flowers" => weed.flowers = number(value)?,
                "flower_height" => weed.flower_height = range(value)?,
                "flower_size" => weed.flower_size = number(value)?,
                "leaf" => weed.leaf_color = color(value)?,
                "flower" => weed.flower_color = color(value)?,
                "centre" => weed.centre_color = color(value)?,
                _ => return Err(format!("unknown weed setting {key}")),
            }
        }
        // Unless given, a flower is one colour throughout, a little deeper in the middle.
        if weed.centre_color.is_nan() {
            weed.centre_color = weed.flower_color * 0.8;
        }
        Ok(weed)
    }

    /// Writes the line [`Weed::parse`] reads, after its `weed` keyword.
    pub fn to_text(&self) -> String {
        format!(
            "kind={} per_m2={} leaves={}..{} length={} width={} stalk={} flowers={} flower_height={} flower_size={} leaf={} flower={} centre={}",
            self.kind.name(),
            self.per_square_metre,
            self.leaves.0,
            self.leaves.1,
            range_text(self.length),
            self.width,
            range_text(self.stalk),
            self.flowers,
            range_text(self.flower_height),
            self.flower_size,
            color_text(self.leaf_color),
            color_text(self.flower_color),
            color_text(self.centre_color)
        )
    }

    /// Adds one plant rooted at `root`. `leaf_u` and `flower_u` map a position across the leaf
    /// or flower (-1..1) to the atlas.
    pub(crate) fn add_plant(
        &self,
        mesh: &mut Mesh,
        rng: &mut Rng,
        root: Vec3,
        leaf_u: &dyn Fn(f32) -> f32,
        flower_u: &dyn Fn(f32) -> f32,
        normals_up: f32,
    ) {
        let lit = |n: Vec3| (n + Vec3::Y * normals_up).normalize();
        let count = self.leaves.0 + rng.index((self.leaves.1 - self.leaves.0 + 1) as usize) as u32;
        let turn = rng.range(0.0, TAU);
        for k in 0..count {
            let angle = turn + (k as f32 + rng.signed() * 0.3) / count as f32 * TAU;
            let out = Vec3::new(angle.cos(), 0.0, angle.sin());
            let length = rng.range(self.length.0, self.length.1);
            match self.kind {
                WeedKind::Rosette => {
                    // Lying nearly flat, the newer inner leaves rising a little more.
                    let rise = rng.range(0.1, 0.45);
                    let axis = (out + Vec3::Y * rise).normalize();
                    add_leaf(
                        mesh,
                        root,
                        axis,
                        length,
                        self.width,
                        (0.9, 0.55),
                        leaf_u,
                        &lit,
                    );
                }
                WeedKind::Clover => {
                    let height = rng.range(self.stalk.0, self.stalk.1);
                    let lean = out * height * rng.range(0.1, 0.4);
                    let top = root + Vec3::Y * height + lean;
                    add_stalk(mesh, root, top, 0.0012, leaf_u, &lit);
                    // Three round leaflets, spread flat around the top of the stalk.
                    for leaflet in 0..3 {
                        let a = angle + leaflet as f32 / 3.0 * TAU;
                        let axis = (Vec3::new(a.cos(), 0.0, a.sin()) + Vec3::Y * 0.1).normalize();
                        add_leaf(
                            mesh,
                            top,
                            axis,
                            length,
                            self.width,
                            (0.5, 0.35),
                            leaf_u,
                            &lit,
                        );
                    }
                }
            }
        }
        if rng.unit() < self.flowers {
            let height = rng.range(self.flower_height.0, self.flower_height.1);
            let lean = Vec3::new(rng.signed(), 0.0, rng.signed()) * height * 0.2;
            let top = root + Vec3::Y * height + lean;
            add_stalk(mesh, root, top, 0.0015, leaf_u, &lit);
            // Clover heads are round; dandelions and daisies are flatter.
            let dome = match self.kind {
                WeedKind::Clover => 0.9,
                WeedKind::Rosette => 0.25,
            };
            let facing = (Vec3::Y + lean.normalize_or_zero() * 0.4).normalize();
            add_flower_head(mesh, top, facing, self.flower_size, dome, flower_u);
        }
    }
}

/// A flat leaf from `base` along `axis`, widest where `s^a (1 - s)^b` peaks, as two rows of
/// three vertices between a stalk point and a tip.
#[allow(clippy::too_many_arguments)]
fn add_leaf(
    mesh: &mut Mesh,
    base: Vec3,
    axis: Vec3,
    length: f32,
    width_ratio: f32,
    (a, b): (f32, f32),
    u: &dyn Fn(f32) -> f32,
    lit: &dyn Fn(Vec3) -> Vec3,
) {
    // The face looks as nearly straight up as the leaf allows.
    let face = (Vec3::Y - axis * axis.dot(Vec3::Y)).normalize_or(Vec3::Y);
    let side = axis.cross(face).normalize();
    let peak = a / (a + b);
    let max = peak.powf(a) * (1.0 - peak).powf(b);
    let half = |s: f32| length * width_ratio * 0.5 * s.powf(a) * (1.0 - s).powf(b) / max;
    // Leaves curve down towards their tips.
    let curl = |s: f32| -face * (s * s * length * 0.15);
    let first = mesh.vertices.len() as u32;
    mesh.vertices.push(Vertex {
        position: base,
        normal: lit(face),
        uv: Vec2::new(u(0.0), 0.0),
    });
    for s in [0.35, 0.7] {
        let centre = base + axis * (s * length) + curl(s);
        let w = half(s);
        for across in [-1.0, 0.0, 1.0] {
            let position =
                centre + side * (across * w) - face * (w * 0.2 * (1.0 - across * across));
            mesh.vertices.push(Vertex {
                position,
                normal: lit((face - side * (across * 0.3)).normalize()),
                uv: Vec2::new(u(across), s),
            });
        }
    }
    mesh.vertices.push(Vertex {
        position: base + axis * length + curl(1.0),
        normal: lit(face),
        uv: Vec2::new(u(0.0), 1.0),
    });
    let (stalk, l0, c0, r0) = (first, first + 1, first + 2, first + 3);
    let (l1, c1, r1, tip) = (first + 4, first + 5, first + 6, first + 7);
    // Counter-clockwise seen from the face.
    mesh.indices.extend_from_slice(&[
        stalk, c0, l0, stalk, r0, c0, //
        l0, c0, c1, l0, c1, l1, c0, r0, r1, c0, r1, c1, //
        l1, c1, tip, c1, r1, tip,
    ]);
}

/// A thin stalk from `base` to `top`: a narrow strip, which is lit from either side.
fn add_stalk(
    mesh: &mut Mesh,
    base: Vec3,
    top: Vec3,
    half_width: f32,
    u: &dyn Fn(f32) -> f32,
    lit: &dyn Fn(Vec3) -> Vec3,
) {
    let along = (top - base).normalize_or(Vec3::Y);
    let side = along.cross(Vec3::X).normalize_or(Vec3::Z) * half_width;
    let face = side.cross(along).normalize();
    let first = mesh.vertices.len() as u32;
    for (point, v) in [(base, 0.0), (top, 1.0)] {
        for (offset, across) in [(-side, -0.2), (side, 0.2)] {
            mesh.vertices.push(Vertex {
                position: point + offset,
                normal: lit(face),
                uv: Vec2::new(u(across), v),
            });
        }
    }
    mesh.indices
        .extend_from_slice(&[first, first + 1, first + 3, first, first + 3, first + 2]);
}

/// Petals around a head on top of a stalk: a fan of triangles from a raised centre.
fn add_flower_head(
    mesh: &mut Mesh,
    centre: Vec3,
    facing: Vec3,
    radius: f32,
    dome: f32,
    u: &dyn Fn(f32) -> f32,
) {
    const SEGMENTS: u32 = 10;
    let rotation = Quat::from_rotation_arc(Vec3::Y, facing);
    let first = mesh.vertices.len() as u32;
    mesh.vertices.push(Vertex {
        position: centre + facing * (radius * dome),
        normal: facing,
        uv: Vec2::new(u(0.0), 0.0),
    });
    for i in 0..=SEGMENTS {
        let a = i as f32 / SEGMENTS as f32 * TAU;
        let out = rotation * Vec3::new(a.cos(), 0.0, a.sin());
        mesh.vertices.push(Vertex {
            position: centre + out * radius,
            normal: (facing + out * dome).normalize(),
            uv: Vec2::new(u(i as f32 / SEGMENTS as f32 * 2.0 - 1.0), 1.0),
        });
    }
    for i in 0..SEGMENTS {
        // Counter-clockwise seen from above.
        mesh.indices
            .extend_from_slice(&[first, first + 2 + i, first + 1 + i]);
    }
}

/// A leaf's colour at `across` (-1..1) and `v` from stalk to tip: a pale midrib, side veins and
/// mottling.
pub(crate) fn leaf_texel(across: f32, v: f32, color: Vec3, seed: u32) -> Texel {
    let d = across.abs();
    let phase = v * 6.0 - d * 2.0;
    let side_veins = (-((phase - phase.round()) * 8.0).powi(2)).exp() * (1.0 - d);
    let midrib = (-(across * 12.0).powi(2)).exp();
    let veins = side_veins.max(midrib);
    let mottle = tiling_fbm(across * 0.5 + 0.5, v, 6, 3, seed);
    let mut c = color * (1.0 + 0.2 * mottle);
    c = c.lerp(c * Vec3::new(1.5, 1.45, 1.2), veins * 0.45);
    Texel {
        color: c,
        height: -veins,
        roughness: 0.55,
        cavity: 0.75 + 0.25 * v,
    }
}

/// A flower head's colour at angle `across` (-1..1 round it) and `v` from its centre (0) to its
/// rim (1): petals radiating from a centre of another colour.
pub(crate) fn flower_texel(across: f32, v: f32, petals: Vec3, centre: Vec3) -> Texel {
    // Many narrow petals, with darker gaps between them towards the rim.
    let petal = 0.5 + 0.5 * (across * 11.0 * TAU).cos();
    let gap = smoothstep(0.55, 1.0, v) * smoothstep(0.35, 0.0, petal);
    let disc = 1.0 - smoothstep(0.25, 0.4, v);
    let color = petals.lerp(centre, disc) * (1.0 - 0.5 * gap) * (0.9 + 0.15 * petal);
    Texel {
        color,
        height: petal * 0.3 + disc * 0.4,
        roughness: 0.6,
        cavity: 1.0 - 0.4 * gap,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_weed_line_reads_and_writes_back() {
        let weed = Weed::parse(&["kind=clover", "per_m2=9", "flower=0.6,0.6,0.55"]).unwrap();
        assert_eq!(weed.kind, WeedKind::Clover);
        assert_eq!(weed.per_square_metre, 9.0);
        let words: Vec<String> = weed.to_text().split(' ').map(str::to_string).collect();
        let words: Vec<&str> = words.iter().map(String::as_str).collect();
        assert_eq!(Weed::parse(&words), Ok(weed));
        assert!(Weed::parse(&["kind=thistle"])
            .unwrap_err()
            .contains("thistle"));
    }

    #[test]
    fn plants_face_up_and_sit_on_the_ground() {
        for kind in ["kind=rosette", "kind=clover"] {
            let weed = Weed::parse(&[kind, "flowers=1"]).unwrap();
            let mut mesh = Mesh::default();
            let mut rng = Rng::new(1, 2);
            let identity = |x: f32| x;
            for _ in 0..20 {
                weed.add_plant(&mut mesh, &mut rng, Vec3::ZERO, &identity, &identity, 0.3);
            }
            let mut up = 0;
            let mut total = 0;
            for triangle in mesh.indices.chunks_exact(3) {
                let [a, b, c] = [0, 1, 2].map(|i| mesh.vertices[triangle[i] as usize]);
                let face = (b.position - a.position).cross(c.position - a.position);
                if face.length() < 1.0e-12 {
                    continue;
                }
                total += 1;
                if face.dot(a.normal + b.normal + c.normal) > 0.0 {
                    up += 1;
                }
            }
            assert!(
                up * 100 >= total * 97,
                "{kind}: {up} of {total} wind with their normals"
            );
            assert!(mesh
                .vertices
                .iter()
                .all(|v| v.position.y > -0.05 && v.position.y < 0.4));
        }
    }
}
