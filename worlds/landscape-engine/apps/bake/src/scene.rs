//! Lays out the demo scene: a ruined temple in a desert canyon.

use glam::{Mat4, Quat, Vec3};
use rayon::prelude::*;
use solstice_asset::{Instance, Light, Material, SceneAsset, Viewpoint};
use solstice_bake::meshes;
use solstice_builder::{build_virtual_mesh, BuildOptions, Mesh};
use solstice_procgen::noise::{fbm, random};
use solstice_procgen::surface::TILE_METRES;
use solstice_procgen::surfaces;

/// How much geometry to generate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Quality {
    /// Fast to bake; for iterating on the renderer.
    Draft,
    /// A sensible size to download in a browser.
    Web,
    /// The real scene, with meshes in the millions of triangles.
    High,
}

/// Material slots.
mod material {
    pub const SANDSTONE: u32 = 0;
    pub const MARBLE: u32 = 1;
    pub const POLISHED: u32 = 2;
    pub const GROUND: u32 = 3;
    pub const BASALT: u32 = 4;
    pub const FIRE: u32 = 5;
    pub const BRONZE: u32 = 6;
}

/// Mesh slots.
mod mesh {
    pub const TERRAIN: u32 = 0;
    pub const ROCK_A: u32 = 1;
    pub const ROCK_B: u32 = 2;
    pub const COLUMN: u32 = 3;
    pub const BLOCK_A: u32 = 4;
    pub const BLOCK_B: u32 = 5;
    pub const SPHERE: u32 = 6;
}

const COLUMN_HEIGHT: f32 = 7.5;
/// Half the footprint of the temple platform, in metres.
const TEMPLE_HALF: Vec3 = Vec3::new(10.5, 0.0, 19.5);
const PLATFORM_TOP: f32 = 1.2;

fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Ground height: rolling dunes, levelled around the temple, rising to canyon walls.
fn terrain_height(x: f32, z: f32) -> f32 {
    let p = Vec3::new(x, 0.0, z);
    let dunes = fbm(p * 0.012, 5, 201) * 5.5;
    let ripples = fbm(p * 0.09, 4, 203) * 0.45;
    let detail = fbm(p * 0.8, 4, 207) * 0.05;

    let dx = (x.abs() - TEMPLE_HALF.x).max(0.0);
    let dz = (z.abs() - TEMPLE_HALF.z).max(0.0);
    let plaza = 1.0 - smoothstep(4.0, 40.0, (dx * dx + dz * dz).sqrt());

    let radius = (x * x + z * z).sqrt();
    let canyon = smoothstep(95.0, 230.0, radius).powf(1.5) * 42.0;

    (dunes + ripples) * (1.0 - plaza) + detail + canyon
}

fn scale(x: f32, y: f32, z: f32) -> Vec3 {
    Vec3::new(x, y, z)
}

fn place(mesh: u32, material: u32, position: Vec3, rotation: Quat, scale: Vec3) -> Instance {
    Instance {
        mesh,
        material,
        transform: Mat4::from_scale_rotation_translation(scale, rotation, position),
    }
}

/// Builds every mesh, in the order of the `mesh` slots.
fn build_meshes(quality: Quality) -> Vec<Mesh> {
    let (terrain_cells, rock_subdiv, column_size, block_cells) = match quality {
        Quality::Draft => (256, 5, (128, 192), 48),
        Quality::Web => (640, 6, (360, 540), 96),
        Quality::High => (1024, 7, (640, 960), 144),
    };
    let generators: Vec<Box<dyn Fn() -> Mesh + Send + Sync>> = vec![
        Box::new(move || meshes::terrain(640.0, terrain_cells, terrain_height)),
        Box::new(move || meshes::rock(rock_subdiv, 11)),
        Box::new(move || meshes::rock(rock_subdiv, 47)),
        Box::new(move || meshes::column(COLUMN_HEIGHT, column_size.0, column_size.1, 5)),
        Box::new(move || meshes::stone_block(block_cells, 3)),
        Box::new(move || meshes::stone_block(block_cells, 29)),
        Box::new(|| meshes::sphere(4)),
    ];
    generators.par_iter().map(|generate| generate()).collect()
}

fn materials() -> Vec<Material> {
    let textured = |layer: i32, uv_scale: f32, roughness: f32, normal_strength: f32| Material {
        texture_layer: layer,
        uv_scale,
        roughness,
        normal_strength,
        base_color: [1.0; 4],

        ..Default::default()
    };
    // Surfaces span TILE_METRES, so this shows them at their real size.
    let real_size = 1.0 / TILE_METRES;
    vec![
        textured(0, real_size, 1.0, 1.0),
        Material {
            base_color: [0.74, 0.71, 0.66, 1.0],
            ..textured(1, real_size, 1.0, 1.0)
        },
        Material {
            base_color: [0.74, 0.71, 0.66, 1.0],
            // A multiplier on the marble's own polish, leaving this floor a little more worn.
            roughness: 0.75,
            ..textured(1, real_size, 1.0, 1.0)
        },
        // Only the ground spreads far enough for its repetition to show.
        Material {
            macro_variation: 1.0,
            ..textured(2, real_size, 1.0, 1.0)
        },
        textured(3, real_size, 1.0, 1.0),
        Material {
            base_color: [0.0, 0.0, 0.0, 1.0],
            emissive: [60.0, 21.0, 4.5],
            ..Default::default()
        },
        Material {
            base_color: [0.71, 0.45, 0.21, 1.0],
            roughness: 0.32,
            metallic: 1.0,
            ..Default::default()
        },
    ]
}

/// Lays stone courses along a straight run, with staggered joints and small offsets.
fn wall_run(
    out: &mut Vec<Instance>,
    start: Vec3,
    end: Vec3,
    base: f32,
    courses: u32,
    thickness: f32,
    seed: u32,
) {
    let length = start.distance(end);
    let along = (end - start).normalize();
    let yaw = Quat::from_rotation_y((-along.z).atan2(along.x));
    const COURSE_HEIGHT: f32 = 0.82;
    const BLOCK_LENGTH: f32 = 1.75;
    for course in 0..courses {
        let stagger = if course % 2 == 0 {
            0.0
        } else {
            BLOCK_LENGTH * 0.5
        };
        let mut distance = -stagger;
        let mut index = 0;
        while distance < length {
            let id = seed + course * 1000 + index;
            let block = BLOCK_LENGTH * (0.86 + random(id, 1) * 0.28);
            let begin = distance.max(0.0);
            let finish = (distance + block).min(length);
            if finish - begin > 0.12 {
                let middle = start + along * (begin + finish) * 0.5;
                let position = middle + Vec3::Y * (base + (course as f32 + 0.5) * COURSE_HEIGHT);
                let jitter = yaw * Vec3::new(0.0, 0.0, (random(id, 2) - 0.5) * 0.035);
                let mesh = if random(id, 3) > 0.5 {
                    mesh::BLOCK_A
                } else {
                    mesh::BLOCK_B
                };
                out.push(place(
                    mesh,
                    material::SANDSTONE,
                    position + jitter,
                    yaw,
                    scale(finish - begin, COURSE_HEIGHT * 0.99, thickness),
                ));
            }
            distance += block;
            index += 1;
        }
    }
}

fn temple(out: &mut Vec<Instance>, lights: &mut Vec<Light>) {
    // Stepped platform: three courses of big blocks, each inset from the one below.
    for (level, inset) in [(0u32, 0.0f32), (1, 0.7), (2, 1.4)] {
        let half_x = TEMPLE_HALF.x - inset;
        let half_z = TEMPLE_HALF.z - inset;
        let y = 0.2 + level as f32 * 0.4;
        let (mut x, mut row) = (-half_x, 0u32);
        while x < half_x {
            let width = 2.0_f32.min(half_x - x);
            let mut z = -half_z;
            let mut column = 0u32;
            while z < half_z {
                let depth = 2.4_f32.min(half_z - z);
                let id = level * 100_000 + row * 300 + column;
                out.push(place(
                    if random(id, 5) > 0.5 {
                        mesh::BLOCK_A
                    } else {
                        mesh::BLOCK_B
                    },
                    if level == 2 {
                        material::MARBLE
                    } else {
                        material::SANDSTONE
                    },
                    Vec3::new(x + width * 0.5, y, z + depth * 0.5),
                    Quat::IDENTITY,
                    scale(width * 0.995, 0.4, depth * 0.995),
                ));
                z += depth;
                column += 1;
            }
            x += width;
            row += 1;
        }
    }

    // Polished floor inside the colonnade, to give reflections something to show.
    let mut x = -6.0_f32;
    while x < 6.0 {
        let mut z = -14.0_f32;
        while z < 14.0 {
            out.push(place(
                mesh::BLOCK_B,
                material::POLISHED,
                Vec3::new(x + 0.75, PLATFORM_TOP + 0.04, z + 0.75),
                Quat::IDENTITY,
                scale(1.495, 0.08, 1.495),
            ));
            z += 1.5;
        }
        x += 1.5;
    }

    // Peristyle: columns on the perimeter of the platform.
    let (columns_x, columns_z) = (6u32, 12u32);
    let (extent_x, extent_z) = (8.25, 17.25);
    let mut column_positions = Vec::new();
    for i in 0..columns_x {
        let x = -extent_x + 2.0 * extent_x * i as f32 / (columns_x - 1) as f32;
        column_positions.push(Vec3::new(x, PLATFORM_TOP, -extent_z));
        column_positions.push(Vec3::new(x, PLATFORM_TOP, extent_z));
    }
    for i in 1..columns_z - 1 {
        let z = -extent_z + 2.0 * extent_z * i as f32 / (columns_z - 1) as f32;
        column_positions.push(Vec3::new(-extent_x, PLATFORM_TOP, z));
        column_positions.push(Vec3::new(extent_x, PLATFORM_TOP, z));
    }

    // A few columns have fallen: leave gaps and lay their drums nearby.
    let fallen = [3usize, 14, 21];
    for (index, &position) in column_positions.iter().enumerate() {
        let id = index as u32;
        if fallen.contains(&index) {
            let direction = if position.x > 0.0 { 1.0 } else { -1.0 };
            let lying = Quat::from_rotation_y(random(id, 8) * 0.9 - 0.45)
                * Quat::from_rotation_z(-direction * 1.53);
            let ground_point = position + Vec3::new(direction * 3.0, -0.1, 0.0);
            out.push(place(
                mesh::COLUMN,
                material::MARBLE,
                ground_point,
                lying,
                Vec3::ONE,
            ));
            continue;
        }
        let yaw = Quat::from_rotation_y(random(id, 6) * std::f32::consts::TAU);
        out.push(place(
            mesh::COLUMN,
            material::MARBLE,
            position,
            yaw,
            Vec3::ONE,
        ));
    }

    // Entablature on top of the columns, broken where columns have fallen.
    let top = PLATFORM_TOP + COLUMN_HEIGHT;
    let beam = |out: &mut Vec<Instance>, from: Vec3, to: Vec3, id: u32| {
        let middle = (from + to) * 0.5 + Vec3::Y * 0.4;
        let along = to - from;
        let yaw = Quat::from_rotation_y((-along.z).atan2(along.x));
        out.push(place(
            if random(id, 9) > 0.5 {
                mesh::BLOCK_A
            } else {
                mesh::BLOCK_B
            },
            material::MARBLE,
            middle,
            yaw,
            scale(along.length() + 0.1, 0.8, 1.15),
        ));
    };
    let at = |x: f32, z: f32| Vec3::new(x, top, z);
    for i in 0..columns_x - 1 {
        let x0 = -extent_x + 2.0 * extent_x * i as f32 / (columns_x - 1) as f32;
        let x1 = -extent_x + 2.0 * extent_x * (i + 1) as f32 / (columns_x - 1) as f32;
        beam(out, at(x0, extent_z), at(x1, extent_z), i);
        if i != 1 {
            beam(out, at(x0, -extent_z), at(x1, -extent_z), i + 50);
        }
    }
    for i in 0..columns_z - 1 {
        let z0 = -extent_z + 2.0 * extent_z * i as f32 / (columns_z - 1) as f32;
        let z1 = -extent_z + 2.0 * extent_z * (i + 1) as f32 / (columns_z - 1) as f32;
        if i != 5 {
            beam(out, at(extent_x, z0), at(extent_x, z1), i + 100);
        }
        if i != 8 {
            beam(out, at(-extent_x, z0), at(-extent_x, z1), i + 150);
        }
    }

    // Cella: an enclosed inner chamber with a doorway facing +Z and a partly open roof.
    let (cx, cz0, cz1) = (5.0, -12.0, 12.0);
    let base = PLATFORM_TOP;
    let courses = 10;
    wall_run(
        out,
        Vec3::new(-cx, 0.0, cz0),
        Vec3::new(-cx, 0.0, cz1),
        base,
        courses,
        0.9,
        1000,
    );
    wall_run(
        out,
        Vec3::new(cx, 0.0, cz0),
        Vec3::new(cx, 0.0, cz1),
        base,
        courses,
        0.9,
        2000,
    );
    wall_run(
        out,
        Vec3::new(-cx, 0.0, cz0),
        Vec3::new(cx, 0.0, cz0),
        base,
        courses,
        0.9,
        3000,
    );
    wall_run(
        out,
        Vec3::new(-cx, 0.0, cz1),
        Vec3::new(-1.6, 0.0, cz1),
        base,
        courses,
        0.9,
        4000,
    );
    wall_run(
        out,
        Vec3::new(1.6, 0.0, cz1),
        Vec3::new(cx, 0.0, cz1),
        base,
        courses,
        0.9,
        5000,
    );
    // Lintel and wall above the doorway.
    let lintel_base = base + 5.74;
    wall_run(
        out,
        Vec3::new(-1.6, 0.0, cz1),
        Vec3::new(1.6, 0.0, cz1),
        lintel_base,
        3,
        0.9,
        6000,
    );

    // Roof beams spanning the chamber. Leaving some out lets shafts of sunlight in.
    let roof_y = base + 8.6;
    let mut z = cz0 + 0.6;
    let mut index = 0;
    while z < cz1 {
        if index % 5 != 2 {
            out.push(place(
                if index % 2 == 0 {
                    mesh::BLOCK_A
                } else {
                    mesh::BLOCK_B
                },
                material::SANDSTONE,
                Vec3::new(0.0, roof_y, z),
                Quat::IDENTITY,
                scale(cx * 2.0 + 1.2, 0.7, 1.1),
            ));
        }
        z += 1.2;
        index += 1;
    }

    // Fire bowls, lighting the chamber in warm tones.
    for &(x, z) in &[(-3.2f32, -9.0f32), (3.2, -9.0), (-3.2, 3.0), (3.2, 3.0)] {
        let pedestal = Vec3::new(x, base + 0.65, z);
        out.push(place(
            mesh::BLOCK_A,
            material::MARBLE,
            pedestal,
            Quat::IDENTITY,
            Vec3::new(0.7, 1.3, 0.7),
        ));
        let flame = pedestal + Vec3::Y * 0.92;
        out.push(place(
            mesh::SPHERE,
            material::FIRE,
            flame,
            Quat::IDENTITY,
            Vec3::new(0.26, 0.34, 0.26),
        ));
        out.push(place(
            mesh::SPHERE,
            material::BRONZE,
            flame - Vec3::Y * 0.22,
            Quat::IDENTITY,
            Vec3::new(0.44, 0.16, 0.44),
        ));
        lights.push(Light {
            position: flame.to_array(),
            range: 16.0,
            intensity: [40.0, 14.0, 3.0],
            _pad: 0,
        });
    }

    // A bronze sphere at the chamber's heart, to show reflections.
    out.push(place(
        mesh::BLOCK_B,
        material::MARBLE,
        Vec3::new(0.0, base + 0.5, -6.5),
        Quat::IDENTITY,
        Vec3::new(1.4, 1.0, 1.4),
    ));
    out.push(place(
        mesh::SPHERE,
        material::BRONZE,
        Vec3::new(0.0, base + 1.75, -6.5),
        Quat::IDENTITY,
        Vec3::splat(0.75),
    ));

    // Rubble: blocks fallen around the temple's edges.
    for i in 0..70u32 {
        let side = random(i, 20);
        let along = random(i, 21) * 2.0 - 1.0;
        let out_by = 1.0 + random(i, 22) * 5.0;
        let (x, z) = if side < 0.5 {
            (
                along * (TEMPLE_HALF.x + 3.0),
                (TEMPLE_HALF.z + out_by) * if side < 0.25 { 1.0 } else { -1.0 },
            )
        } else {
            (
                (TEMPLE_HALF.x + out_by) * if side < 0.75 { 1.0 } else { -1.0 },
                along * (TEMPLE_HALF.z + 3.0),
            )
        };
        let size = 0.4 + random(i, 23) * 0.9;
        let tumble = Quat::from_euler(
            glam::EulerRot::XYZ,
            random(i, 24) * 0.9 - 0.45,
            random(i, 25) * std::f32::consts::TAU,
            random(i, 26) * 0.9 - 0.45,
        );
        out.push(place(
            if i % 2 == 0 {
                mesh::BLOCK_A
            } else {
                mesh::BLOCK_B
            },
            if i % 3 == 0 {
                material::MARBLE
            } else {
                material::SANDSTONE
            },
            Vec3::new(x, terrain_height(x, z) + size * 0.25, z),
            tumble,
            Vec3::new(size * 1.5, size, size * 1.1),
        ));
    }
}

/// Is this point clear of the temple, with a margin in metres?
fn outside_temple(x: f32, z: f32, margin: f32) -> bool {
    x.abs() > TEMPLE_HALF.x + margin || z.abs() > TEMPLE_HALF.z + margin
}

fn rocks(out: &mut Vec<Instance>, boulders: u32, pebbles: u32) {
    let rotation = |i: u32, stream: u32| {
        Quat::from_euler(
            glam::EulerRot::YXZ,
            random(i, stream) * std::f32::consts::TAU,
            random(i, stream + 1) * 0.8 - 0.4,
            random(i, stream + 2) * 0.8 - 0.4,
        )
    };

    // Canyon walls: enormous boulders in a ring, the main demonstration that one mesh
    // can hold both cliff-scale silhouette and close-up detail.
    for i in 0..220u32 {
        let angle = random(i, 30) * std::f32::consts::TAU;
        let radius = 125.0 + random(i, 31) * 130.0;
        let (x, z) = (angle.cos() * radius, angle.sin() * radius);
        let size = 14.0 + random(i, 32) * 26.0 + (radius - 125.0) * 0.14;
        out.push(place(
            if i % 2 == 0 {
                mesh::ROCK_A
            } else {
                mesh::ROCK_B
            },
            material::BASALT,
            Vec3::new(x, terrain_height(x, z) + size * 0.15, z),
            rotation(i, 33),
            Vec3::new(size, size * (0.7 + random(i, 36) * 0.7), size),
        ));
    }

    // Boulders on the valley floor.
    for i in 0..boulders {
        let angle = random(i, 40) * std::f32::consts::TAU;
        let radius = 18.0 + random(i, 41).sqrt() * 120.0;
        let (x, z) = (angle.cos() * radius, angle.sin() * radius);
        if !outside_temple(x, z, 4.0) {
            continue;
        }
        let size = 0.35 + random(i, 42).powi(4) * 4.5;
        out.push(place(
            if i % 2 == 0 {
                mesh::ROCK_A
            } else {
                mesh::ROCK_B
            },
            material::BASALT,
            Vec3::new(x, terrain_height(x, z) + size * 0.1, z),
            rotation(i, 43),
            Vec3::splat(size),
        ));
    }

    // Stones scattered everywhere, clustered by a noise field.
    let mut placed = 0;
    let mut i = 0;
    while placed < pebbles && i < pebbles * 6 {
        i += 1;
        let x = (random(i, 50) - 0.5) * 220.0;
        let z = (random(i, 51) - 0.5) * 220.0;
        if !outside_temple(x, z, 1.5) {
            continue;
        }
        let density = fbm(Vec3::new(x, 0.0, z) * 0.05, 3, 77);
        if density < -0.05 {
            continue;
        }
        let size = 0.04 + random(i, 52).powi(3) * 0.32;
        out.push(place(
            if i % 2 == 0 {
                mesh::ROCK_A
            } else {
                mesh::ROCK_B
            },
            if i % 5 == 0 {
                material::SANDSTONE
            } else {
                material::BASALT
            },
            Vec3::new(x, terrain_height(x, z) + size * 0.2, z),
            rotation(i, 53),
            Vec3::splat(size),
        ));
        placed += 1;
    }
}

/// Builds the complete scene.
pub fn build_temple(quality: Quality) -> SceneAsset {
    let (texture_size, boulders, pebbles) = match quality {
        Quality::Draft => (256, 300, 2000),
        Quality::Web => (1024, 900, 12_000),
        Quality::High => (1024, 1500, 24_000),
    };

    println!("generating meshes...");
    let meshes_source = build_meshes(quality);
    println!("building cluster DAGs...");
    let options = BuildOptions::default();
    let meshes = meshes_source
        .iter()
        .map(|mesh| build_virtual_mesh(mesh, &options))
        .collect();

    println!("generating textures...");
    let textures = vec![
        surfaces::sandstone(0).bake(texture_size),
        surfaces::marble(0).bake(texture_size),
        surfaces::sand(0).bake(texture_size),
        surfaces::basalt(0).bake(texture_size),
    ];

    let mut instances = vec![place(
        mesh::TERRAIN,
        material::GROUND,
        Vec3::ZERO,
        Quat::IDENTITY,
        Vec3::ONE,
    )];
    let mut lights = Vec::new();
    temple(&mut instances, &mut lights);
    rocks(&mut instances, boulders, pebbles);

    SceneAsset {
        meshes,
        materials: materials(),
        instances,
        lights,
        texture_size,
        texture_encoding: Default::default(),
        textures,
        viewpoint: Some(Viewpoint {
            position: [20.0, 4.2, 34.0],
            yaw: 0.6,
            pitch: -0.04,
        }),
        sea: None,
        gameplay: Vec::new(),
        collide: Vec::new(),
        ground: None,
        ..Default::default()
    }
}
