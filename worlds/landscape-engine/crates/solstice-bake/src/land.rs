//! A coast's land in bands: sand along the water, a strip of dirt, then grass, with a meadow of
//! real grass on it and a wood beyond, as a scene file asks with `land`, `path`, `yard`,
//! `clear`, `meadow` and `wood` (see `scene_file.rs`).
//!
//! The bands follow how far inland a spot is from where the shore begins, so they keep to the
//! coast wherever it wanders, and each border meanders by its own noise, as a beach's edge and
//! a field's do. Paths and yards are dirt wherever they lie.
//!
//! Each border is where a smooth field crosses zero: how far past the sand's wandering edge a
//! spot lies, and how far past the dirt's or a path's. The terrain is one mesh whose texture
//! coordinates hold how much of each kind of ground there is at each point, sand and dirt, the
//! rest being grass, easing from one to the other over a metre or so either side of a border.
//! Its material blends the three textures there ([`Material::BLENDS`]), each laid by the
//! world's x and z, with what stands proud in each showing first, as grass grows through the
//! edge of a path and sand drifts into the hollows of turf.
//!
//! Grass patches fill the meadow. Where its edge is near, a patch is cut to the shape of the
//! grass there, from a few ragged shapes, and the grass dwindles towards its edge, so the
//! meadow ends as a frayed line, not a row of squares. Loose clumps stand on the turf beyond.
//! They sway in the wind. Trees and bushes are scattered by dart throwing over grass, clear of
//! paths, yards and the places the file keeps clear, and sway too.

use glam::{Mat4, Vec2, Vec3};
use solstice_asset::{Instance, Material, SceneAsset};
use solstice_builder::{build_virtual_mesh, BuildOptions, Mesh};
use solstice_procgen::foliage::grass::Meadow;
use solstice_procgen::foliage::Rng;
use solstice_procgen::noise::fbm;
use solstice_procgen::surface::TILE_METRES;

/// The kinds of ground, in the order of [`Plan::kind`]'s answers.
pub const SAND: usize = 0;
pub const DIRT: usize = 1;
pub const GRASS: usize = 2;
/// How far either side of a border two kinds of ground blend, in metres.
const BLEND_WIDTH: f32 = 2.0;
/// How far each border wanders either way, in metres.
const SAND_WANDER: f32 = 4.0;
const DIRT_WANDER: f32 = 2.5;

/// What the scene file asks of the land.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Plan {
    /// Whether the land is banded at all: a `land` line.
    pub banded: bool,
    /// How far inland from where the shore begins sand reaches, and the dirt beyond it, metres.
    pub sand: f32,
    pub dirt: f32,
    /// Dirt paths: from, to, width.
    pub paths: Vec<(Vec2, Vec2, f32)>,
    /// Dirt yards: opposite corners.
    pub yards: Vec<(Vec2, Vec2)>,
    /// Places kept clear of plants: centre and radius.
    pub clear: Vec<(Vec2, f32)>,
    /// A meadow of real grass on the grass within this distance of the origin, if any.
    pub meadow: Option<f32>,
    /// Woods: centre, radius, and the spacing between trees.
    pub woods: Vec<(Vec2, f32, f32)>,
}

/// The distance from `p` to the segment from `a` to `b`.
fn to_segment(p: Vec2, a: Vec2, b: Vec2) -> f32 {
    let along = b - a;
    let t = ((p - a).dot(along) / along.length_squared().max(1.0e-6)).clamp(0.0, 1.0);
    p.distance(a + along * t)
}

impl Plan {
    /// The two fields whose zeros are the borders at `p`, `inland` metres in from where the
    /// shore begins, in metres: past the sand's edge (sand below zero), and past the dirt's edge
    /// or a path's or a yard's (dirt below zero, where it is not sand).
    pub fn fields(&self, p: Vec2, inland: f32) -> (f32, f32) {
        let sand_edge =
            self.sand + fbm(Vec3::new(p.x * 0.03, 11.0, p.y * 0.03), 4, 703) * SAND_WANDER * 2.0;
        let dirt_edge = sand_edge
            + self.dirt
            + fbm(Vec3::new(p.x * 0.05, 17.0, p.y * 0.05), 4, 705) * DIRT_WANDER * 2.0;
        let wander = fbm(Vec3::new(p.x * 0.4, 31.0, p.y * 0.4), 3, 701) * 0.35;
        let mut dirt = inland - dirt_edge;
        for &(a, b, width) in &self.paths {
            dirt = dirt.min(to_segment(p, a, b) - width * 0.5 - wander);
        }
        for &(a, b) in &self.yards {
            let (low, high) = (a.min(b), a.max(b));
            let centre = (low + high) * 0.5;
            let half = (high - low) * 0.5;
            let d = (p - centre).abs() - half;
            let outside = d.max(Vec2::ZERO).length() + d.x.max(d.y).min(0.0);
            dirt = dirt.min(outside - wander);
        }
        (inland - sand_edge, dirt)
    }

    /// The kind of ground at `p`, `inland` metres in from where the shore begins.
    pub fn kind(&self, p: Vec2, inland: f32) -> usize {
        kind_of(self.fields(p, inland))
    }

    /// Whether a plant may stand at `p`, `reach` metres across: on grass, not on a path or yard,
    /// nor anywhere kept clear.
    pub fn may_plant(&self, p: Vec2, reach: f32, kind_at: &dyn Fn(Vec2) -> usize) -> bool {
        [Vec2::ZERO, Vec2::X, Vec2::NEG_X, Vec2::Y, Vec2::NEG_Y]
            .iter()
            .all(|&d| kind_at(p + d * reach) == GRASS)
            && self.clear.iter().all(|&(c, r)| p.distance(c) > r + reach)
    }
}

/// The kind of ground for a pair of border fields.
fn kind_of((sand, dirt): (f32, f32)) -> usize {
    if sand < 0.0 {
        SAND
    } else if dirt < 0.0 {
        DIRT
    } else {
        GRASS
    }
}

fn smoothstep(low: f32, high: f32, x: f32) -> f32 {
    let t = ((x - low) / (high - low)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// How much of the ground is sand and how much dirt, for a pair of border fields; the rest is
/// grass. Each is half at its border, as [`kind_of`] draws it.
pub fn shares((sand, dirt): (f32, f32)) -> Vec2 {
    let sand = 1.0 - smoothstep(-BLEND_WIDTH, BLEND_WIDTH, sand);
    let dirt = (1.0 - sand) * (1.0 - smoothstep(-BLEND_WIDTH, BLEND_WIDTH, dirt));
    Vec2::new(sand, dirt)
}

/// The terrain, `size` wide with `cells` quads a side, with each point's shares of sand and
/// dirt in its texture coordinates.
pub fn blended_terrain(
    size: f32,
    cells: u32,
    height: &dyn Fn(f32, f32) -> f32,
    fields: &(dyn Fn(Vec2) -> (f32, f32) + Sync),
) -> Mesh {
    use rayon::prelude::*;
    let mut mesh = crate::meshes::terrain(size, cells, height);
    mesh.vertices
        .par_iter_mut()
        .for_each(|v| v.uv = shares(fields(Vec2::new(v.position.x, v.position.z))));
    mesh
}

/// A texture layer for the built-in surface `name`, added to the scene.
fn layer(scene: &mut SceneAsset, name: &str) -> i32 {
    scene
        .textures
        .push(crate::textures::surface(name, scene.texture_size));
    scene.textures.len() as i32 - 1
}

/// Adds the banded land to `scene`: its three kinds of ground, the meadow and the woods.
/// Returns the sand's texture layer, which water ripples are drawn from.
pub fn add(
    scene: &mut SceneAsset,
    plan: &Plan,
    size: f32,
    cells: u32,
    height: &(dyn Fn(f32, f32) -> f32 + Sync),
    inland: &(dyn Fn(f32, f32) -> f32 + Sync),
) -> i32 {
    let kind_at = |p: Vec2| plan.kind(p, inland(p.x, p.y));
    let fields = |p: Vec2| plan.fields(p, inland(p.x, p.y));
    println!("  laying the land...");
    let mesh = blended_terrain(size, cells, &|x, z| height(x, z), &fields);
    // Grass first, then sand and dirt, in consecutive layers and materials, as the blend reads
    // them. Each darkened as it is seen in the field, so they sit beside the scans: turf about
    // 0.75 of its texture, damp sand 0.68, earth 0.62.
    let first = scene.textures.len() as i32;
    for name in ["grass", "sand", "dirt"] {
        layer(scene, name);
    }
    for (k, tone) in [0.75, 0.68, 0.62].into_iter().enumerate() {
        scene.materials.push(Material {
            base_color: [tone, tone, tone, 1.0],
            roughness: 1.0,
            uv_scale: 1.0 / TILE_METRES,
            normal_strength: 1.0,
            texture_layer: first + k as i32,
            macro_variation: 1.0,
            flags: if k == 0 { Material::BLENDS } else { 0 },
            ..Default::default()
        });
    }
    scene.instances.push(Instance {
        mesh: scene.meshes.len() as u32,
        material: scene.materials.len() as u32 - 3,
        transform: Mat4::IDENTITY,
    });
    scene
        .meshes
        .push(build_virtual_mesh(&mesh, &BuildOptions::default()));
    if let Some(radius) = plan.meadow {
        add_meadow(scene, plan, radius, height, &kind_at);
    }
    if !plan.woods.is_empty() {
        add_woods(scene, plan, height, &kind_at);
    }
    // The sand's layer.
    first + 1
}

/// Different grass patches, laid at random so no repeat shows: whole ones, and of each shape.
const PATCH_VARIANTS: u32 = 4;
const SHAPE_VARIANTS: u32 = 3;

/// A small, fast hash for placement.
fn hash(x: i32, z: i32, channel: u32) -> u32 {
    let mut h = (x as u32).wrapping_mul(0x8da6_b343)
        ^ (z as u32).wrapping_mul(0xd816_3841)
        ^ channel.wrapping_mul(0xcb1a_b31f);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2c1b_3c6d);
    h ^ (h >> 12)
}

/// A patch whose local square is `0..size`, laid on the ground with its corner at (x, z),
/// sheared to the ground's slope so every blade grows straight up from it, and turned by `turn`
/// radians about its middle.
fn patch_transform(x: f32, z: f32, size: f32, turn: f32, height: &dyn Fn(f32, f32) -> f32) -> Mat4 {
    let half = size * 0.5;
    let (cx, cz) = (x + half, z + half);
    let h = height(cx, cz);
    let slope_x = (height(cx + half, cz) - height(cx - half, cz)) / size;
    let slope_z = (height(cx, cz + half) - height(cx, cz - half)) / size;
    let shear = Mat4::from_cols(
        glam::Vec4::new(1.0, slope_x, 0.0, 0.0),
        glam::Vec4::Y,
        glam::Vec4::new(0.0, slope_z, 1.0, 0.0),
        glam::Vec4::W,
    );
    Mat4::from_translation(Vec3::new(cx, h, cz))
        * shear
        * Mat4::from_rotation_y(turn)
        * Mat4::from_translation(Vec3::new(-half, 0.0, -half))
}

/// The shapes a meadow's patches are cut to where its edge is near, besides a whole square.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Shape {
    /// Grass over the near half, its edge across the middle.
    Half,
    /// Grass in a quarter round about one corner.
    Corner,
    /// Grass but for a quarter round about the far corner.
    Notched,
    /// A round clump in the middle.
    Clump,
    /// Grass all over but frayed along the sides in this mask, [`NORTH`] and so on: a whole
    /// patch at the meadow's rim, where the cells beyond have no grass patch.
    Rim(u8),
}

/// The sides of a patch's own square: towards +Z, +X, -Z and -X.
const NORTH: u8 = 1;
const EAST: u8 = 2;
const SOUTH: u8 = 4;
const WEST: u8 = 8;

/// The shapes for cells the grass only partly covers.
const SHAPES: [Shape; 4] = [Shape::Half, Shape::Corner, Shape::Notched, Shape::Clump];
/// The rims, one for each set of open sides up to a turn: one side, two beside each other, two
/// opposite, three, and all four.
const RIMS: [Shape; 5] = [
    Shape::Rim(NORTH),
    Shape::Rim(NORTH | EAST),
    Shape::Rim(NORTH | SOUTH),
    Shape::Rim(NORTH | EAST | SOUTH),
    Shape::Rim(NORTH | EAST | SOUTH | WEST),
];
/// How far in from an open side a rim's grass reaches before its fray, in metres.
const RIM_DEPTH: f32 = 0.45;

impl Shape {
    /// How far inside the shape a spot of the patch lies, in metres, before its edge frays:
    /// above zero is grass.
    fn inside(self, u: f32, v: f32, size: f32) -> f32 {
        match self {
            Shape::Half => size * 0.55 - v,
            Shape::Corner => size * 0.7 - Vec2::new(u, v).length(),
            Shape::Notched => Vec2::new(size - u, size - v).length() - size * 0.45,
            Shape::Clump => size * 0.4 - Vec2::new(u - size * 0.5, v - size * 0.5).length(),
            Shape::Rim(open) => [(NORTH, size - v), (EAST, size - u), (SOUTH, v), (WEST, u)]
                .into_iter()
                .filter(|&(side, _)| open & side != 0)
                .map(|(_, d)| d - RIM_DEPTH)
                .fold(size, f32::min),
        }
    }

    /// How the grass grows at a spot of a patch of this shape: above zero where it does,
    /// dwindling over its last half metre, the edge frayed inward by noise so no two patches end
    /// alike.
    fn vigour(self, u: f32, v: f32, size: f32, seed: u32) -> f32 {
        // A broad wander, so a row of patches doesn't line its edges up, and a finer one.
        let wander = fbm(
            Vec3::new(u * 0.8, seed as f32 * 3.1, v * 0.8),
            2,
            711 + seed,
        ) * 0.7;
        let fray = fbm(
            Vec3::new(u * 2.5, seed as f32 * 3.1, v * 2.5),
            3,
            709 + seed,
        ) * 0.25;
        // Only ever inward, so no grass grows past the plain edge the fit was judged by.
        let fray = (wander + fray).min(0.0);
        (self.inside(u, v, size) + fray) / 0.5
    }
}

/// Spots of a patch at which its fit to the ground is judged: near its corners, the middles of
/// its sides, and its middle, in its own `0..1` square.
const PROBES: [(f32, f32); 9] = [
    (0.1, 0.1),
    (0.5, 0.1),
    (0.9, 0.1),
    (0.1, 0.5),
    (0.5, 0.5),
    (0.9, 0.5),
    (0.1, 0.9),
    (0.5, 0.9),
    (0.9, 0.9),
];

/// The shape and turn that best fit a patch whose probes are `grass` where the ground is grass:
/// the one covering the most of it without reaching over anything else, by the shapes' plain
/// edges. None where no shape fits.
fn fit(grass: [bool; 9], size: f32) -> Option<(Shape, u32)> {
    let mut best = None;
    let mut most = 0;
    for shape in SHAPES {
        for turn in 0..4u32 {
            let mut covered = 0;
            let mut fits = true;
            for (n, &(u, v)) in PROBES.iter().enumerate() {
                // The spot of the patch's own square that lands on this probe once turned.
                let (u, v) = turned(u, v, turn);
                if shape.inside(u * size, v * size, size) > 0.0 {
                    if grass[n] {
                        covered += 1;
                    } else {
                        fits = false;
                    }
                }
            }
            if fits && covered > most {
                most = covered;
                best = Some((shape, turn));
            }
        }
    }
    best
}

/// The rim and turn whose open sides, once turned, are `open`: middles of a side of the cell
/// where the rim's grass stops short of it. None for no open side.
fn rim(open: u8, size: f32) -> Option<(Shape, u32)> {
    const SIDES: [(u8, (f32, f32)); 4] = [
        (NORTH, (0.5, 0.99)),
        (EAST, (0.99, 0.5)),
        (SOUTH, (0.5, 0.01)),
        (WEST, (0.01, 0.5)),
    ];
    for shape in RIMS {
        for turn in 0..4u32 {
            let opens = SIDES
                .iter()
                .filter(|&&(_, (u, v))| {
                    let (u, v) = turned(u, v, turn);
                    shape.inside(u * size, v * size, size) < 0.0
                })
                .fold(0, |mask, &(side, _)| mask | side);
            if opens == open {
                return Some((shape, turn));
            }
        }
    }
    None
}

/// Which spot of a patch's own `0..1` square lands on `(u, v)` once the patch is turned by
/// `turn` quarter turns about its middle, as [`patch_transform`] turns it.
fn turned(u: f32, v: f32, turn: u32) -> (f32, f32) {
    let r = Mat4::from_rotation_y(-(turn as f32) * std::f32::consts::FRAC_PI_2)
        .transform_vector3(Vec3::new(u - 0.5, 0.0, v - 0.5));
    (r.x + 0.5, r.z + 0.5)
}

/// Grass patches over the grass within `radius` of the origin: whole ones where it is all grass,
/// cut to its shape along its edge, and loose clumps on the turf beyond.
fn add_meadow(
    scene: &mut SceneAsset,
    plan: &Plan,
    radius: f32,
    height: &dyn Fn(f32, f32) -> f32,
    kind_at: &dyn Fn(Vec2) -> usize,
) {
    println!("  growing the meadow...");
    let meadow = Meadow::default();
    let size = meadow.size;
    let first_mesh = scene.meshes.len() as u32;
    // Whole patches, then each shape's.
    for seed in 0..PATCH_VARIANTS {
        let mesh = Meadow {
            seed,
            ..meadow.clone()
        }
        .mesh();
        scene
            .meshes
            .push(build_virtual_mesh(&mesh, &BuildOptions::default()));
    }
    for shape in SHAPES.into_iter().chain(RIMS) {
        for seed in 0..SHAPE_VARIANTS {
            let mesh = Meadow {
                seed: 100 + seed,
                ..meadow.clone()
            }
            .mesh_where(&|u, v| shape.vigour(u, v, size, seed));
            scene
                .meshes
                .push(build_virtual_mesh(&mesh, &BuildOptions::default()));
        }
    }
    let shaped_mesh = |shape: Shape, seed: u32| {
        let which = SHAPES
            .iter()
            .chain(&RIMS)
            .position(|&s| s == shape)
            .expect("a shape with meshes") as u32;
        first_mesh + PATCH_VARIANTS + which * SHAPE_VARIANTS + seed
    };
    let texture_size = scene.texture_size;
    scene.textures.push(crate::textures::layer(
        "grass-atlas-default-0",
        texture_size,
        || meadow.atlas(0).bake(texture_size),
    ));
    let blades = scene.textures.len() as i32 - 1;
    scene.materials.push(crate::foliage::blades(blades));
    let material = scene.materials.len() as u32 - 1;
    let cells = (radius * 2.0 / size) as i32;
    let quarter = std::f32::consts::FRAC_PI_2;
    let (mut whole, mut shaped, mut loose) = (0, 0, 0);
    // First what grows in each cell: a whole patch, a shaped one, or none.
    #[derive(Clone, Copy, PartialEq)]
    enum Cell {
        Bare,
        Whole,
        Shaped(Shape, u32),
    }
    let cell_at = |i: i32, j: i32| {
        let (x, z) = (i as f32 * size - radius, j as f32 * size - radius);
        let centre = Vec2::new(x + size * 0.5, z + size * 0.5);
        if centre.length() > radius {
            return Cell::Bare;
        }
        // Which of the patch's probes stand on grass that may be planted.
        let grass = PROBES
            .map(|(u, v)| plan.may_plant(Vec2::new(x + u * size, z + v * size), 0.15, kind_at));
        if grass.iter().all(|&g| g) && plan.may_plant(centre, size * 0.5, kind_at) {
            Cell::Whole
        } else if let Some((shape, turn)) = fit(grass, size) {
            Cell::Shaped(shape, turn)
        } else {
            Cell::Bare
        }
    };
    let grid: Vec<Cell> = (0..cells * cells)
        .map(|n| cell_at(n % cells, n / cells))
        .collect();
    let at = |i: i32, j: i32| {
        if i < 0 || j < 0 || i >= cells || j >= cells {
            Cell::Bare
        } else {
            grid[(j * cells + i) as usize]
        }
    };
    for j in 0..cells {
        for i in 0..cells {
            let (x, z) = (i as f32 * size - radius, j as f32 * size - radius);
            let centre = Vec2::new(x + size * 0.5, z + size * 0.5);
            match at(i, j) {
                Cell::Whole => {
                    // Frayed on the sides that face no grass patch, so the meadow's rim is never
                    // a square's straight side.
                    let open = [(NORTH, 0, 1), (EAST, 1, 0), (SOUTH, 0, -1), (WEST, -1, 0)]
                        .into_iter()
                        .filter(|&(_, di, dj)| at(i + di, j + dj) == Cell::Bare)
                        .fold(0, |mask, (side, _, _)| mask | side);
                    let (mesh, turn) = match rim(open, size) {
                        Some((shape, turn)) => {
                            (shaped_mesh(shape, hash(i, j, 2) % SHAPE_VARIANTS), turn)
                        }
                        None => (
                            first_mesh + hash(i, j, 0) % PATCH_VARIANTS,
                            hash(i, j, 1) % 4,
                        ),
                    };
                    scene.instances.push(Instance {
                        mesh,
                        material,
                        transform: patch_transform(x, z, size, turn as f32 * quarter, height),
                    });
                    whole += 1;
                    continue;
                }
                Cell::Shaped(shape, turn) => {
                    scene.instances.push(Instance {
                        mesh: shaped_mesh(shape, hash(i, j, 2) % SHAPE_VARIANTS),
                        material,
                        transform: patch_transform(x, z, size, turn as f32 * quarter, height),
                    });
                    shaped += 1;
                }
                Cell::Bare => {}
            }
            // Now and then a loose clump on the turf near the edge, set anywhere and turned any
            // way, so the edge has outliers, as a meadow's does.
            if centre.length() <= radius && hash(i, j, 3).is_multiple_of(2) {
                let jitter = Vec2::new(
                    (hash(i, j, 4) % 1000) as f32 / 1000.0 - 0.5,
                    (hash(i, j, 5) % 1000) as f32 / 1000.0 - 0.5,
                ) * size;
                let at = centre + jitter;
                if plan.may_plant(at, size * 0.4, kind_at) {
                    let turn = (hash(i, j, 6) % 1000) as f32 / 1000.0 * std::f32::consts::TAU;
                    let corner = at - Vec2::splat(size * 0.5);
                    scene.instances.push(Instance {
                        mesh: shaped_mesh(Shape::Clump, hash(i, j, 7) % SHAPE_VARIANTS),
                        material,
                        transform: patch_transform(corner.x, corner.y, size, turn, height),
                    });
                    loose += 1;
                }
            }
        }
    }
    println!("    {whole} whole grass patches, {shaped} shaped to the edge, {loose} loose clumps");
}

/// Trees and bushes over the grass of each wood, scattered by dart throwing: each its own space,
/// as competition for light keeps them, without a grid's rows.
fn add_woods(
    scene: &mut SceneAsset,
    plan: &Plan,
    height: &dyn Fn(f32, f32) -> f32,
    kind_at: &dyn Fn(Vec2) -> usize,
) {
    println!("  growing the woods...");
    let grown = crate::foliage::grow(scene, &[("oak", 2), ("birch", 2), ("bush", 2)]);
    let mut rng = Rng::new(11, 5);
    let mut trees: Vec<Vec2> = Vec::new();
    let mut bushes: Vec<Vec2> = Vec::new();
    let place = |scene: &mut SceneAsset, which: usize, at: Vec2, rng: &mut Rng| {
        let copy = rng.index(grown[which].variants as usize) as u32;
        let transform = Mat4::from_translation(Vec3::new(at.x, height(at.x, at.y) - 0.05, at.y))
            * Mat4::from_rotation_y(rng.range(0.0, std::f32::consts::TAU))
            * Mat4::from_scale(Vec3::splat(rng.range(0.8, 1.25)));
        grown[which].place(scene, copy, transform);
    };
    for &(centre, radius, spacing) in &plan.woods {
        for _ in 0..(radius * radius * 2.0) as u32 {
            let at = centre
                + Vec2::from_angle(rng.range(0.0, std::f32::consts::TAU))
                    * radius
                    * rng.unit().sqrt();
            // Thicker towards the middle of the wood.
            let middle = 1.0 - at.distance(centre) / radius;
            let gap = spacing * (1.4 - 0.6 * middle);
            if trees.iter().any(|t| t.distance(at) < gap) || !plan.may_plant(at, 2.5, kind_at) {
                continue;
            }
            trees.push(at);
            let which = if rng.unit() < 0.65 { 0 } else { 1 };
            place(scene, which, at, &mut rng);
        }
        // Bushes along the wood's edge, where light reaches.
        for _ in 0..(radius * 6.0) as u32 {
            let at = centre
                + Vec2::from_angle(rng.range(0.0, std::f32::consts::TAU))
                    * radius
                    * rng.range(0.75, 1.1);
            if trees.iter().any(|t| t.distance(at) < 2.5)
                || bushes.iter().any(|b| b.distance(at) < 1.6)
                || !plan.may_plant(at, 1.0, kind_at)
            {
                continue;
            }
            bushes.push(at);
            place(scene, 2, at, &mut rng);
        }
    }
    println!("    {} trees and {} bushes", trees.len(), bushes.len());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan() -> Plan {
        Plan {
            banded: true,
            sand: 10.0,
            dirt: 6.0,
            paths: vec![(Vec2::new(0.0, 0.0), Vec2::new(30.0, 0.0), 3.0)],
            yards: vec![(Vec2::new(-5.0, -5.0), Vec2::new(-2.0, -2.0))],
            clear: vec![(Vec2::new(50.0, 50.0), 10.0)],
            meadow: Some(40.0),
            woods: Vec::new(),
        }
    }

    #[test]
    fn sand_then_dirt_then_grass_inland() {
        let plan = plan();
        // Far from paths and yards, along a line straight inland.
        let at = Vec2::new(100.0, 100.0);
        assert_eq!(plan.kind(at, -5.0), SAND);
        assert_eq!(plan.kind(at, 60.0), GRASS);
        let mut last = SAND;
        for step in 0..200 {
            let k = plan.kind(at, step as f32 * 0.25 - 5.0);
            assert!(k >= last, "the bands go sand, dirt, grass, and never back");
            last = k;
        }
        // Somewhere along the way there is dirt.
        assert!((0..200).any(|s| plan.kind(at, s as f32 * 0.25) == DIRT));
    }

    #[test]
    fn paths_and_yards_are_dirt_wherever_they_lie() {
        let plan = plan();
        assert_eq!(plan.kind(Vec2::new(15.0, 0.0), 80.0), DIRT);
        assert_eq!(plan.kind(Vec2::new(-3.5, -3.5), 80.0), DIRT);
        assert_eq!(plan.kind(Vec2::new(15.0, 6.0), 80.0), GRASS);
    }

    #[test]
    fn plants_keep_off_paths_and_clear_places() {
        let plan = plan();
        let kind_at = |p: Vec2| plan.kind(p, 80.0);
        assert!(plan.may_plant(Vec2::new(15.0, 8.0), 1.0, &kind_at));
        assert!(!plan.may_plant(Vec2::new(15.0, 2.0), 1.0, &kind_at));
        assert!(!plan.may_plant(Vec2::new(52.0, 52.0), 1.0, &kind_at));
    }

    #[test]
    fn the_shares_ease_across_each_border() {
        // Deep in each kind, all of it; at a border, half each; and never more than all.
        assert_eq!(shares((-5.0, -5.0)), Vec2::new(1.0, 0.0));
        assert_eq!(shares((5.0, -5.0)), Vec2::new(0.0, 1.0));
        assert_eq!(shares((5.0, 5.0)), Vec2::new(0.0, 0.0));
        assert!((shares((0.0, -5.0)).x - 0.5).abs() < 1.0e-6);
        assert!((shares((5.0, 0.0)).y - 0.5).abs() < 1.0e-6);
        for a in -20..20 {
            for b in -20..20 {
                let fields = (a as f32 * 0.2, b as f32 * 0.2);
                let s = shares(fields);
                assert!(s.x >= 0.0 && s.y >= 0.0 && s.x + s.y <= 1.0 + 1.0e-6);
                // Beyond the blend, all of it is the kind drawn there.
                if fields.0.abs() >= BLEND_WIDTH && fields.1.abs() >= BLEND_WIDTH {
                    let all = [s.x, s.y, 1.0 - s.x - s.y];
                    assert!((all[kind_of(fields)] - 1.0).abs() < 1.0e-6, "{fields:?}");
                }
            }
        }
    }

    #[test]
    fn the_blended_terrain_carries_the_shares() {
        let height = |x: f32, z: f32| (x * 0.3).sin() + (z * 0.2).cos();
        let fields = |p: Vec2| (p.x - 2.0, p.length() - 6.0);
        let mesh = blended_terrain(20.0, 20, &height, &fields);
        assert!(!mesh.vertices.is_empty());
        for v in &mesh.vertices {
            let p = Vec2::new(v.position.x, v.position.z);
            assert_eq!(v.uv, shares(fields(p)));
            assert!((v.position.y - height(p.x, p.y)).abs() < 1.0e-5);
        }
    }

    #[test]
    fn shaped_patches_only_cover_grass() {
        let size = 2.0;
        let covers = |grass: [bool; 9]| {
            let (shape, turn) = fit(grass, size).expect("a shape fits");
            for (n, &(u, v)) in PROBES.iter().enumerate() {
                let (u, v) = turned(u, v, turn);
                if shape.inside(u * size, v * size, size) > 0.0 {
                    assert!(grass[n], "{shape:?} turned {turn} reaches over probe {n}");
                }
            }
            shape
        };
        // Grass along any one side: a half patch, turned to lie along it.
        let near = [true, true, true, true, true, true, false, false, false];
        assert_eq!(covers(near), Shape::Half);
        let left = [true, true, false, true, true, false, true, true, false];
        assert_eq!(covers(left), Shape::Half);
        // Grass at one corner alone: a corner.
        let corner = [false, false, false, false, false, true, false, true, true];
        assert_eq!(covers(corner), Shape::Corner);
        // All but one corner: notched.
        let notched = [true, true, true, true, true, true, true, true, false];
        assert_eq!(covers(notched), Shape::Notched);
        // No grass, no patch.
        assert_eq!(fit([false; 9], size), None);
    }

    #[test]
    fn every_set_of_open_sides_has_a_rim() {
        let size = 2.0;
        assert_eq!(rim(0, size), None);
        for open in 1..16u8 {
            let (shape, turn) = rim(open, size).expect("a rim");
            // Its grass reaches the closed sides' middles, and stops short of the open ones'.
            for (side, (u, v)) in [
                (NORTH, (0.5, 0.99)),
                (EAST, (0.99, 0.5)),
                (SOUTH, (0.5, 0.01)),
                (WEST, (0.01, 0.5)),
            ] {
                let (u, v) = turned(u, v, turn);
                let grows = shape.inside(u * size, v * size, size) > 0.0;
                assert_eq!(
                    grows,
                    open & side == 0,
                    "open {open:04b}, {shape:?} turned {turn}"
                );
            }
        }
    }

    #[test]
    fn turning_matches_the_patch_transform() {
        // A spot of the patch, turned with the patch, lands where `turned` says.
        let size = 2.0;
        for turn in 0..4u32 {
            let m = patch_transform(
                0.0,
                0.0,
                size,
                turn as f32 * std::f32::consts::FRAC_PI_2,
                &|_, _| 0.0,
            );
            for &(u, v) in &PROBES {
                let (pu, pv) = turned(u, v, turn);
                let landed = m.transform_point3(Vec3::new(pu * size, 0.0, pv * size));
                assert!(
                    (landed.x - u * size).abs() < 1.0e-4 && (landed.z - v * size).abs() < 1.0e-4,
                    "turn {turn}: {landed:?} for {u} {v}"
                );
            }
        }
    }

    #[test]
    fn a_meadow_grown_everywhere_is_the_whole_patch() {
        let meadow = Meadow::default();
        let (whole, all) = (meadow.mesh(), meadow.mesh_where(&|_, _| 1.0));
        assert_eq!(whole.indices, all.indices);
        assert_eq!(whole.vertices.len(), all.vertices.len());
        for (a, b) in whole.vertices.iter().zip(&all.vertices) {
            assert_eq!((a.position, a.normal, a.uv), (b.position, b.normal, b.uv));
        }
    }

    #[test]
    fn a_shaped_patch_has_grass_only_inside_its_frayed_edge() {
        let meadow = Meadow::default();
        let size = meadow.size;
        let whole = meadow.mesh();
        let half = meadow.mesh_where(&|u, v| Shape::Half.vigour(u, v, size, 0));
        assert!(!half.vertices.is_empty());
        assert!(half.vertices.len() < whole.vertices.len() * 3 / 4);
        // Nothing grows from beyond the half's frayed edge, nor much past it as blades lean.
        for v in &half.vertices {
            assert!(v.position.z < size * 0.55 + 0.125 + 0.6, "{}", v.position.z);
        }
    }
}
