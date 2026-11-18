//! The houses: eight plans, each one set of meshes shared by every house built from it.
//!
//! A plan's origin is the middle of its front wall at street level; it faces +Z and runs back
//! along -Z. Walls and shutters are built once per colour, which costs almost nothing. Roofs, the
//! only heavy part, are a sheet of real tiles shared by every house of a plan and by both its
//! slopes, and the virtual geometry draws them coarser with distance.

use crate::geometry::Geometry;
use crate::gltf::{File, Plain, Textured};
use crate::ground;
use crate::stone::{sheet, HeightMap, FLAGS};
use glam::{Quat, Vec2, Vec3};
use std::io;
use std::path::Path;

/// One house design.
#[derive(Clone, Copy)]
struct Plan {
    width: f32,
    depth: f32,
    storeys: u32,
    bays: u32,
    door_bay: u32,
    /// A wide window on the ground floor, in the bay after the door's.
    shop: bool,
    /// A balcony on the first floor, in the middle bay.
    balcony: bool,
    /// Extra height on each upper storey, so that a street's eaves do not line up.
    lift: f32,
    /// Dressed corner stones, for the better houses only.
    quoins: bool,
}

const fn plan(width: f32, depth: f32, storeys: u32, bays: u32, door_bay: u32) -> Plan {
    Plan {
        width,
        depth,
        storeys,
        bays,
        door_bay,
        shop: false,
        balcony: false,
        lift: 0.0,
        quoins: false,
    }
}

const fn with_shop(mut p: Plan) -> Plan {
    p.shop = true;
    p
}

const fn with_balcony(mut p: Plan) -> Plan {
    p.balcony = true;
    p
}

const fn lifted(mut p: Plan, lift: f32) -> Plan {
    p.lift = lift;
    p
}

const fn with_quoins(mut p: Plan) -> Plan {
    p.quoins = true;
    p
}

const PLANS: [Plan; 8] = [
    lifted(plan(4.5, 12.0, 3, 2, 0), -0.12),
    lifted(with_balcony(plan(5.5, 12.0, 3, 2, 1)), 0.2),
    with_shop(plan(6.5, 12.0, 2, 3, 0)),
    lifted(with_quoins(with_balcony(plan(8.0, 12.0, 3, 3, 1))), 0.35),
    lifted(plan(5.5, 9.0, 3, 2, 0), 0.1),
    lifted(with_shop(plan(7.0, 9.0, 2, 3, 2)), 0.25),
    lifted(plan(5.5, 6.4, 3, 2, 1), -0.1),
    with_quoins(plan(7.0, 6.4, 2, 3, 1)),
];

/// Which plans suit a block of each depth.
const DEEP: [usize; 4] = [0, 1, 2, 3];
/// Where the street climbs to a house's right, a low shop window would run into the rising ground.
const DEEP_CLIMBING: [usize; 3] = [0, 1, 3];
const MIDDLING: [usize; 2] = [4, 5];
const SHALLOW: [usize; 2] = [6, 7];

const GROUND_STOREY: f32 = 3.2;
const UPPER_STOREY: f32 = 3.0;
/// From the top floor's ceiling up to the eaves.
const TO_EAVES: f32 = 0.3;
/// Rise of the roof per metre of run.
const PITCH: f32 = 0.55;
/// How far the roof runs past the wall at the eaves.
const OVERHANG: f32 = 0.3;
/// How far walls run down behind the street, so that a house on a slope never shows its foot.
const SKIRT: f32 = 2.0;

const WINDOW_WIDTH: f32 = 1.0;
const WINDOW_HEIGHT: f32 = 1.6;
const SILL_HEIGHT: f32 = 0.95;
pub const DOOR_WIDTH: f32 = 1.1;
pub const DOOR_HEIGHT: f32 = 2.25;
/// How deep the walls are at their openings.
const REVEAL: f32 = 0.24;
/// The granite beside an opening.
const SURROUND: f32 = 0.14;
/// The gap between a window's surround and its shutter.
const SHUTTER_CLEAR: f32 = 0.16;
/// The sill stone's depth below an opening.
const SILL_STONE: f32 = 0.12;

/// How far apart a roof's vertices are, which sets what a roof costs.
const ROOF_GRID: f32 = 0.1;
const ROOF_DEPTH: f32 = 0.05;

const STUCCO: Textured = Textured {
    surface: "white_stucco",
    metres: 2.0,
    arm: true,
    tint: [1.0; 3],
};
/// Limewash, and the ochres this coast's towns mix into it.
const WALL_TINTS: [[f32; 3]; 3] = [[0.96, 0.94, 0.9], [0.86, 0.74, 0.54], [0.9, 0.82, 0.74]];
const ROOF: Textured = Textured {
    surface: "clay_roof_tiles_02",
    metres: 2.5,
    arm: true,
    tint: [1.0, 0.96, 0.9],
};
const PLANKS: Textured = Textured {
    surface: "distressed_painted_planks",
    metres: 1.6,
    arm: true,
    tint: [0.4, 0.3, 0.22],
};
/// The dressed stone of the openings: paler than the harbour's weathered walls. Its tint is above
/// 1, which only this project's importer reads; the texture is too dark for any tint that isn't.
const TRIM: Textured = Textured {
    tint: [1.3, 1.28, 1.24],
    ..FLAGS
};
/// The planks' paint is olive, about (0.22, 0.2, 0.14), so a tint can only pick out one colour.
const SHUTTER_TINTS: [[f32; 3]; 3] = [[0.35, 0.8, 0.55], [0.25, 0.6, 1.0], [1.0, 0.35, 0.3]];

/// What to build. Leaving parts out is how each one's cost is measured.
#[derive(Clone, Copy)]
pub struct Parts {
    pub walls: bool,
    pub roofs: bool,
    pub details: bool,
}

/// One wall face. Seen from outside, X runs to the right and Y up from the street.
#[derive(Clone, Copy)]
struct Face {
    origin: Vec3,
    right: Vec3,
    out: Vec3,
}

impl Face {
    /// A point on the face, `inward` metres behind it (negative is proud of it).
    fn at(&self, x: f32, y: f32, inward: f32) -> Vec3 {
        self.origin + self.right * x + Vec3::Y * y - self.out * inward
    }

    /// A block on the face, from `recess` behind it to `proud` in front.
    fn block(&self, g: &mut Geometry, x: (f32, f32), y: (f32, f32), proud: f32, recess: f32) {
        let (a, b) = (self.at(x.0, y.0, -proud), self.at(x.1, y.1, recess));
        g.cuboid(a.min(b), a.max(b));
    }
}

#[derive(Clone, Copy)]
struct Opening {
    x: (f32, f32),
    y: (f32, f32),
}

/// A wall with holes: quads around the openings, and reveals running back into them.
fn wall(g: &mut Geometry, face: &Face, width: f32, height: f32, openings: &[Opening]) {
    let mut xs = vec![0.0, width];
    let mut ys = vec![-SKIRT, height];
    for o in openings {
        xs.extend([o.x.0, o.x.1]);
        ys.extend([o.y.0, o.y.1]);
    }
    for list in [&mut xs, &mut ys] {
        list.sort_by(f32::total_cmp);
        list.dedup_by(|a, b| (*a - *b).abs() < 1.0e-4);
    }
    for (x0, x1) in xs.windows(2).map(|w| (w[0], w[1])) {
        for (y0, y1) in ys.windows(2).map(|w| (w[0], w[1])) {
            let middle = ((x0 + x1) * 0.5, (y0 + y1) * 0.5);
            let inside = |o: &Opening| {
                o.x.0 < middle.0 && middle.0 < o.x.1 && o.y.0 < middle.1 && middle.1 < o.y.1
            };
            if !openings.iter().any(inside) {
                g.polygon(
                    &[
                        face.at(x0, y0, 0.0),
                        face.at(x1, y0, 0.0),
                        face.at(x1, y1, 0.0),
                        face.at(x0, y1, 0.0),
                    ],
                    face.out,
                );
            }
        }
    }
    for o in openings {
        let corner = |x: f32, y: f32, d: f32| face.at(x, y, d);
        for (a, b, outward) in [
            ((o.x.0, o.y.0), (o.x.0, o.y.1), face.right),
            ((o.x.1, o.y.0), (o.x.1, o.y.1), -face.right),
            ((o.x.0, o.y.1), (o.x.1, o.y.1), -Vec3::Y),
            ((o.x.0, o.y.0), (o.x.1, o.y.0), Vec3::Y),
        ] {
            g.polygon(
                &[
                    corner(a.0, a.1, 0.0),
                    corner(b.0, b.1, 0.0),
                    corner(b.0, b.1, REVEAL),
                    corner(a.0, a.1, REVEAL),
                ],
                outward,
            );
        }
    }
}

/// The meshes of one plan, by material.
#[derive(Default)]
struct Built {
    walls: Geometry,
    trim: Geometry,
    wood: Geometry,
    frames: Geometry,
    glass: Geometry,
    iron: Geometry,
    ridge: Geometry,
    shutters: Geometry,
}

impl Plan {
    fn eaves(&self) -> f32 {
        GROUND_STOREY + (UPPER_STOREY + self.lift) * (self.storeys - 1) as f32 + TO_EAVES
    }

    fn ridge(&self) -> f32 {
        self.eaves() + PITCH * self.depth * 0.5
    }

    fn bay_centre(&self, bay: u32) -> f32 {
        (bay as f32 + 0.5) * self.width / self.bays as f32
    }

    fn floor(&self, storey: u32) -> f32 {
        if storey == 0 {
            0.0
        } else {
            GROUND_STOREY + (UPPER_STOREY + self.lift) * (storey - 1) as f32
        }
    }

    /// Where the door is, across the front, from the plan's centre line.
    fn door_x(&self) -> f32 {
        self.bay_centre(self.door_bay) - self.width * 0.5
    }

    fn balcony_bay(&self) -> Option<u32> {
        self.balcony.then_some(self.bays / 2)
    }

    /// The front's openings: the door, shop window and windows, each with what kind it is.
    fn front_openings(&self) -> Vec<(Opening, Kind)> {
        let mut list = Vec::new();
        for storey in 0..self.storeys {
            for bay in 0..self.bays {
                let c = self.bay_centre(bay);
                let floor = self.floor(storey);
                let centred = |half: f32, y0: f32, y1: f32| Opening {
                    x: (c - half, c + half),
                    y: (y0, y1),
                };
                let window = centred(
                    WINDOW_WIDTH * 0.5,
                    floor + SILL_HEIGHT,
                    floor + SILL_HEIGHT + WINDOW_HEIGHT,
                );
                list.push(if storey == 0 && bay == self.door_bay {
                    (centred(DOOR_WIDTH * 0.5, 0.0, DOOR_HEIGHT), Kind::Door)
                } else if storey == 0 && self.shop && bay == (self.door_bay + 1) % self.bays {
                    (centred(0.8, 0.5, 2.45), Kind::Shop)
                } else if storey == 1 && self.balcony_bay() == Some(bay) {
                    (centred(0.5, floor + 0.02, floor + 2.3), Kind::Balcony)
                } else if storey == 0 {
                    (window, Kind::Plain)
                } else {
                    (window, Kind::Shuttered)
                });
            }
        }
        list
    }

    /// The back has only windows, and none on the ground floor where the hill is `banked` against it.
    fn rear_openings(&self, banked: bool) -> Vec<(Opening, Kind)> {
        let mut list = Vec::new();
        for storey in u32::from(banked)..self.storeys {
            for bay in 0..self.bays {
                let c = self.bay_centre(bay);
                let floor = self.floor(storey) + SILL_HEIGHT;
                let opening = Opening {
                    x: (c - WINDOW_WIDTH * 0.5, c + WINDOW_WIDTH * 0.5),
                    y: (floor, floor + WINDOW_HEIGHT),
                };
                list.push((opening, Kind::Plain));
            }
        }
        list
    }

    /// How wide a shutter leaf can be with every window in the front given the same pair, if
    /// there is room for one. Quoined houses have none: their corner stones take the room.
    fn shutter_leaf(&self) -> Option<f32> {
        let margin = if self.quoins { 0.44 } else { 0.04 };
        let beside = (self.width / self.bays as f32 - WINDOW_WIDTH) * 0.5;
        let room = beside - SHUTTER_CLEAR - margin;
        (room >= 0.3).then_some(room.min(WINDOW_WIDTH * 0.5))
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Door,
    Shop,
    Balcony,
    Shuttered,
    Plain,
}

/// A window's frame, bars and glass, set back in its opening.
fn glaze(built: &mut Built, face: &Face, o: Opening) {
    const BAR: f32 = 0.05;
    const SET_BACK: (f32, f32) = (0.12, 0.17);
    let frames = &mut built.frames;
    let mut bar = |x: (f32, f32), y: (f32, f32)| face.block(frames, x, y, -SET_BACK.0, SET_BACK.1);
    bar((o.x.0, o.x.0 + BAR), o.y);
    bar((o.x.1 - BAR, o.x.1), o.y);
    bar(o.x, (o.y.0, o.y.0 + BAR));
    bar(o.x, (o.y.1 - BAR, o.y.1));
    let middle = (o.x.0 + o.x.1) * 0.5;
    bar((middle - BAR * 0.5, middle + BAR * 0.5), o.y);
    let height = o.y.0 + (o.y.1 - o.y.0) * 0.62;
    bar(o.x, (height - BAR * 0.5, height + BAR * 0.5));
    built.glass.polygon(
        &[
            face.at(o.x.0, o.y.0, SET_BACK.1),
            face.at(o.x.1, o.y.0, SET_BACK.1),
            face.at(o.x.1, o.y.1, SET_BACK.1),
            face.at(o.x.0, o.y.1, SET_BACK.1),
        ],
        face.out,
    );
}

/// The granite framing an opening, as this coast's houses have it.
fn surround(trim: &mut Geometry, face: &Face, o: Opening, sill: bool) {
    const PROUD: f32 = 0.03;
    let (left, right) = ((o.x.0 - SURROUND, o.x.0), (o.x.1, o.x.1 + SURROUND));
    face.block(trim, left, o.y, PROUD, 0.0);
    face.block(trim, right, o.y, PROUD, 0.0);
    face.block(
        trim,
        (left.0, right.1),
        (o.y.1, o.y.1 + 0.2),
        PROUD + 0.01,
        0.0,
    );
    if sill {
        face.block(
            trim,
            (left.0 - 0.04, right.1 + 0.04),
            (o.y.0 - SILL_STONE, o.y.0),
            PROUD + 0.06,
            0.0,
        );
    }
}

fn build_facade(built: &mut Built, plan: &Plan, face: &Face, openings: &[(Opening, Kind)]) {
    let holes: Vec<Opening> = openings.iter().map(|(o, _)| *o).collect();
    wall(&mut built.walls, face, plan.width, plan.eaves(), &holes);
    // The plinth, stopping either side of a door, and the cornice under the eaves.
    let across = (0.0, plan.width);
    face.block(&mut built.trim, across, (-SKIRT, 0.0), 0.03, 0.0);
    let mut start = 0.0;
    for (o, _) in openings.iter().filter(|(_, kind)| *kind == Kind::Door) {
        let end = o.x.0 - SURROUND;
        face.block(&mut built.trim, (start, end), (0.0, 0.4), 0.03, 0.0);
        start = o.x.1 + SURROUND;
    }
    face.block(&mut built.trim, (start, plan.width), (0.0, 0.4), 0.03, 0.0);
    face.block(
        &mut built.trim,
        across,
        (plan.eaves() - 0.42, plan.eaves() - PITCH * OVERHANG),
        OVERHANG,
        0.0,
    );
    for &(o, kind) in openings {
        match kind {
            Kind::Door => {
                surround(&mut built.trim, face, o, false);
                face.block(&mut built.wood, o.x, o.y, -0.12, 0.2);
                // Running down into the ground, since the street may fall away along a slope.
                let step = (o.x.0 - 0.05, o.x.1 + 0.05);
                face.block(&mut built.trim, step, (-0.4, 0.05), 0.25, 0.0);
            }
            Kind::Shop => {
                surround(&mut built.trim, face, o, true);
                glaze(built, face, o);
            }
            Kind::Plain => {
                surround(&mut built.trim, face, o, true);
                glaze(built, face, o);
            }
            Kind::Shuttered => {
                surround(&mut built.trim, face, o, true);
                glaze(built, face, o);
                if let Some(leaf) = plan.shutter_leaf() {
                    let left = (o.x.0 - SHUTTER_CLEAR - leaf, o.x.0 - SHUTTER_CLEAR);
                    let right = (o.x.1 + SHUTTER_CLEAR, o.x.1 + SHUTTER_CLEAR + leaf);
                    face.block(&mut built.shutters, left, o.y, 0.04, 0.0);
                    face.block(&mut built.shutters, right, o.y, 0.04, 0.0);
                }
            }
            Kind::Balcony => {
                surround(&mut built.trim, face, o, false);
                glaze(built, face, o);
                let (centre, floor) = ((o.x.0 + o.x.1) * 0.5, o.y.0);
                let span = (centre - 0.95, centre + 0.95);
                face.block(&mut built.trim, span, (floor - 0.14, floor), 0.75, 0.0);
                balcony_rail(&mut built.iron, face, span, floor);
            }
        }
    }
}

fn balcony_rail(iron: &mut Geometry, face: &Face, span: (f32, f32), floor: f32) {
    const REACH: f32 = 0.7;
    const THICK: f32 = 0.02;
    const HEIGHT: f32 = 1.0;
    let rail = (floor + HEIGHT - THICK, floor + HEIGHT);
    face.block(iron, span, rail, REACH, -REACH + THICK);
    let posts = 13;
    for i in 0..posts {
        let x = span.0 + (span.1 - span.0) * i as f32 / (posts - 1) as f32;
        face.block(
            iron,
            (x - THICK * 0.5, x + THICK * 0.5),
            (floor, floor + HEIGHT),
            REACH,
            -REACH + THICK,
        );
    }
    for x in [span.0, span.1 - THICK] {
        face.block(iron, (x, x + THICK), rail, REACH, 0.0);
    }
}

/// Everything of a plan except its roof tiles.
fn build(plan: &Plan, banked: bool) -> Built {
    let mut built = Built::default();
    let (half, depth) = (plan.width * 0.5, plan.depth);
    let front = Face {
        origin: Vec3::new(-half, 0.0, 0.0),
        right: Vec3::X,
        out: Vec3::Z,
    };
    let rear = Face {
        origin: Vec3::new(half, 0.0, -depth),
        right: -Vec3::X,
        out: -Vec3::Z,
    };
    build_facade(&mut built, plan, &front, &plan.front_openings());
    build_facade(&mut built, plan, &rear, &plan.rear_openings(banked));

    // Corner stones up the front, long and short by turns.
    for side in [false, true].into_iter().filter(|_| plan.quoins) {
        let mut y = 0.4;
        let mut long = true;
        while y < plan.eaves() - 0.5 {
            let reach = if long { 0.42 } else { 0.26 };
            let x = if side {
                (plan.width - reach, plan.width)
            } else {
                (0.0, reach)
            };
            front.block(&mut built.trim, x, (y, y + 0.3), 0.025, 0.0);
            y += 0.3;
            long = !long;
        }
    }

    // The gable walls, stopping under their stone copings, which stand just above the tiles.
    let (eaves, ridge) = (plan.eaves(), plan.ridge());
    const COPING_DROP: f32 = 0.3;
    for x in [-half, half] {
        let outward = Vec3::X * x.signum();
        built.walls.polygon(
            &[
                Vec3::new(x, -SKIRT, 0.0),
                Vec3::new(x, -SKIRT, -depth),
                Vec3::new(x, eaves - COPING_DROP, -depth),
                Vec3::new(x, ridge - COPING_DROP, -depth * 0.5),
                Vec3::new(x, eaves - COPING_DROP, 0.0),
            ],
            outward,
        );
        let inner = x - outward.x * 0.3;
        let ring = |z: f32| {
            let y = ridge - PITCH * (z + depth * 0.5).abs();
            vec![
                Vec3::new(x, y - COPING_DROP, z),
                Vec3::new(inner, y - COPING_DROP, z),
                Vec3::new(inner, y + 0.2, z),
                Vec3::new(x, y + 0.2, z),
            ]
        };
        built.trim.loft(&[ring(0.0), ring(-depth * 0.5)]);
        built.trim.loft(&[ring(-depth * 0.5), ring(-depth)]);
    }

    // The ridge tiles.
    let ring = |x: f32| {
        [
            (-0.2, -0.08),
            (-0.14, 0.05),
            (0.0, 0.1),
            (0.14, 0.05),
            (0.2, -0.08),
        ]
        .map(|(z, y)| Vec3::new(x, ridge + y, z - depth * 0.5))
        .to_vec()
    };
    built.ridge.loft(&[ring(-half + 0.3), ring(half - 0.3)]);

    // A chimney on the back slope, near one gable, in the house's own colour with a stone cap.
    let stack = (half - 0.9, -depth * 0.5 - 0.9);
    built.walls.cuboid(
        Vec3::new(stack.0 - 0.45, eaves, stack.1 - 0.3),
        Vec3::new(stack.0 + 0.45, ridge + 1.1, stack.1 + 0.3),
    );
    built.trim.cuboid(
        Vec3::new(stack.0 - 0.55, ridge + 1.1, stack.1 - 0.4),
        Vec3::new(stack.0 + 0.55, ridge + 1.25, stack.1 + 0.4),
    );
    built
}

/// One slope of tiles, with the ridge at the origin, falling towards +Z.
fn slope(plan: &Plan, map: &HeightMap) -> Geometry {
    let run = plan.depth * 0.5 + OVERHANG;
    let length = run * (1.0 + PITCH * PITCH).sqrt();
    let columns = (plan.width / ROOF_GRID).round() as u32;
    let rows = (length / ROOF_GRID).round() as u32;
    let normal = Vec3::new(0.0, 1.0, PITCH).normalize();
    sheet(columns, rows, map, ROOF.metres, ROOF_DEPTH, |i, j| {
        let (across, down) = (i as f32 / columns as f32, j as f32 / rows as f32);
        (
            Vec3::new(
                -plan.width * 0.5 + across * plan.width,
                -PITCH * run * down,
                run * down,
            ),
            normal,
            Vec2::new(across * plan.width, down * length),
        )
    })
}

/// A block of houses. Its front is on the side given, and it fills the block's length.
#[derive(Clone, Copy)]
struct Block {
    min: Vec2,
    max: Vec2,
    front: Front,
}

#[derive(Clone, Copy, PartialEq)]
enum Front {
    West,
    South,
    North,
}

const fn block(x0: f32, z0: f32, x1: f32, z1: f32, front: Front) -> Block {
    Block {
        min: Vec2::new(x0, z0),
        max: Vec2::new(x1, z1),
        front,
    }
}

/// The town's blocks, with the gaps for its lanes between them.
const BLOCKS: [Block; 15] = [
    // The quay-front row, broken by Net Lane, the passage from the water stairs and Chapel Steps.
    block(11.0, -36.0, 23.0, -31.6, Front::West),
    block(11.0, -28.4, 23.0, -11.6, Front::West),
    block(11.0, -8.4, 23.0, 23.7, Front::West),
    block(11.0, 26.3, 23.0, 50.0, Front::West),
    // Round the square.
    block(26.0, -28.0, 56.0, -16.0, Front::South),
    block(26.0, 17.0, 56.0, 23.6, Front::North),
    block(57.0, -14.0, 68.0, 16.0, Front::West),
    // Along Net Lane and Chapel Steps.
    block(38.0, -46.0, 68.0, -32.0, Front::South),
    block(26.0, 26.5, 68.0, 44.0, Front::North),
    block(72.0, -46.0, 104.0, -32.0, Front::South),
    // Up the hill, each row above the last.
    block(72.0, -28.0, 84.0, 22.0, Front::West),
    block(92.0, -28.0, 104.0, 18.0, Front::West),
    block(108.0, -46.0, 120.0, -32.0, Front::West),
    block(108.0, -28.0, 120.0, 22.0, Front::West),
    block(108.0, 26.5, 120.0, 46.0, Front::West),
];

impl Block {
    /// How long the block runs along its street, and how deep it is behind it.
    fn length_and_depth(&self) -> (f32, f32) {
        let size = self.max - self.min;
        if self.front == Front::West {
            (size.y, size.x)
        } else {
            (size.x, size.y)
        }
    }

    fn plans(&self) -> &'static [usize] {
        let depth = self.length_and_depth().1;
        if depth < 8.0 {
            &SHALLOW
        } else if depth < 11.5 {
            &MIDDLING
        } else if self.front == Front::South
            && ground::height(self.max.x, self.max.y) - ground::height(self.min.x, self.max.y) > 1.0
        {
            &DEEP_CLIMBING
        } else {
            &DEEP
        }
    }

    /// The point on the front wall, `along` metres down the street.
    fn front_point(&self, along: f32) -> Vec2 {
        match self.front {
            Front::West => Vec2::new(self.min.x, self.min.y + along),
            Front::South => Vec2::new(self.min.x + along, self.max.y),
            Front::North => Vec2::new(self.min.x + along, self.min.y),
        }
    }

    fn turn(&self) -> Quat {
        let degrees: f32 = match self.front {
            Front::West => -90.0,
            Front::South => 0.0,
            Front::North => 180.0,
        };
        Quat::from_rotation_y(degrees.to_radians())
    }
}

/// Fills `length` with plans taken in turn from `choices`, starting wherever gives houses closest
/// to their true size, and the scale that makes them fit it exactly.
fn fill(length: f32, choices: &[usize], seed: usize) -> (Vec<usize>, f32) {
    let run = |start: usize| {
        let (mut chosen, mut total, mut turn) = (Vec::new(), 0.0, start);
        loop {
            let index = choices[turn % choices.len()];
            let width = PLANS[index].width;
            if !chosen.is_empty() && total + width * 0.5 > length {
                break;
            }
            chosen.push(index);
            total += width;
            turn += 3;
        }
        (chosen, length / total)
    };
    (seed..seed + choices.len())
        .map(run)
        .min_by(|a, b| (a.1 - 1.0).abs().total_cmp(&(b.1 - 1.0).abs()))
        .expect("there are always some choices")
}

/// A house in its place.
struct Placed {
    plan: usize,
    at: Vec3,
    turn: Quat,
    scale: f32,
    colour: usize,
    shutters: usize,
    /// The hill stands against the back wall's ground floor, so that floor has no windows.
    banked: bool,
}

impl Placed {
    /// Where a point of the plan comes to on the ground: `x` across from the front's centre, and
    /// `behind` metres back from it.
    fn on_plan(&self, x: f32, behind: f32) -> Vec2 {
        let point = self.at + self.turn * (Vec3::new(x, 0.0, -behind) * self.scale);
        Vec2::new(point.x, point.z)
    }

    /// How far the ground there stands above the house's floor.
    fn ground_above_floor(&self, x: f32, behind: f32) -> f32 {
        let point = self.on_plan(x, behind);
        ground::height(point.x, point.y) - self.at.y
    }
}

/// Every house in the town, standing with its door at street level.
fn layout() -> Vec<Placed> {
    let mut houses = Vec::new();
    for (index, block) in BLOCKS.iter().enumerate() {
        let (length, _) = block.length_and_depth();
        let (chosen, scale) = fill(length, block.plans(), index * 7);
        let turn = block.turn();
        let mut along = 0.0;
        for (i, &plan) in chosen.iter().enumerate() {
            let width = PLANS[plan].width * scale;
            let front = block.front_point(along + width * 0.5);
            let details = &PLANS[plan];
            let mut house = Placed {
                plan,
                at: Vec3::new(front.x, 0.0, front.y),
                turn,
                scale,
                // Limewash and ochre about equally, and a paler mix less often. Shutters step
                // along the street, so neighbours' differ.
                colour: (index * 5 + i * 3) % 7 / 3,
                shutters: (index + i) % 3,
                banked: false,
            };
            // The floor is level with the higher side of the doorway; the step covers the rest.
            let reach = DOOR_WIDTH * 0.5 + SURROUND;
            house.at.y = [-reach, reach]
                .map(|side| house.ground_above_floor(details.door_x() + side, 0.0))
                .into_iter()
                .fold(f32::MIN, f32::max);
            let half = details.width * 0.5;
            house.banked = [-half, 0.0, half].into_iter().any(|x| {
                house.ground_above_floor(x, details.depth)
                    > (SILL_HEIGHT - SILL_STONE) * house.scale - 0.05
            });
            houses.push(house);
            along += width;
        }
    }
    houses
}

/// The file of houses, built from the textures under `scans`.
pub fn file(scans: &Path, parts: Parts) -> io::Result<File> {
    let map = HeightMap::load(
        &scans
            .join(ROOF.surface)
            .join(format!("{}_disp.png", ROOF.surface)),
    )?;
    let mut file = File::default();
    let walls: Vec<usize> = WALL_TINTS
        .iter()
        .map(|&tint| file.textured(&Textured { tint, ..STUCCO }))
        .collect();
    let shutters: Vec<usize> = SHUTTER_TINTS
        .iter()
        .map(|&tint| file.textured(&Textured { tint, ..PLANKS }))
        .collect();
    let (roof, wood, trim) = (
        file.textured(&ROOF),
        file.textured(&PLANKS),
        file.textured(&TRIM),
    );
    // White paint, so that the openings read from across the harbour: the planks are too dark.
    let frames = file.material(&Plain {
        name: "window_paint",
        color: [0.72, 0.7, 0.64],
        roughness: 0.5,
    });
    let glass = file.material(&Plain {
        name: "window_glass",
        color: [0.02, 0.025, 0.03],
        roughness: 0.08,
    });
    let iron = file.material(&Plain {
        name: "wrought_iron",
        color: [0.025, 0.025, 0.025],
        roughness: 0.55,
    });

    struct Meshes {
        walls: Vec<Option<usize>>,
        shutters: Vec<Option<usize>>,
        details: Option<usize>,
    }
    let houses = layout();
    let roofs: Vec<Option<usize>> = PLANS
        .iter()
        .map(|plan| {
            parts
                .roofs
                .then(|| file.mesh(&[(&slope(plan, &map), roof)]))
                .flatten()
        })
        .collect();
    // Each plan's body, built only in the versions some house uses: without and with a bank.
    let mut bodies: Vec<[Option<Meshes>; 2]> = Vec::new();
    for (index, plan) in PLANS.iter().enumerate() {
        let mut versions = [None, None];
        for banked in [false, true] {
            if !houses.iter().any(|h| h.plan == index && h.banked == banked) {
                continue;
            }
            let built = build(plan, banked);
            let mut coloured = |geometry: &Geometry, materials: &[usize]| -> Vec<Option<usize>> {
                materials
                    .iter()
                    .map(|&m| file.mesh(&[(geometry, m)]))
                    .collect()
            };
            let walls = if parts.walls {
                coloured(&built.walls, &walls)
            } else {
                Vec::new()
            };
            let shutter_meshes = if parts.details {
                coloured(&built.shutters, &shutters)
            } else {
                Vec::new()
            };
            let details = parts.details.then(|| {
                file.mesh(&[
                    (&built.trim, trim),
                    (&built.wood, wood),
                    (&built.frames, frames),
                    (&built.glass, glass),
                    (&built.iron, iron),
                    (&built.ridge, roof),
                ])
            });
            versions[usize::from(banked)] = Some(Meshes {
                walls,
                shutters: shutter_meshes,
                details: details.flatten(),
            });
        }
        bodies.push(versions);
    }

    for house in &houses {
        let set = bodies[house.plan][usize::from(house.banked)]
            .as_ref()
            .expect("every version a house uses is built");
        let plan = &PLANS[house.plan];
        let mut place = |mesh: Option<&Option<usize>>| {
            if let Some(Some(mesh)) = mesh {
                file.node(*mesh, house.at, house.turn, house.scale);
            }
        };
        place(set.walls.get(house.colour));
        place(set.shutters.get(house.shutters));
        place(Some(&set.details));
        if let Some(mesh) = roofs[house.plan] {
            let ridge = Vec3::new(0.0, plan.ridge(), -plan.depth * 0.5) * house.scale;
            let at = house.at + house.turn * ridge;
            file.node(mesh, at, house.turn, house.scale);
            let back = house.turn * Quat::from_rotation_y(std::f32::consts::PI);
            file.node(mesh, at, back, house.scale);
        }
    }
    Ok(file)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_block_fills_its_street_without_stretching_a_house_much() {
        for (index, block) in BLOCKS.iter().enumerate() {
            let (length, _) = block.length_and_depth();
            let (chosen, scale) = fill(length, block.plans(), index * 7);
            assert!((0.88..=1.14).contains(&scale), "block {index}: {scale}");
            let total: f32 = chosen.iter().map(|&p| PLANS[p].width * scale).sum();
            assert!((total - length).abs() < 1.0e-3);
        }
    }

    /// What a house covers on the ground, as (min, max).
    fn footprint(house: &Placed) -> (Vec2, Vec2) {
        let plan = &PLANS[house.plan];
        let half = plan.width * 0.5;
        let corners = [
            (-half, 0.0),
            (half, 0.0),
            (-half, plan.depth),
            (half, plan.depth),
        ]
        .map(|(x, behind)| house.on_plan(x, behind));
        let first = (corners[0], corners[0]);
        corners
            .into_iter()
            .fold(first, |(low, high), c| (low.min(c), high.max(c)))
    }

    fn overlaps(a: (Vec2, Vec2), b: (Vec2, Vec2)) -> bool {
        const TOUCHING: f32 = 0.01;
        a.0.x < b.1.x - TOUCHING
            && b.0.x < a.1.x - TOUCHING
            && a.0.y < b.1.y - TOUCHING
            && b.0.y < a.1.y - TOUCHING
    }

    #[test]
    fn no_house_stands_on_a_street_or_on_another_house() {
        const STREETS: [(&str, [f32; 4]); 5] = [
            ("Net Lane", [11.0, -31.6, 120.0, -28.4]),
            ("the passage", [11.0, -11.6, 23.0, -8.4]),
            ("Chapel Steps", [11.0, 23.7, 120.0, 26.3]),
            ("Upper Street", [68.0, -34.0, 72.0, 29.0]),
            ("the square", [24.0, -14.0, 56.0, 16.0]),
        ];
        let houses = layout();
        for (i, house) in houses.iter().enumerate() {
            let covers = footprint(house);
            for (name, [x0, z0, x1, z1]) in STREETS {
                let street = (Vec2::new(x0, z0), Vec2::new(x1, z1));
                assert!(!overlaps(covers, street), "house {i} is on {name}");
            }
            for (j, other) in houses.iter().enumerate().skip(i + 1) {
                assert!(!overlaps(covers, footprint(other)), "houses {i} and {j}");
            }
        }
    }

    #[test]
    fn every_door_meets_the_street_and_every_foundation_is_buried() {
        for (i, house) in layout().iter().enumerate() {
            let plan = &PLANS[house.plan];
            let reach = DOOR_WIDTH * 0.5 + SURROUND;
            for side in [-reach, reach] {
                let below = -house.ground_above_floor(plan.door_x() + side, 0.0);
                // The door's step runs 0.4 m down.
                assert!((-1.0e-3..=0.35).contains(&below), "house {i}: {below}");
            }
            // At the middle of the door, the rise is one the player can walk up.
            let centre = -house.ground_above_floor(plan.door_x(), 0.0);
            assert!(centre <= 0.17, "house {i}: {centre}");
            let half = plan.width * 0.5;
            for (x, behind) in [
                (-half, 0.0),
                (half, 0.0),
                (-half, plan.depth),
                (half, plan.depth),
            ] {
                let rise = house.ground_above_floor(x, behind);
                assert!(rise > -SKIRT * house.scale + 0.5, "house {i}: {rise}");
            }
        }
    }

    #[test]
    fn the_ground_never_reaches_a_window_sill() {
        for (i, house) in layout().iter().enumerate() {
            let plan = &PLANS[house.plan];
            let half = plan.width * 0.5;
            let check = |o: &Opening, x: f32, behind: f32| {
                let rise = house.ground_above_floor(x, behind);
                let sill = (o.y.0 - SILL_STONE) * house.scale;
                assert!(rise < sill, "house {i}: ground {rise}, sill {sill}");
            };
            for (o, kind) in plan.front_openings() {
                if kind != Kind::Door {
                    check(&o, o.x.0 - half, 0.0);
                    check(&o, o.x.1 - half, 0.0);
                }
            }
            for (o, _) in plan.rear_openings(house.banked) {
                check(&o, half - o.x.0, plan.depth);
                check(&o, half - o.x.1, plan.depth);
            }
        }
    }

    #[test]
    fn nothing_of_stone_crosses_a_doorway() {
        let inside = |a: Vec3, b: Vec3, c: Vec3, p: Vec2| {
            let side = |u: Vec3, v: Vec3| (v.x - u.x) * (p.y - u.y) - (v.y - u.y) * (p.x - u.x);
            let (d0, d1, d2) = (side(a, b), side(b, c), side(c, a));
            (d0 >= 0.0 && d1 >= 0.0 && d2 >= 0.0) || (d0 <= 0.0 && d1 <= 0.0 && d2 <= 0.0)
        };
        for plan in &PLANS {
            let built = build(plan, false);
            let door = plan.door_x();
            for height in [0.2, 1.0, DOOR_HEIGHT - 0.05] {
                let point = Vec2::new(door, height);
                for t in built.trim.indices.chunks_exact(3) {
                    let [a, b, c] = [0, 1, 2].map(|i| built.trim.positions[t[i] as usize]);
                    let in_front = a.z > 0.0 && b.z > 0.0 && c.z > 0.0;
                    assert!(!(in_front && inside(a, b, c, point)), "at {height}");
                }
            }
        }
    }

    #[test]
    fn doors_stay_wide_and_tall_enough_at_the_smallest_scale() {
        // The player needs 0.9 by 2.1 m.
        let smallest = BLOCKS
            .iter()
            .enumerate()
            .map(|(i, b)| fill(b.length_and_depth().0, b.plans(), i * 7).1)
            .fold(f32::MAX, f32::min);
        assert!(DOOR_WIDTH * smallest >= 0.9, "{smallest}");
        assert!(DOOR_HEIGHT * smallest >= 2.1, "{smallest}");
    }

    #[test]
    fn every_opening_fits_inside_its_wall_and_below_the_eaves() {
        for plan in &PLANS {
            let rear = plan.rear_openings(false);
            for (o, _) in plan.front_openings().iter().chain(&rear) {
                assert!(o.x.0 > 0.0 && o.x.1 < plan.width, "{:?}", o.x);
                // A lintel stands 0.2 m above its opening, and the cornice starts 0.42 below the eaves.
                assert!(o.y.1 + 0.2 <= plan.eaves() - 0.42 + 1.0e-3, "{:?}", o.y);
            }
        }
    }

    #[test]
    fn a_wall_covers_its_area_less_its_openings() {
        let face = Face {
            origin: Vec3::ZERO,
            right: Vec3::X,
            out: Vec3::Z,
        };
        let opening = Opening {
            x: (1.0, 2.0),
            y: (1.0, 3.0),
        };
        let mut g = Geometry::default();
        wall(&mut g, &face, 4.0, 5.0, &[opening]);
        let front_area: f32 = g
            .indices
            .chunks_exact(3)
            .filter(|t| g.normals[t[0] as usize].z > 0.5)
            .map(|t| {
                let [a, b, c] = [0, 1, 2].map(|i| g.positions[t[i] as usize]);
                (b - a).cross(c - a).length() * 0.5
            })
            .sum();
        assert!(
            (front_area - (4.0 * (5.0 + SKIRT) - 2.0)).abs() < 1.0e-3,
            "{front_area}"
        );
    }

    #[test]
    fn a_plan_has_shutters_on_every_window_or_on_none() {
        for (index, plan) in PLANS.iter().enumerate() {
            let built = build(plan, false);
            let shuttered = plan
                .front_openings()
                .iter()
                .filter(|(_, kind)| *kind == Kind::Shuttered)
                .count();
            // Two leaves of one cuboid each.
            let expected = plan.shutter_leaf().map_or(0, |_| shuttered * 2 * 24);
            assert_eq!(built.shutters.positions.len(), expected, "plan {index}");
        }
        assert!(PLANS.iter().filter(|p| p.shutter_leaf().is_some()).count() >= 5);
    }

    #[test]
    fn the_town_has_about_seventy_houses() {
        let count = layout().len();
        assert!((60..=80).contains(&count), "{count}");
    }
}
