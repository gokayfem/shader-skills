//! The long grass along the road: a strip mown short at the road's edge, and behind it rough
//! grass out to the barrier and a little beyond.
//!
//! Grass is drawn as square patches of blades. Laid on a grid they would end in steps, a
//! square at a time, and a corner of the circuit would be edged with blocks. So they are laid
//! in rows that follow the road, each patch turned to lie along it, and the rows that end the
//! grass, nearest the road and furthest from it, are of patches whose blades thin out and
//! stop along a wandering line, as grass does where a mower turns.

use super::{blades, hash, Land, Road, BARRIER};
use glam::{Mat4, Vec3, Vec4};
use rustc_hash::FxHashMap;
use solstice_asset::{Instance, SceneAsset};
use solstice_builder::{build_virtual_mesh, BuildOptions, Mesh};
use solstice_procgen::foliage::grass::Meadow;

/// The side of a patch, in metres.
const TILE: f32 = 3.0;
/// How wide the strip mown short beside the road is, and how far behind the barrier the long
/// grass goes on.
const MOWN: f32 = 2.4;
const BEHIND: f32 = 8.0;
/// How far apart patches are laid along a row, as a share of their side: a little close, so
/// that round the outside of a bend no gap opens between them.
const PITCH: f32 = 0.93;
/// How many different patches there are of each kind.
const WHOLE: u32 = 3;
const EDGED: u32 = 2;

/// The blades of a patch, each as the triangles it is made of and where its root is.
fn blades_of(mesh: &Mesh) -> Vec<(Vec<u32>, Vec3)> {
    // Corners at the same place are one point of a blade.
    let mut places: FxHashMap<[i32; 3], u32> = FxHashMap::default();
    let point: Vec<u32> = mesh
        .vertices
        .iter()
        .map(|vertex| {
            let key = (vertex.position * 20_000.0).round().as_ivec3().to_array();
            let next = places.len() as u32;
            *places.entry(key).or_insert(next)
        })
        .collect();
    let mut parent: Vec<u32> = (0..places.len() as u32).collect();
    fn root(parent: &mut [u32], mut v: u32) -> u32 {
        while parent[v as usize] != v {
            parent[v as usize] = parent[parent[v as usize] as usize];
            v = parent[v as usize];
        }
        v
    }
    for triangle in mesh.indices.chunks_exact(3) {
        let a = root(&mut parent, point[triangle[0] as usize]);
        for &corner in &triangle[1..] {
            let other = root(&mut parent, point[corner as usize]);
            parent[other as usize] = a;
        }
    }
    let mut found: FxHashMap<u32, (Vec<u32>, Vec3)> = FxHashMap::default();
    for (index, triangle) in mesh.indices.chunks_exact(3).enumerate() {
        let blade = root(&mut parent, point[triangle[0] as usize]);
        let entry = found
            .entry(blade)
            .or_insert_with(|| (Vec::new(), Vec3::splat(f32::INFINITY)));
        entry.0.push(index as u32);
        for &corner in triangle {
            let position = mesh.vertices[corner as usize].position;
            if position.y < entry.1.y {
                entry.1 = position;
            }
        }
    }
    let mut blades: Vec<(Vec<u32>, Vec3)> = found.into_values().collect();
    // In a fixed order, so that a scene bakes the same every time.
    blades.sort_by(|a, b| a.0[0].cmp(&b.0[0]));
    blades
}

/// A patch with only the blades `keep` says to, by where each one's root is.
fn thinned(mesh: &Mesh, keep: impl Fn(Vec3, u32) -> bool) -> Mesh {
    let mut out = Mesh::default();
    let mut moved: FxHashMap<u32, u32> = FxHashMap::default();
    for (number, (triangles, root)) in blades_of(mesh).into_iter().enumerate() {
        if !keep(root, number as u32) {
            continue;
        }
        for triangle in triangles {
            for corner in 0..3 {
                let index = mesh.indices[triangle as usize * 3 + corner];
                let next = out.vertices.len() as u32;
                let new = *moved.entry(index).or_insert_with(|| {
                    out.vertices.push(mesh.vertices[index as usize]);
                    next
                });
                out.indices.push(new);
            }
        }
    }
    out
}

/// How far in from a patch's side its grass ends, at a point along that side: a wandering
/// line, different for each patch but always the same distance in at the patch's two ends, so
/// that it carries on unbroken from one patch into the next.
fn edge(along: f32, patch: u32) -> f32 {
    let turn = along / TILE * std::f32::consts::PI;
    let wave = |k: u32| hash(patch as i32, k as i32, 77) as f32 / u32::MAX as f32 * 2.0 - 1.0;
    0.55 + 0.42 * turn.sin() * wave(1)
        + 0.3 * (2.0 * turn).sin() * wave(2)
        + 0.2 * (3.0 * turn).sin() * wave(3)
}

/// Grows the long grass along both sides of the road.
pub(super) fn grow(scene: &mut SceneAsset, road: &Road, land: &Land) {
    let samples = &road.track.samples;
    let count = samples.len();
    let meadow = Meadow {
        size: TILE,
        ..Meadow::by_name("verge").expect("a built-in grass")
    };
    let blade = blades(scene, &meadow);

    // The patches: whole ones, and ones whose grass ends along the side towards the road
    // (a patch's own low x) or the side away from it. Near its end the grass thins.
    let first = scene.meshes.len() as u32;
    let mut triangles = 0;
    let chance =
        |number: u32, patch: u32| hash(number as i32, patch as i32, 78) as f32 / u32::MAX as f32;
    for seed in 0..WHOLE + 2 * EDGED {
        let whole = Meadow {
            seed,
            ..meadow.clone()
        }
        .mesh();
        let mesh = if seed < WHOLE {
            whole
        } else {
            let outer = seed >= WHOLE + EDGED;
            thinned(&whole, |root, number| {
                let in_from_side = if outer { TILE - root.x } else { root.x };
                let past = in_from_side - edge(root.z, seed);
                past > 0.0 && past > 0.55 * chance(number, seed).powi(2)
            })
        };
        triangles += mesh.triangle_count();
        scene
            .meshes
            .push(build_virtual_mesh(&mesh, &BuildOptions::default()));
    }

    let rows = ((BARRIER + BEHIND - MOWN) / TILE).ceil() as usize;
    let mut patches = 0;
    for (which, side) in [(0, 1.0_f32), (1, -1.0)] {
        // Away from the road, on the level, at each sample.
        let outward: Vec<Vec3> = samples
            .iter()
            .map(|sample| (sample.left * Vec3::new(1.0, 0.0, 1.0)).normalize_or_zero() * side)
            .collect();
        for row in 0..rows {
            // How far out from the road's middle the middle of this row is.
            let out = |i: usize| road.fence[which][i] - BARRIER + MOWN + (row as f32 + 0.5) * TILE;
            let line: Vec<Vec3> = (0..count)
                .map(|i| samples[i].position + outward[i] * out(i))
                .collect();
            // A patch every so far along the row's own length, which round a bend is not
            // the road's.
            let mut owed = 0.0;
            for i in 0..count {
                let j = (i + 1) % count;
                let length = (line[j] - line[i]).with_y(0.0).length();
                // A row doubling back on itself, inside a bend tighter than it is far out.
                if (line[j] - line[i]).dot(samples[j].position - samples[i].position) <= 0.0 {
                    continue;
                }
                let mut along = owed;
                while along < length {
                    let share = along / length.max(1.0e-3);
                    along += TILE * PITCH;
                    let at = line[i].lerp(line[j], share);
                    let away = outward[i].lerp(outward[j], share).normalize_or_zero();
                    let (x, z) = (at.x, at.z);
                    // On dry, gentle ground, and beside this stretch of road rather than
                    // nearer to another, whose own rows cover it.
                    let wanted = out(i) + (out(j) - out(i)) * share;
                    let here = road.beside(x, z);
                    if land.at(x, z) < 2.0
                        || land.slope(x, z) > 0.7
                        || here.is_none_or(|b| (b.offset.abs() - wanted).abs() > 1.6)
                    {
                        continue;
                    }
                    let number = hash(
                        i as i32,
                        (along * 8.0) as i32,
                        80 + row as u32 + 16 * which as u32,
                    );
                    let mesh = if row == 0 {
                        WHOLE + number % EDGED
                    } else if row == rows - 1 {
                        WHOLE + EDGED + number % EDGED
                    } else {
                        number % WHOLE
                    };
                    // Sheared to the ground's slope, so that every blade still grows straight
                    // up and every root touches the ground.
                    let step = TILE * 0.5;
                    let slope_x = (land.at(x + step, z) - land.at(x - step, z)) / (2.0 * step);
                    let slope_z = (land.at(x, z + step) - land.at(x, z - step)) / (2.0 * step);
                    let shear = Mat4::from_cols(
                        Vec4::new(1.0, slope_x, 0.0, 0.0),
                        Vec4::Y,
                        Vec4::new(0.0, slope_z, 1.0, 0.0),
                        Vec4::W,
                    );
                    // The patch's own x is away from the road, and its z along it.
                    let lying = Mat4::from_cols(
                        away.extend(0.0),
                        Vec4::Y,
                        away.cross(Vec3::Y).extend(0.0),
                        Vec4::W,
                    );
                    scene.instances.push(Instance {
                        mesh: first + mesh,
                        material: blade,
                        transform: Mat4::from_translation(Vec3::new(x, land.at(x, z), z))
                            * shear
                            * lying
                            * Mat4::from_translation(Vec3::new(-step, 0.0, -step)),
                    });
                    patches += 1;
                }
                owed = along - length;
            }
        }
    }
    println!(
        "    {patches} patches of {TILE} m in {rows} rows a side, {} triangles each",
        triangles / (WHOLE + 2 * EDGED) as usize
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_patch_thinned_along_one_side_keeps_whole_blades_and_ends_in_a_wandering_line() {
        let meadow = Meadow {
            size: TILE,
            ..Meadow::by_name("verge").unwrap()
        };
        let whole = meadow.mesh();
        let blades = blades_of(&whole);
        assert!(blades.len() > 2000, "{} blades", blades.len());
        // Every triangle belongs to one blade, and a blade is a few triangles, not the patch.
        let total: usize = blades.iter().map(|blade| blade.0.len()).sum();
        assert_eq!(total, whole.triangle_count());
        let largest = blades.iter().map(|blade| blade.0.len()).max().unwrap();
        assert!(
            largest < whole.triangle_count() / 50,
            "a blade of {largest} triangles"
        );

        let cut = thinned(&whole, |root, _| root.x > edge(root.z, 4));
        assert!(cut.triangle_count() < whole.triangle_count());
        assert!(cut.triangle_count() > whole.triangle_count() / 2);
        // No root is left on the far side of the line, and the line wanders.
        for blade in blades_of(&cut) {
            assert!(blade.1.x > edge(blade.1.z, 4) - 1.0e-3);
        }
        let reach: Vec<f32> = (0..=30).map(|k| edge(k as f32 * 0.1, 4)).collect();
        let (least, most) = reach
            .iter()
            .fold((f32::MAX, f32::MIN), |(l, m), &r| (l.min(r), m.max(r)));
        assert!(
            most - least > 0.2 && least > -0.4 && most < 1.5,
            "from {least} to {most}"
        );
        // And it is the same distance in at both ends of every patch, so that it carries on.
        for patch in 0..6 {
            assert!((edge(0.0, patch) - edge(TILE, patch)).abs() < 1.0e-4);
            assert!((edge(0.0, patch) - 0.55).abs() < 1.0e-4);
        }
    }
}
