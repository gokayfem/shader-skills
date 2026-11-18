//! A meadow at the edge of a wood, to show procedural foliage.

use glam::{Mat4, Vec3};
use solstice_asset::scene::Viewpoint;
use solstice_asset::{Instance, Material, SceneAsset};
use solstice_bake::meshes;
use solstice_builder::{build_virtual_mesh, BuildOptions};
use solstice_procgen::foliage::bark::Bark;
use solstice_procgen::foliage::grass::Meadow;
use solstice_procgen::foliage::leaf::LeafKind;
use solstice_procgen::foliage::tree::Species;
use solstice_procgen::foliage::Rng;
use solstice_procgen::noise::fbm;
use solstice_procgen::surface::TILE_METRES;
use solstice_procgen::surfaces;

/// Edge of the ground, in metres.
const GROUND_SIZE: f32 = 160.0;
/// Edge of the square that is covered in grass, centred on the ground.
const MEADOW_SIZE: f32 = 70.0;
/// Different grass patches, laid out at random so no repeat shows.
const PATCH_VARIANTS: u32 = 4;
/// The plants of the wood, and how many differently grown copies of each.
const PLANTS: [(&str, u32); 4] = [("oak", 2), ("birch", 2), ("fir", 2), ("bush", 2)];

mod material {
    pub const GROUND: u32 = 0;
    pub const GRASS: u32 = 1;
}

/// The texture layers a scene has so far, so each bark and leaf atlas is generated only once.
struct Layers {
    names: Vec<String>,
    textures: Vec<solstice_asset::TextureLayer>,
    size: u32,
}

impl Layers {
    fn layer(
        &mut self,
        name: &str,
        make: impl FnOnce() -> solstice_procgen::surface::Surface,
    ) -> i32 {
        if let Some(index) = self.names.iter().position(|n| n == name) {
            return index as i32;
        }
        println!("  {name}");
        self.names.push(name.to_string());
        self.textures.push(make().bake(self.size));
        self.textures.len() as i32 - 1
    }
}

/// Meshes and materials of one species, wood then leaves for each grown copy.
struct Grown {
    first_mesh: u32,
    variants: u32,
    bark: u32,
    leaves: u32,
}

/// Gently rolling ground.
fn ground_height(x: f32, z: f32) -> f32 {
    fbm(Vec3::new(x * 0.012, 0.0, z * 0.012), 4, 3) * 5.0
        + fbm(Vec3::new(x * 0.05, 1.0, z * 0.05), 3, 4) * 0.6
}

/// A small, fast, deterministic hash for placement.
fn hash(x: i32, z: i32, channel: u32) -> u32 {
    let mut h = (x as u32).wrapping_mul(0x8da6_b343)
        ^ (z as u32).wrapping_mul(0xd816_3841)
        ^ channel.wrapping_mul(0xcb1a_b31f);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2c1b_3c6d);
    h ^ (h >> 12)
}

/// Places a patch whose local square is `0..size` so it lies on the ground: sheared to the ground's
/// slope, so every blade still grows straight up and every root touches the ground, and turned by
/// a quarter turn at random.
fn patch_transform(corner_x: f32, corner_z: f32, size: f32, turn: u32) -> Mat4 {
    let half = size * 0.5;
    let (cx, cz) = (corner_x + half, corner_z + half);
    let h = ground_height(cx, cz);
    let step = half;
    let slope_x = (ground_height(cx + step, cz) - ground_height(cx - step, cz)) / (2.0 * step);
    let slope_z = (ground_height(cx, cz + step) - ground_height(cx, cz - step)) / (2.0 * step);
    let shear = Mat4::from_cols(
        glam::Vec4::new(1.0, slope_x, 0.0, 0.0),
        glam::Vec4::Y,
        glam::Vec4::new(0.0, slope_z, 1.0, 0.0),
        glam::Vec4::W,
    );
    Mat4::from_translation(Vec3::new(cx, h, cz))
        * shear
        * Mat4::from_rotation_y(turn as f32 * std::f32::consts::FRAC_PI_2)
        * Mat4::from_translation(Vec3::new(-half, 0.0, -half))
}

pub fn build(texture_size: u32) -> SceneAsset {
    let options = BuildOptions::default();
    let meadow = Meadow::default();

    println!("growing grass...");
    let mut meshes = vec![build_virtual_mesh(
        &meshes::terrain(GROUND_SIZE, 256, ground_height),
        &options,
    )];
    for seed in 0..PATCH_VARIANTS {
        let mesh = Meadow {
            seed,
            ..meadow.clone()
        }
        .mesh();
        println!("  patch {seed}: {} triangles", mesh.triangle_count());
        meshes.push(build_virtual_mesh(&mesh, &options));
    }

    println!("generating textures...");
    let mut layers = Layers {
        names: Vec::new(),
        textures: Vec::new(),
        size: texture_size,
    };
    layers.layer("grass", || surfaces::grass(0));
    layers.layer("grass_blades", || meadow.atlas(0));
    let mut materials = vec![
        Material {
            base_color: [0.8, 0.8, 0.8, 1.0],
            roughness: 1.0,
            uv_scale: 1.0 / TILE_METRES,
            texture_layer: 0,
            macro_variation: 1.0,
            ..Default::default()
        },
        solstice_bake::foliage::blades(1),
    ];

    // Each plant is two meshes, wood then leaves, placed together.
    println!("growing trees...");
    let mut grown = Vec::new();
    for (name, variants) in PLANTS {
        let species = Species::by_name(name).expect("listed species exists");
        let first_mesh = meshes.len() as u32;
        for seed in 0..variants {
            let plant = species.grow(seed);
            println!(
                "  {name} {seed}: {} wood and {} leaf triangles",
                plant.wood.triangle_count(),
                plant.leaves.triangle_count()
            );
            meshes.push(build_virtual_mesh(&plant.wood, &options));
            meshes.push(build_virtual_mesh(&plant.leaves, &options));
        }
        let bark: Bark = species.bark;
        let leaf: LeafKind = species.leaves.kind;
        let bark_layer = layers.layer(bark.name(), || bark.surface(0));
        let leaf_layer = layers.layer(leaf.name(), || leaf.atlas(0));
        materials.push(solstice_bake::foliage::bark(bark_layer));
        materials.push(solstice_bake::foliage::leaves(leaf_layer));
        grown.push(Grown {
            first_mesh,
            variants,
            bark: materials.len() as u32 - 2,
            leaves: materials.len() as u32 - 1,
        });
    }
    let [oak, birch, fir, bush] = [0, 1, 2, 3].map(|i| &grown[i]);

    let mut instances = vec![Instance {
        mesh: 0,
        material: material::GROUND,
        transform: Mat4::IDENTITY,
    }];
    let patches = (MEADOW_SIZE / meadow.size) as i32;
    let start = -MEADOW_SIZE * 0.5;
    for j in 0..patches {
        for i in 0..patches {
            let variant = hash(i, j, 0) % PATCH_VARIANTS;
            let turn = hash(i, j, 1) % 4;
            let x = start + i as f32 * meadow.size;
            let z = start + j as f32 * meadow.size;
            instances.push(Instance {
                mesh: 1 + variant,
                material: material::GRASS,
                transform: patch_transform(x, z, meadow.size, turn),
            });
        }
    }
    println!("{} grass patches", instances.len() - 1);

    let place_plant =
        |instances: &mut Vec<Instance>, plant: &Grown, variant: u32, x: f32, z: f32, k: u32| {
            let mesh = plant.first_mesh + (variant % plant.variants) * 2;
            let turn = (hash(x as i32, z as i32, k) % 1000) as f32 / 1000.0 * std::f32::consts::TAU;
            let size = 0.8 + (hash(x as i32, z as i32, k + 1) % 1000) as f32 / 1000.0 * 0.45;
            let transform = Mat4::from_translation(Vec3::new(x, ground_height(x, z) - 0.05, z))
                * Mat4::from_rotation_y(turn)
                * Mat4::from_scale(Vec3::splat(size));
            instances.push(Instance {
                mesh,
                material: plant.bark,
                transform,
            });
            instances.push(Instance {
                mesh: mesh + 1,
                material: plant.leaves,
                transform,
            });
        };
    // A wood beyond the meadow, thickening with distance, a few trees standing alone in the
    // meadow, and bushes along the wood's edge. Trees are scattered by dart throwing, which keeps
    // each one its own space as competition for light does, without the rows of a jittered grid.
    let mut rng = Rng::new(7, 3);
    let wood_density = |x: f32, z: f32| ((x.abs().max(z.abs()) - 30.0) / 22.0).clamp(0.0, 1.0);
    let mut stands: Vec<(f32, f32)> = vec![(-9.0, -14.0), (13.0, -22.0), (-20.0, 4.0)];
    for _ in 0..40_000 {
        let (x, z) = (rng.range(-78.0, 78.0), rng.range(-78.0, 78.0));
        let density = wood_density(x, z);
        if density <= 0.0 || rng.unit() > density {
            continue;
        }
        // Closer together deeper in the wood.
        let spacing = 9.0 - 4.0 * density;
        if stands
            .iter()
            .all(|&(sx, sz)| (sx - x).powi(2) + (sz - z).powi(2) > spacing * spacing)
        {
            stands.push((x, z));
        }
    }
    // Firs gather deep in the wood, birches in groups at its edge, oaks everywhere between.
    let mut counts = [0; 3];
    for (index, &(x, z)) in stands.iter().enumerate() {
        let depth = wood_density(x, z);
        let stand = fbm(Vec3::new(x * 0.03, 7.0, z * 0.03), 3, 5);
        let pick = if depth > 0.6 && stand > -0.05 {
            2
        } else if depth < 0.7 && stand < 0.05 {
            1
        } else {
            0
        };
        counts[pick] += 1;
        let plant = [oak, birch, fir][pick];
        place_plant(&mut instances, plant, index as u32, x, z, 9);
    }
    let trees = stands.len();
    let mut bushes = 0;
    for _ in 0..3_000 {
        let (x, z) = (rng.range(-60.0, 60.0), rng.range(-60.0, 60.0));
        // Thickest just outside the meadow, in the light at the wood's edge.
        let edge = x.abs().max(z.abs());
        let chance = (1.0 - ((edge - 34.0) / 7.0).abs()).max(0.0);
        if rng.unit() < chance * 0.12 {
            let variant = rng.index(bush.variants as usize) as u32;
            place_plant(&mut instances, bush, variant, x, z, 11);
            bushes += 1;
        }
    }
    println!(
        "{trees} trees ({} oaks, {} birches, {} firs) and {bushes} bushes",
        counts[0], counts[1], counts[2]
    );

    let eye = Vec3::new(0.0, 0.0, 12.0);
    SceneAsset {
        meshes,
        materials,
        instances,
        texture_size,
        textures: layers.textures,
        viewpoint: Some(Viewpoint {
            position: [eye.x, ground_height(eye.x, eye.z) + 1.6, eye.z],
            yaw: 0.0,
            pitch: -0.12,
        }),
        ..Default::default()
    }
}
