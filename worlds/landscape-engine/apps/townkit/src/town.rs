//! What the town is built of: the stonework, and plain massing for what later steps replace.

use crate::geometry::Geometry;
use crate::ground::{self, QUAY_DECK};
use glam::{Vec2, Vec3};

/// How far down the stonework goes, into the seabed.
pub const FOOTING: f32 = -13.0;

/// One flight of water stairs: 16 cm risers on 30 cm treads, from the deck to below the sea.
const STAIR_RISE: f32 = 0.16;
pub const STAIR_GOING: f32 = 0.3;
pub const STAIR_STEPS: usize = 13;
pub const STAIRS_Z: f32 = -10.0;
pub const STAIRS_WIDTH: f32 = 2.0;
/// The length of one bay of the pier, as the scene scales the scan.
const PIER_BAY: f32 = 3.48;
/// Where the scan's own legs end, under the water.
const PIER_PILE_TOP: f32 = -2.6;

/// The quay: its wall leans back a little as it rises, as harbour walls do.
fn quay_length(geometry: &mut Geometry, from: f32, to: f32) {
    let ring = |z: f32| {
        vec![
            Vec3::new(-0.6, FOOTING, z),
            Vec3::new(0.0, QUAY_DECK, z),
            Vec3::new(10.0, QUAY_DECK, z),
            Vec3::new(10.0, FOOTING, z),
        ]
    };
    geometry.loft(&[ring(from), ring(to)]);
}

/// The quay from end to end, cut by its water stairs.
pub fn quay() -> Geometry {
    let mut g = Geometry::default();
    let (north, south) = (STAIRS_Z - STAIRS_WIDTH * 0.5, STAIRS_Z + STAIRS_WIDTH * 0.5);
    quay_length(&mut g, -52.0, north);
    quay_length(&mut g, south, 50.0);
    let run = STAIR_GOING * STAIR_STEPS as f32;
    g.cuboid(
        Vec3::new(run, FOOTING, north),
        Vec3::new(10.0, QUAY_DECK, south),
    );
    for step in 0..STAIR_STEPS {
        let inner = run - STAIR_GOING * step as f32;
        g.cuboid(
            Vec3::new(inner - STAIR_GOING, FOOTING, north),
            Vec3::new(inner, QUAY_DECK - STAIR_RISE * (step + 1) as f32, south),
        );
    }
    g
}

/// The mole's centre line, from inside the quay's north end out to the lighthouse, by distance.
pub struct MolePath {
    points: Vec<Vec2>,
}

impl MolePath {
    const SAMPLES: usize = 800;

    pub fn new() -> Self {
        let [a, b, c, d] = [
            Vec2::new(6.0, -48.0),
            Vec2::new(-35.0, -50.0),
            Vec2::new(-65.0, -62.0),
            Vec2::new(-92.0, -65.0),
        ];
        let point = |t: f32| {
            let s = 1.0 - t;
            a * s * s * s + b * 3.0 * s * s * t + c * 3.0 * s * t * t + d * t * t * t
        };
        Self {
            points: (0..=Self::SAMPLES)
                .map(|i| point(i as f32 / Self::SAMPLES as f32))
                .collect(),
        }
    }

    pub fn length(&self) -> f32 {
        self.points.windows(2).map(|w| (w[1] - w[0]).length()).sum()
    }

    /// The point `distance` along the line, the way it is heading, and the side facing the sea.
    pub fn at(&self, distance: f32) -> (Vec2, Vec2, Vec2) {
        let mut left = distance;
        for w in self.points.windows(2) {
            let step = (w[1] - w[0]).length();
            if left <= step {
                let heading = (w[1] - w[0]) / step;
                return (
                    w[0] + heading * left,
                    heading,
                    Vec2::new(-heading.y, heading.x),
                );
            }
            left -= step;
        }
        let n = self.points.len();
        let heading = (self.points[n - 1] - self.points[n - 2]).normalize();
        (
            self.points[n - 1],
            heading,
            Vec2::new(-heading.y, heading.x),
        )
    }
}

/// Deck height of the mole, above the quay's.
pub const MOLE_DECK: f32 = 3.2;
/// The top of the mole's seaward parapet.
pub const PARAPET_TOP: f32 = 4.4;
/// How far along its centre line the mole leaves the quay.
pub const MOLE_START: f32 = 6.0;
/// The flight of steps from the quay's deck up to the mole's: level stone courses cannot follow a
/// ramp.
const MOLE_STEPS: usize = 8;
const MOLE_RISE: f32 = (MOLE_DECK - QUAY_DECK) / MOLE_STEPS as f32;
pub const MOLE_STEPS_END: f32 = MOLE_START + STAIR_GOING * MOLE_STEPS as f32;
const MOLE_STATIONS: usize = 48;
/// The cross-section's offsets towards the sea, and heights, at the top and foot of each face.
pub const SEA_FACE: [(f32, f32); 2] = [(3.5, PARAPET_TOP), (4.8, FOOTING)];
pub const BASIN_FACE: [(f32, f32); 2] = [(-3.5, MOLE_DECK), (-4.6, FOOTING)];
pub const PARAPET_INNER: f32 = 2.9;
/// The round platform at the mole's end, which carries the lighthouse.
pub const HEAD_CENTRE: Vec3 = Vec3::new(-92.0, FOOTING, -65.0);
pub const HEAD_TOP_RADIUS: f32 = 7.5;
pub const HEAD_FOOT_RADIUS: f32 = 10.0;

/// A breakwater with a parapet on its seaward (north) side, reached from the quay by steps.
pub fn mole() -> Geometry {
    let mut g = Geometry::default();
    let path = MolePath::new();
    let ring = |distance: f32, deck: f32| {
        let (p, _, seaward) = path.at(distance);
        [
            SEA_FACE[1],
            SEA_FACE[0],
            (PARAPET_INNER, PARAPET_TOP),
            (PARAPET_INNER, deck),
            (BASIN_FACE[0].0, deck),
            BASIN_FACE[1],
        ]
        .map(|(offset, y)| {
            let q = p + seaward * offset;
            Vec3::new(q.x, y, q.y)
        })
        .to_vec()
    };
    for step in 0..MOLE_STEPS {
        // The first step starts well back on the quay, since the mole leaves it at an angle.
        let back = if step == 0 { 0.5 } else { 0.0 };
        let from = MOLE_START + STAIR_GOING * step as f32 - back;
        let deck = QUAY_DECK + MOLE_RISE * (step + 1) as f32;
        g.loft(&[ring(from, deck), ring(from + STAIR_GOING + back, deck)]);
    }
    let end = path.length();
    let rings: Vec<Vec<Vec3>> = (0..=MOLE_STATIONS)
        .map(|i| {
            let t = i as f32 / MOLE_STATIONS as f32;
            ring(MOLE_STEPS_END + (end - MOLE_STEPS_END) * t, MOLE_DECK)
        })
        .collect();
    g.loft(&rings);
    g.frustum(
        HEAD_CENTRE,
        HEAD_FOOT_RADIUS,
        HEAD_TOP_RADIUS,
        MOLE_DECK - FOOTING,
        48,
    );
    g
}

/// The materials massing is drawn in, in order.
#[derive(Default)]
pub struct Massing {
    pub walls: Geometry,
    pub roofs: Geometry,
    pub stone: Geometry,
    pub timber: Geometry,
}

/// How far a building's walls go below the ground at its lowest corner.
const SINK: f32 = 0.5;

/// A plain building with a pitched roof, standing on the ground. Its ridge runs along z, or along x.
fn building(m: &mut Massing, min: Vec2, max: Vec2, eaves: f32, ridge: f32, ridge_along_z: bool) {
    let foot = [min, max, Vec2::new(min.x, max.y), Vec2::new(max.x, min.y)]
        .map(|c| ground::height(c.x, c.y));
    let base = foot.iter().copied().fold(f32::MAX, f32::min);
    let top = foot.iter().copied().fold(f32::MIN, f32::max);
    let (low, wall, peak) = (base - SINK, top + eaves, top + ridge);
    let p = |x: f32, y: f32, z: f32| Vec3::new(x, y, z);
    let mid = (min + max) * 0.5;
    let (walls, roofs) = (&mut m.walls, &mut m.roofs);
    walls.polygon(
        &[
            p(min.x, low, min.y),
            p(max.x, low, min.y),
            p(max.x, low, max.y),
            p(min.x, low, max.y),
        ],
        -Vec3::Y,
    );
    let mut wall_face = |corners: &[Vec3], outward: Vec3| walls.polygon(corners, outward);
    if ridge_along_z {
        wall_face(
            &[
                p(min.x, low, min.y),
                p(min.x, low, max.y),
                p(min.x, wall, max.y),
                p(min.x, wall, min.y),
            ],
            -Vec3::X,
        );
        wall_face(
            &[
                p(max.x, low, min.y),
                p(max.x, low, max.y),
                p(max.x, wall, max.y),
                p(max.x, wall, min.y),
            ],
            Vec3::X,
        );
        for (z, out) in [(min.y, -Vec3::Z), (max.y, Vec3::Z)] {
            wall_face(
                &[
                    p(min.x, low, z),
                    p(max.x, low, z),
                    p(max.x, wall, z),
                    p(mid.x, peak, z),
                    p(min.x, wall, z),
                ],
                out,
            );
        }
        roofs.polygon(
            &[
                p(min.x, wall, min.y),
                p(min.x, wall, max.y),
                p(mid.x, peak, max.y),
                p(mid.x, peak, min.y),
            ],
            Vec3::new(-1.0, 1.0, 0.0),
        );
        roofs.polygon(
            &[
                p(max.x, wall, min.y),
                p(max.x, wall, max.y),
                p(mid.x, peak, max.y),
                p(mid.x, peak, min.y),
            ],
            Vec3::new(1.0, 1.0, 0.0),
        );
    } else {
        wall_face(
            &[
                p(min.x, low, min.y),
                p(max.x, low, min.y),
                p(max.x, wall, min.y),
                p(min.x, wall, min.y),
            ],
            -Vec3::Z,
        );
        wall_face(
            &[
                p(min.x, low, max.y),
                p(max.x, low, max.y),
                p(max.x, wall, max.y),
                p(min.x, wall, max.y),
            ],
            Vec3::Z,
        );
        for (x, out) in [(min.x, -Vec3::X), (max.x, Vec3::X)] {
            wall_face(
                &[
                    p(x, low, min.y),
                    p(x, low, max.y),
                    p(x, wall, max.y),
                    p(x, peak, mid.y),
                    p(x, wall, min.y),
                ],
                out,
            );
        }
        roofs.polygon(
            &[
                p(min.x, wall, min.y),
                p(max.x, wall, min.y),
                p(max.x, peak, mid.y),
                p(min.x, peak, mid.y),
            ],
            Vec3::new(0.0, 1.0, -1.0),
        );
        roofs.polygon(
            &[
                p(min.x, wall, max.y),
                p(max.x, wall, max.y),
                p(max.x, peak, mid.y),
                p(min.x, peak, mid.y),
            ],
            Vec3::new(0.0, 1.0, 1.0),
        );
    }
}

/// The warehouse, chapel, wall, lighthouse and ships, in plain shapes.
/// Each is replaced by the real thing as its step lands.
pub fn massing() -> Massing {
    let mut m = Massing::default();
    let block = |x0: f32, z0: f32, x1: f32, z1: f32| (Vec2::new(x0, z0), Vec2::new(x1, z1));

    let (min, max) = block(14.0, -46.0, 38.0, -36.0);
    building(&mut m, min, max, 5.5, 8.0, false);
    // The chapel: a nave with its door on Chapel Steps, and a square tower on its seaward side.
    let (min, max) = block(90.5, 27.0, 99.5, 45.0);
    building(&mut m, min, max, 7.0, 11.0, true);
    let foot = ground::height(85.5, 28.0).min(ground::height(90.5, 33.0));
    m.stone.cuboid(
        Vec3::new(85.5, foot - SINK, 28.0),
        Vec3::new(90.5, foot + 22.0, 33.0),
    );

    // The town wall, stepping with the ground in lengths, with the gate's opening at z = 0.
    for (from, to) in [(-70.0_f32, -1.7_f32), (1.7, 70.0)] {
        let lengths = ((to - from) / 8.0).round();
        let step = (to - from) / lengths;
        for i in 0..lengths as u32 {
            let (a, b) = (from + step * i as f32, from + step * (i + 1) as f32);
            let (ground_a, ground_b) = (ground::height(128.0, a), ground::height(128.0, b));
            m.stone.cuboid(
                Vec3::new(127.0, ground_a.min(ground_b) - SINK * 4.0, a),
                Vec3::new(129.0, ground_a.max(ground_b) + 6.5, b),
            );
        }
    }

    // The lighthouse: a tapering tower, then its lantern.
    let base = Vec3::new(-92.0, MOLE_DECK, -64.0);
    m.stone.frustum(base, 3.5, 2.4, 18.0, 32);
    m.timber.frustum(base + Vec3::Y * 18.0, 1.4, 1.4, 4.0, 16);

    // The step from the quay down to the timber pier, whose scan the scene places, and the piles
    // that carry it on down to the seabed, below where the scan's own legs end.
    m.stone.cuboid(
        Vec3::new(-0.3, 0.9, 38.5),
        Vec3::new(0.0, QUAY_DECK - STAIR_RISE, 41.5),
    );
    let mut pile = |x: f32, z: f32| {
        m.timber
            .frustum(Vec3::new(x, -9.0, z), 0.14, 0.14, PIER_PILE_TOP + 9.0, 8);
    };
    for i in 0..14 {
        let x = -PIER_BAY * i as f32;
        pile(x, 38.8);
        pile(x, 41.2);
    }
    for i in 0..7 {
        let z = 29.55 + PIER_BAY * i as f32;
        pile(-50.0, z);
        pile(-45.3, z);
    }

    // A merchant ship alongside the quay, and small boats on their moorings.
    m.timber
        .cuboid(Vec3::new(-9.0, -2.5, -22.0), Vec3::new(-2.5, 3.0, 18.0));
    for z in [-12.0, 8.0] {
        m.timber
            .frustum(Vec3::new(-5.8, 3.0, z), 0.3, 0.15, 24.0, 12);
    }
    for (x, z, yaw) in [
        (-58.0, -15.0, 0.3),
        (-62.0, 10.0, -0.2),
        (-55.0, 24.0, 0.5),
        (-30.0, 30.0, 1.5),
        (-20.0, 33.0, 1.4),
        (-35.0, 47.0, 1.5),
    ] {
        let (sin, cos) = f32::sin_cos(yaw);
        let corner = |dx: f32, dz: f32| Vec2::new(x + dx * cos - dz * sin, z + dx * sin + dz * cos);
        let ring = |y: f32, scale: f32| {
            [(-1.0, -3.2), (1.0, -3.2), (1.0, 3.2), (-1.0, 3.2)]
                .map(|(dx, dz)| {
                    let c = corner(dx * scale, dz);
                    Vec3::new(c.x, y, c.y)
                })
                .to_vec()
        };
        m.timber.loft(&[ring(-0.5, 0.7), ring(0.75, 1.0)]);
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_water_stairs_reach_below_the_sea_within_the_collision_limit() {
        assert!(QUAY_DECK - STAIR_RISE * (STAIR_STEPS as f32) < 0.0);
        const { assert!(STAIR_RISE <= 0.17) };
    }

    #[test]
    fn the_ground_stays_below_every_tread_of_the_water_stairs() {
        let run = STAIR_GOING * STAIR_STEPS as f32;
        for step in 0..STAIR_STEPS {
            let top = QUAY_DECK - STAIR_RISE * (step + 1) as f32;
            let inner = run - STAIR_GOING * step as f32;
            for x in [inner - STAIR_GOING, inner] {
                // The mesh is straight between its grid corners, one metre apart.
                let (low, t) = (x.floor(), x.fract());
                let surface = ground::height(low, STAIRS_Z) * (1.0 - t)
                    + ground::height(low + 1.0, STAIRS_Z) * t;
                assert!(surface < top, "step {step} is buried at {x}: {surface}");
            }
        }
    }

    #[test]
    fn the_mole_is_about_a_hundred_metres_and_its_steps_are_within_the_collision_limit() {
        let length = MolePath::new().length();
        assert!((90.0..110.0).contains(&length), "{length}");
        const { assert!(MOLE_RISE <= 0.17) };
    }
}
