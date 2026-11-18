//! Dressed stone: the quay's and mole's walls, paving and copings.
//!
//! Each surface is a tile displaced from its texture's height map, instanced along the stonework,
//! so that the relief is real geometry and costs one tile however far it runs. The tiles sit on
//! the plain solids in the `town` file, which stay simple for collision. Vertices keep the flat
//! surface's normal: the texture's normal map, which shows the same stones, does the shading.

use crate::geometry::Geometry;
use crate::gltf::{File, Plain, Textured};
use crate::ground;
use crate::town::{
    self, MolePath, BASIN_FACE, HEAD_CENTRE, HEAD_FOOT_RADIUS, HEAD_TOP_RADIUS, MOLE_DECK,
    MOLE_START, MOLE_STEPS_END, PARAPET_INNER, PARAPET_TOP, SEA_FACE,
};
use glam::{Mat3, Quat, Vec2, Vec3};
use std::io;
use std::path::Path;

// The scans are of warmer stone than this coast's grey granite, so most are tinted towards it.
/// Large worn ashlar, for walls.
const COURSES: Textured = Textured {
    surface: "stone_wall_02",
    metres: 2.0,
    arm: false,
    tint: [0.7, 0.76, 0.88],
};
/// Granite setts, for the quay's deck.
const SETTS: Textured = Textured {
    surface: "cobblestone_floor_07",
    metres: 2.0,
    arm: false,
    tint: [1.0, 1.0, 1.0],
};
/// Large flags, for the mole's deck, copings and steps.
pub const FLAGS: Textured = Textured {
    surface: "granite_tile_04",
    metres: 2.0,
    arm: true,
    tint: [0.85, 0.88, 0.94],
};
/// The market square's paving.
const SQUARE: Textured = Textured {
    surface: "square_cobblestone",
    metres: 2.0,
    arm: true,
    tint: [0.78, 0.82, 0.86],
};

/// How deep each surface's relief is, in metres.
const COURSES_DEPTH: f32 = 0.05;
/// How far paving lifts off the solid under it, so that the two never share a surface.
const LIFT: f32 = 0.012;
const SETTS_DEPTH: f32 = 0.035;
const FLAGS_DEPTH: f32 = 0.008;
const SQUARE_DEPTH: f32 = 0.03;
/// The spacing of a tile's vertices.
const WALL_GRID: f32 = 0.03;
const PAVING_GRID: f32 = 0.025;
/// The round wall of the mole's head is one mesh rather than tiles, so it is a little coarser.
const HEAD_GRID: f32 = 0.04;

/// The top of the walls, under the copings that cap them.
const COPING_HEIGHT: f32 = 0.28;
/// How far copings stand proud of the solid under them, so that the two never share a surface.
const COPING_RISE: f32 = 0.06;
const COPING_LENGTH: f32 = 1.5;

/// A texture's height map, repeating.
pub struct HeightMap {
    size: usize,
    heights: Vec<f32>,
}

impl HeightMap {
    /// Reads a grey, RGB or RGBA PNG of 8 or 16 bits.
    pub fn load(path: &Path) -> io::Result<Self> {
        let failed = |e: String| {
            io::Error::other(format!(
                "{}: {e} (run: python scripts/fetch-scans.py scenes/harbour.surfaces 2k)",
                path.display()
            ))
        };
        let file = std::fs::File::open(path).map_err(|e| failed(e.to_string()))?;
        let mut reader = png::Decoder::new(file)
            .read_info()
            .map_err(|e| failed(e.to_string()))?;
        let mut buffer = vec![0; reader.output_buffer_size()];
        let info = reader
            .next_frame(&mut buffer)
            .map_err(|e| failed(e.to_string()))?;
        if info.width != info.height {
            return Err(failed("not square".to_string()));
        }
        let bytes = match info.bit_depth {
            png::BitDepth::Eight => 1,
            png::BitDepth::Sixteen => 2,
            other => return Err(failed(format!("{other:?} bits per channel"))),
        };
        let stride = info.color_type.samples() * bytes;
        let heights = buffer[..info.buffer_size()]
            .chunks_exact(stride)
            .map(|texel| {
                if bytes == 2 {
                    f32::from(u16::from_be_bytes([texel[0], texel[1]])) / 65535.0
                } else {
                    f32::from(texel[0]) / 255.0
                }
            })
            .collect();
        Ok(Self {
            size: info.width as usize,
            heights,
        })
    }

    /// The height at a texture coordinate, from 0 to 1, repeating. Bilinear.
    fn sample(&self, u: f32, v: f32) -> f32 {
        let (x, y) = (
            u.rem_euclid(1.0) * self.size as f32,
            v.rem_euclid(1.0) * self.size as f32,
        );
        let (ix, iy) = (x.floor() as usize, y.floor() as usize);
        let (fx, fy) = (x - ix as f32, y - iy as f32);
        let at = |i: usize, j: usize| self.heights[(j % self.size) * self.size + i % self.size];
        let top = at(ix, iy) * (1.0 - fx) + at(ix + 1, iy) * fx;
        let bottom = at(ix, iy + 1) * (1.0 - fx) + at(ix + 1, iy + 1) * fx;
        top * (1.0 - fy) + bottom * fy
    }
}

/// A grid of vertices, displaced along `normal_at` by the map at each point's texture coordinate.
/// `at` takes a column and row (either may run one step over) and gives the point, its normal and
/// its UV in metres. Viewed from outside, columns must run towards the right of rows.
pub fn sheet(
    columns: u32,
    rows: u32,
    map: &HeightMap,
    metres: f32,
    depth: f32,
    at: impl Fn(u32, u32) -> (Vec3, Vec3, Vec2),
) -> Geometry {
    let mut g = Geometry::default();
    for j in 0..=rows {
        for i in 0..=columns {
            let (position, normal, uv) = at(i, j);
            let height = map.sample(uv.x / metres, uv.y / metres);
            g.positions.push(position + normal * height * depth);
            g.normals.push(normal);
            g.uvs.push(uv);
        }
    }
    let corner = |i: u32, j: u32| j * (columns + 1) + i;
    for j in 0..rows {
        for i in 0..columns {
            let (a, b, c, d) = (
                corner(i, j),
                corner(i + 1, j),
                corner(i, j + 1),
                corner(i + 1, j + 1),
            );
            g.indices.extend([a, c, b, b, c, d]);
        }
    }
    g
}

/// One repeat of a texture as a square, centred on the origin and facing +Y. Its edges match its
/// neighbours', since the height map repeats.
fn tile(map: &HeightMap, texture: &Textured, depth: f32, grid: f32) -> Geometry {
    let cells = (texture.metres / grid).round() as u32;
    let step = texture.metres / cells as f32;
    let half = texture.metres * 0.5;
    sheet(cells, cells, map, texture.metres, depth, |i, j| {
        let uv = Vec2::new(i as f32 * step, j as f32 * step);
        (Vec3::new(uv.x - half, 0.0, uv.y - half), Vec3::Y, uv)
    })
}

/// A block with its edges taken off, `length` along X, its top face at Y = 0. Blocks that sit side
/// by side then read as separate stones.
fn block(length: f32, width: f32, height: f32) -> Geometry {
    const CHAMFER: f32 = 0.025;
    let ring = |x: f32, inset: f32| {
        let (w, top) = (width * 0.5 - inset, -inset);
        [
            (-w, -height),
            (w, -height),
            (w, top - CHAMFER),
            (w - CHAMFER, top),
            (-w + CHAMFER, top),
            (-w, top - CHAMFER),
        ]
        .map(|(z, y)| Vec3::new(x, y, z))
        .to_vec()
    };
    let half = length * 0.5;
    let mut g = Geometry::default();
    g.loft(&[
        ring(-half, CHAMFER),
        ring(-half + CHAMFER, 0.0),
        ring(half - CHAMFER, 0.0),
        ring(half, CHAMFER),
    ]);
    g
}

/// A surface's axes as a rotation from a tile, whose X runs along the surface and whose Y faces out.
fn facing(along: Vec3, outward: Vec3) -> Quat {
    let along = along.normalize();
    let z = along.cross(outward).normalize();
    let y = z.cross(along);
    Quat::from_mat3(&Mat3::from_cols(along, y, z))
}

/// How many tiles of `size` fit in `length`, and the scale that makes them fill it exactly.
fn fit(length: f32, size: f32) -> (u32, f32) {
    let count = (length / size).round().max(1.0);
    (count as u32, length / count / size)
}

/// Lays `rows` of tiles down a wall face from `top`, running `length` in the direction `along`.
/// `up` leans with the face, and a wall's outward side is `along × up`.
fn wall(file: &mut File, mesh: usize, face: (Vec3, Vec3, Vec3), length: f32, rows: u32) {
    let (top, along, up) = face;
    let (count, scale) = fit(length, COURSES.metres);
    let size = COURSES.metres * scale;
    let rotation = facing(along, along.cross(up));
    for row in 0..rows {
        for i in 0..count {
            let at = top + along * size * (i as f32 + 0.5) - up * size * (row as f32 + 0.5);
            file.node(mesh, at, rotation, scale);
        }
    }
}

/// Paves a rectangle at a constant height, with `columns` tiles across X.
fn paving(file: &mut File, mesh: usize, metres: f32, from: Vec2, to: Vec2, columns: u32, y: f32) {
    let (count, scale) = fit(to.y - from.y, metres);
    let size = metres * scale;
    for i in 0..count {
        for j in 0..columns {
            let at = Vec3::new(
                from.x + size * (j as f32 + 0.5),
                y,
                from.y + size * (i as f32 + 0.5),
            );
            file.node(mesh, at, Quat::IDENTITY, scale);
        }
    }
}

/// A row of coping stones. Each block's top face is at `top`, and its length runs along the line.
fn coping_run(file: &mut File, stones: &[usize], from: Vec3, to: Vec3) {
    let line = to - from;
    let (count, scale) = fit(line.length(), COPING_LENGTH);
    let direction = line.normalize();
    let rotation = Quat::from_rotation_arc(Vec3::X, direction);
    for i in 0..count {
        let at = from + direction * (COPING_LENGTH * scale * (i as f32 + 0.5));
        file.node(stones[i as usize % stones.len()], at, rotation, scale);
    }
}

fn quay(file: &mut File, setts: usize, courses: usize, stones: &[usize]) {
    use town::{STAIRS_WIDTH, STAIRS_Z, STAIR_GOING, STAIR_STEPS};
    const NORTH: f32 = -52.0;
    const SOUTH: f32 = 50.0;
    const PIER_LANDING: (f32, f32) = (38.5, 41.5);
    /// How far the deck's setts run inland: to the foot of the front row of houses.
    const INLAND: f32 = 11.0;
    const COPING_WIDTH: f32 = 0.66;
    let deck = ground::QUAY_DECK;
    let (gap_north, gap_south) = (STAIRS_Z - STAIRS_WIDTH * 0.5, STAIRS_Z + STAIRS_WIDTH * 0.5);
    let edge = COPING_WIDTH - 0.06;

    for (from, to) in [(NORTH, gap_north), (gap_south, SOUTH)] {
        let scale = fit(to - from, SETTS.metres).1;
        // Running on under the front row's walls, and into the mouths of the lanes.
        let columns = ((INLAND - edge) / (SETTS.metres * scale)).ceil() as u32;
        paving(
            file,
            setts,
            SETTS.metres,
            Vec2::new(edge, from),
            Vec2::new(edge, to),
            columns,
            deck + LIFT,
        );
    }
    // Behind the head of the water stairs.
    let back = STAIR_GOING * STAIR_STEPS as f32;
    paving(
        file,
        setts,
        SETTS.metres,
        Vec2::new(back, gap_north),
        Vec2::new(back, gap_south),
        4,
        deck + LIFT,
    );

    // The harbour face, leaning back as it rises, in two lengths either side of the stairs.
    let lean = Vec3::new(0.6, ground::QUAY_DECK - town::FOOTING, 0.0).normalize();
    let top_y = deck - COPING_HEIGHT + COPING_RISE;
    let face_x = -0.6 * (deck - top_y) / (deck - town::FOOTING);
    for (from, to) in [(NORTH, gap_north), (gap_south, SOUTH)] {
        wall(
            file,
            courses,
            (Vec3::new(face_x, top_y, from), Vec3::Z, lean),
            to - from,
            3,
        );
    }
    // The copings stop either side of the stairs, and of the landing for the pier.
    let line = |z: f32| Vec3::new(COPING_WIDTH * 0.5 - 0.06, deck + COPING_RISE, z);
    for (from, to) in [
        (NORTH, gap_north),
        (gap_south, PIER_LANDING.0),
        (PIER_LANDING.1, SOUTH),
    ] {
        coping_run(file, stones, line(from), line(to));
    }
    // The south end, seen from the beach.
    wall(
        file,
        courses,
        (Vec3::new(0.0, deck, SOUTH), Vec3::X, Vec3::Y),
        10.0,
        2,
    );
}

fn mole(file: &mut File, flags: usize, courses: usize, stones: &[usize], head: usize) {
    const COPING_WIDTH: f32 = 0.7;
    let path = MolePath::new();
    let at_3d = |p: Vec2, offset: Vec2, y: f32| {
        let q = p + offset;
        Vec3::new(q.x, y, q.y)
    };
    let flat = |v: Vec2| Vec3::new(v.x, 0.0, v.y);
    // The straight run stops inside the round head, whose edge the walls run into.
    let end = path.length() - HEAD_TOP_RADIUS + 1.2;

    // The deck, between the basin side's coping and the parapet.
    let paved = (BASIN_FACE[0].0 + COPING_WIDTH, PARAPET_INNER);
    let (columns, scale) = fit(paved.1 - paved.0, FLAGS.metres);
    let size = FLAGS.metres * scale;
    let (lengths, stretch) = fit(end - MOLE_STEPS_END, size);
    let stride = size * stretch;
    for i in 0..lengths {
        let (p, heading, seaward) = path.at(MOLE_STEPS_END + stride * (i as f32 + 0.5));
        for j in 0..columns {
            let across = paved.0 + size * (j as f32 + 0.5);
            let at = at_3d(p, seaward * across, MOLE_DECK + LIFT);
            let rotation = facing(flat(heading), Vec3::Y);
            file.node(flags, at, rotation, scale * stretch.max(1.0));
        }
    }

    // Each face leans out as it falls, and runs along the curve.
    // The basin side starts past the steps, where the solid behind it reaches the deck.
    let faces = [
        (SEA_FACE, 1.0, 4, MOLE_START),
        (BASIN_FACE, -1.0, 3, MOLE_STEPS_END),
    ];
    for (face, side, rows, start) in faces {
        let (count, scale) = fit(end - start, COURSES.metres);
        let size = COURSES.metres * scale;
        let [(top_offset, top_y), (foot_offset, foot_y)] = face;
        let slope = (foot_offset - top_offset) / (top_y - foot_y);
        let start_y = top_y + COPING_RISE - COPING_HEIGHT;
        for i in 0..count {
            let (p, heading, seaward) = path.at(start + size * (i as f32 + 0.5));
            let up = (Vec3::Y - flat(seaward) * slope).normalize();
            // A wall's outward side is `along × up`, and seaward lies to the right of the heading.
            let along = flat(heading) * side;
            let rotation = facing(along, along.cross(up));
            for row in 0..rows {
                let y = start_y - size * (row as f32 + 0.5);
                let offset = top_offset + (top_y - y) * slope;
                file.node(courses, at_3d(p, seaward * offset, y), rotation, scale);
            }
        }
    }

    // The copings: the basin side's edge and the top of the parapet.
    let lines = [
        (
            (BASIN_FACE[0].0 + COPING_WIDTH * 0.5 - 0.05),
            MOLE_DECK + COPING_RISE,
            MOLE_STEPS_END,
        ),
        (
            (SEA_FACE[0].0 + PARAPET_INNER) * 0.5,
            PARAPET_TOP + COPING_RISE,
            MOLE_START,
        ),
    ];
    for (offset, top, start) in lines {
        let (count, scale) = fit(end - start, COPING_LENGTH);
        let stride = COPING_LENGTH * scale;
        for i in 0..count {
            let (p, heading, seaward) = path.at(start + stride * (i as f32 + 0.5));
            let rotation = Quat::from_rotation_arc(Vec3::X, flat(heading));
            let at = at_3d(p, seaward * offset, top);
            file.node(stones[i as usize % stones.len()], at, rotation, scale);
        }
    }

    // The round head's wall, and its ring of copings.
    file.node(head, Vec3::ZERO, Quat::IDENTITY, 1.0);
    let centre = Vec3::new(HEAD_CENTRE.x, MOLE_DECK + COPING_RISE, HEAD_CENTRE.z);
    let radius = HEAD_TOP_RADIUS - 0.3;
    let circumference = std::f32::consts::TAU * radius;
    let (count, scale) = fit(circumference, COPING_LENGTH);
    for i in 0..count {
        let angle = i as f32 / count as f32 * std::f32::consts::TAU;
        let radial = Vec3::new(angle.cos(), 0.0, angle.sin());
        let rotation = Quat::from_rotation_arc(Vec3::X, Vec3::new(-radial.z, 0.0, radial.x));
        let stone = stones[i as usize % stones.len()];
        file.node(stone, centre + radial * radius, rotation, scale);
    }
}

/// The round wall of the mole's head, as one mesh in world space.
fn head_wall(map: &HeightMap) -> Geometry {
    let radius_at = |y: f32| {
        HEAD_TOP_RADIUS
            + (HEAD_FOOT_RADIUS - HEAD_TOP_RADIUS) * (MOLE_DECK - y) / (MOLE_DECK - town::FOOTING)
    };
    /// A little proud of the solid's many-sided cone, so that its flat sides do not show through.
    const PROUD: f32 = 0.06;
    let tiles = (std::f32::consts::TAU * radius_at(0.0) / COURSES.metres).round();
    let tile_width = std::f32::consts::TAU * radius_at(0.0) / tiles;
    let columns = (std::f32::consts::TAU * radius_at(0.0) / HEAD_GRID).round() as u32;
    let top = MOLE_DECK - COPING_HEIGHT + COPING_RISE;
    let rows = (6.5 / HEAD_GRID).round() as u32;
    sheet(columns, rows, map, COURSES.metres, COURSES_DEPTH, |i, j| {
        // Decreasing with the column, so that the wall faces outward.
        let angle = -(i as f32 / columns as f32) * std::f32::consts::TAU;
        let y = top - j as f32 * HEAD_GRID;
        let radial = Vec3::new(angle.cos(), 0.0, angle.sin());
        let radius = radius_at(y) + PROUD;
        let slope = (HEAD_FOOT_RADIUS - HEAD_TOP_RADIUS) / (MOLE_DECK - town::FOOTING);
        let position = Vec3::new(HEAD_CENTRE.x, y, HEAD_CENTRE.z) + radial * radius;
        let uv = Vec2::new(
            i as f32 / columns as f32 * tiles * COURSES.metres,
            j as f32 * HEAD_GRID * COURSES.metres / tile_width,
        );
        (position, (radial + Vec3::Y * slope).normalize(), uv)
    })
}

/// The market square's paving, laid on the ground's fall towards the sea.
fn square(file: &mut File, mesh: usize) {
    // 16 by 17 tiles, stopping where the ground's fall ends at x = 56.
    const ACROSS: u32 = 16;
    const DOWN: u32 = 17;
    let origin = Vec2::new(24.0, -17.0);
    let slope = 1.0 / 60.0;
    let along = Vec3::new(1.0, slope, 0.0).normalize();
    let rotation = facing(along, Vec3::new(-slope, 1.0, 0.0).normalize());
    let run = SQUARE.metres / along.x;
    for j in 0..DOWN {
        for i in 0..ACROSS {
            let x = origin.x + run * (i as f32 + 0.5);
            let z = origin.y + SQUARE.metres * (j as f32 + 0.5);
            let at = Vec3::new(x, ground::height(x, z) + LIFT, z);
            file.node(mesh, at, rotation, 1.0);
        }
    }
}

/// A cast-iron mooring bollard, standing on the origin.
fn mesh_iron(file: &mut File) -> usize {
    let material = file.material(&Plain {
        name: "cast_iron",
        color: [0.03, 0.03, 0.03],
        roughness: 0.55,
    });
    let mut g = Geometry::default();
    g.frustum(Vec3::ZERO, 0.17, 0.13, 0.5, 20);
    g.frustum(Vec3::Y * 0.5, 0.13, 0.19, 0.08, 20);
    g.frustum(Vec3::Y * 0.58, 0.19, 0.16, 0.05, 20);
    file.mesh(&[(&g, material)]).expect("it has triangles")
}

/// Bollards along the quay's edge, where boats tie up, clear of the water stairs.
fn bollards(file: &mut File, mesh: usize) {
    const SPACING: f32 = 12.0;
    let mut z = -46.0;
    while z < 50.0 {
        if (z - town::STAIRS_Z).abs() > town::STAIRS_WIDTH {
            file.node(
                mesh,
                Vec3::new(1.1, ground::QUAY_DECK, z),
                Quat::IDENTITY,
                1.0,
            );
        }
        z += SPACING;
    }
}

/// The file of dressed stone, built from the textures under `scans`.
pub fn file(scans: &Path) -> io::Result<File> {
    let load = |texture: &Textured| {
        HeightMap::load(
            &scans
                .join(texture.surface)
                .join(format!("{}_disp.png", texture.surface)),
        )
    };
    let (courses_map, setts_map, flags_map, square_map) = (
        load(&COURSES)?,
        load(&SETTS)?,
        load(&FLAGS)?,
        load(&SQUARE)?,
    );

    let mut file = File::default();
    let materials = [&COURSES, &SETTS, &FLAGS, &SQUARE].map(|t| file.textured(t));
    let [courses, setts, flags, paved] = materials;
    let mut mesh = |geometry: &Geometry, material: usize| {
        file.mesh(&[(geometry, material)])
            .expect("every tile has triangles")
    };
    let wall_tile = mesh(
        &tile(&courses_map, &COURSES, COURSES_DEPTH, WALL_GRID),
        courses,
    );
    let sett_tile = mesh(&tile(&setts_map, &SETTS, SETTS_DEPTH, PAVING_GRID), setts);
    let flag_tile = mesh(&tile(&flags_map, &FLAGS, FLAGS_DEPTH, PAVING_GRID), flags);
    let square_tile = mesh(
        &tile(&square_map, &SQUARE, SQUARE_DEPTH, PAVING_GRID),
        paved,
    );
    let head = mesh(&head_wall(&courses_map), courses);
    // Three cuts of stone, so that the row of copings is not one repeated block.
    let stones: Vec<usize> = [(0.34, 0.3), (0.33, 0.28), (0.35, 0.32)]
        .map(|(width, height)| mesh(&block(COPING_LENGTH, width * 2.0, height), flags))
        .to_vec();

    let iron = mesh_iron(&mut file);
    bollards(&mut file, iron);
    quay(&mut file, sett_tile, wall_tile, &stones);
    mole(&mut file, flag_tile, wall_tile, &stones, head);
    square(&mut file, square_tile);
    Ok(file)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slope_map() -> HeightMap {
        HeightMap {
            size: 2,
            heights: vec![0.0, 1.0, 0.0, 1.0],
        }
    }

    #[test]
    fn a_height_map_repeats() {
        let map = slope_map();
        assert_eq!(map.sample(0.0, 0.0), 0.0);
        assert_eq!(map.sample(1.0, 0.0), 0.0);
        assert_eq!(map.sample(0.5, 0.25), 1.0);
    }

    #[test]
    fn a_tiles_opposite_edges_match_so_neighbours_join_without_cracks() {
        let texture = Textured {
            surface: "test",
            metres: 2.0,
            arm: false,
            tint: [1.0; 3],
        };
        let g = tile(&slope_map(), &texture, 0.1, 0.25);
        let cells = 8;
        for row in 0..=cells {
            let left = g.positions[row * (cells + 1)];
            let right = g.positions[row * (cells + 1) + cells];
            assert!((left.y - right.y).abs() < 1.0e-5, "row {row}");
            assert!((right.x - left.x - 2.0).abs() < 1.0e-5);
        }
    }

    #[test]
    fn a_tile_faces_up() {
        let texture = Textured {
            surface: "test",
            metres: 2.0,
            arm: false,
            tint: [1.0; 3],
        };
        let g = tile(&slope_map(), &texture, 0.0, 1.0);
        let [a, b, c] = [0, 1, 2].map(|i| g.positions[g.indices[i] as usize]);
        assert!((b - a).cross(c - a).y > 0.0);
    }

    #[test]
    fn fitting_fills_the_length_exactly() {
        let (count, scale) = fit(41.0, 2.0);
        assert!((count as f32 * 2.0 * scale - 41.0).abs() < 1.0e-4);
    }

    #[test]
    fn a_face_turns_a_tile_to_lie_along_it_and_face_out() {
        let rotation = facing(Vec3::Z, Vec3::NEG_X);
        assert!((rotation * Vec3::X).abs_diff_eq(Vec3::Z, 1.0e-5));
        assert!((rotation * Vec3::Y).abs_diff_eq(Vec3::NEG_X, 1.0e-5));
    }

    #[test]
    fn a_block_is_right_way_out() {
        let g = block(1.5, 0.6, 0.3);
        let volume: f32 = g
            .indices
            .chunks_exact(3)
            .map(|t| {
                let [a, b, c] = [0, 1, 2].map(|i| g.positions[t[i] as usize]);
                a.dot(b.cross(c)) / 6.0
            })
            .sum();
        assert!(volume > 0.2, "{volume}");
    }
}
