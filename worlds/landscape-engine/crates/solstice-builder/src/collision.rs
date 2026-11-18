//! Makes a scene's collision shapes (docs/gameplay.md, Colliding with the scene).
//!
//! Each mesh's collision is a cut of its own cluster DAG at a few centimetres of error: the
//! shapes the player touches match what is drawn, without the full detail. Small props get a
//! convex hull of the same cut instead. One shape serves every instance of a mesh.

use glam::Vec3;
use solstice_asset::gpu::unpack_triangle;
use solstice_asset::{
    Collide, Collision, CollisionShape, SceneAsset, VirtualMesh, MAX_HULL_POINTS, NO_SHAPE,
};
use std::collections::HashMap;

/// How far a collision mesh may be from what is drawn, in metres.
pub const MESH_ERROR: f32 = 0.05;
/// The same, for very large meshes marked `collide=coarse`, such as ground.
pub const COARSE_ERROR: f32 = 0.10;
/// Meshes with a bounding sphere smaller than this, in metres, get a convex hull.
pub const SMALL_PROP_RADIUS: f32 = 0.75;

/// The triangles of a cut through a mesh's DAG at `error`, in the mesh's own space. Those
/// clusters are exactly the ones fine enough, whose parents are not: together they make one
/// watertight surface.
pub fn cut(mesh: &VirtualMesh, error: f32) -> (Vec<[f32; 3]>, Vec<[u32; 3]>) {
    let mut vertices = Vec::new();
    let mut triangles = Vec::new();
    let mut indices: HashMap<[u16; 3], u32> = HashMap::new();
    for node in mesh.nodes.iter().filter(|n| n.is_leaf != 0) {
        if node.max_parent_error <= error {
            continue;
        }
        for cluster in &mesh.clusters[node.first as usize..(node.first + node.count) as usize] {
            if cluster.lod_error > error {
                continue;
            }
            let base = cluster.vertex_offset as usize;
            let first = cluster.triangle_offset as usize;
            for packed in &mesh.triangles[first..first + cluster.triangle_count() as usize] {
                let corner = unpack_triangle(*packed).map(|local| {
                    let data = mesh.vertices[base + usize::from(local)].data;
                    let grid = [data[0], data[1], data[2]];
                    *indices.entry(grid).or_insert_with(|| {
                        let p = mesh.quant_origin
                            + Vec3::new(f32::from(grid[0]), f32::from(grid[1]), f32::from(grid[2]))
                                * mesh.quant_step;
                        vertices.push(p.to_array());
                        vertices.len() as u32 - 1
                    })
                });
                // Welding on the grid can close up a sliver, which collision can't take.
                if corner[0] != corner[1] && corner[1] != corner[2] && corner[0] != corner[2] {
                    triangles.push(corner);
                }
            }
        }
    }
    (vertices, triangles)
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Kind {
    Mesh,
    Coarse,
    Hull,
}

/// The largest scale of a transform's axes.
fn largest_scale(transform: &glam::Mat4) -> f32 {
    let (x, y, z) = (transform.x_axis, transform.y_axis, transform.z_axis);
    x.truncate()
        .length()
        .max(y.truncate().length())
        .max(z.truncate().length())
}

/// A scene's collision, if it has gameplay to use it. Without, it has none, and costs nothing.
pub fn build_collision(scene: &SceneAsset) -> Option<Collision> {
    if scene.gameplay.is_empty() {
        return None;
    }
    let kind_of = |index: usize, mesh: &VirtualMesh, scale: f32| match scene
        .collide
        .get(index)
        .copied()
        .unwrap_or_default()
    {
        Collide::None => None,
        Collide::Hull => Some(Kind::Hull),
        Collide::Coarse => Some(Kind::Coarse),
        Collide::Auto if mesh.bounds_radius * scale < SMALL_PROP_RADIUS => Some(Kind::Hull),
        Collide::Auto => Some(Kind::Mesh),
    };
    // Error is in the mesh's own space, so the largest instance decides how fine a cut it needs.
    let mut largest: HashMap<(u32, Kind), f32> = HashMap::new();
    for (index, instance) in scene.instances.iter().enumerate() {
        let scale = largest_scale(&instance.transform);
        if let Some(kind) = kind_of(index, &scene.meshes[instance.mesh as usize], scale) {
            let entry = largest.entry((instance.mesh, kind)).or_insert(0.0);
            *entry = entry.max(scale);
        }
    }
    let mut made: HashMap<(u32, Kind), u32> = HashMap::new();
    let mut shapes = Vec::new();
    let mut instance_shapes = Vec::with_capacity(scene.instances.len());
    for (index, instance) in scene.instances.iter().enumerate() {
        let mesh = &scene.meshes[instance.mesh as usize];
        let Some(kind) = kind_of(index, mesh, largest_scale(&instance.transform)) else {
            instance_shapes.push(NO_SHAPE);
            continue;
        };
        let key = (instance.mesh, kind);
        let shape = *made.entry(key).or_insert_with(|| {
            shapes.push(make_shape(mesh, kind, largest[&key]));
            shapes.len() as u32 - 1
        });
        instance_shapes.push(shape);
    }
    Some(Collision {
        shapes,
        instance_shapes,
        heightfield: scene.ground.clone(),
    })
}

/// At most [`MAX_HULL_POINTS`] of these points, with those furthest out in every direction, which
/// are what shape a hull, and the rest evenly from what is left.
fn thin_hull(points: Vec<[f32; 3]>) -> Vec<[f32; 3]> {
    if points.len() <= MAX_HULL_POINTS {
        return points;
    }
    const DIRECTIONS: usize = MAX_HULL_POINTS / 2;
    let mut keep = vec![false; points.len()];
    // Directions spread over a sphere, along a spiral.
    let golden = std::f32::consts::PI * (3.0 - 5.0_f32.sqrt());
    for i in 0..DIRECTIONS {
        let y = 1.0 - 2.0 * (i as f32 + 0.5) / DIRECTIONS as f32;
        let radius = (1.0 - y * y).max(0.0).sqrt();
        let (sin, cos) = (golden * i as f32).sin_cos();
        let direction = Vec3::new(cos * radius, y, sin * radius);
        let furthest = points
            .iter()
            .enumerate()
            .max_by(|(_, a), (_, b)| {
                let along = |p: &[f32; 3]| Vec3::from_array(*p).dot(direction);
                along(a).total_cmp(&along(b))
            })
            .map(|(index, _)| index);
        if let Some(index) = furthest {
            keep[index] = true;
        }
    }
    let kept = keep.iter().filter(|k| **k).count();
    let rest = points.len() - kept;
    let stride = (rest / (MAX_HULL_POINTS - kept).max(1)).max(1);
    let mut others = 0;
    let mut extremes_left = kept;
    let mut out = Vec::with_capacity(MAX_HULL_POINTS);
    for (point, keep) in points.into_iter().zip(keep) {
        if keep {
            out.push(point);
            extremes_left -= 1;
        } else {
            // Room is held for the extremes still to come.
            if others % stride == 0 && out.len() + extremes_left < MAX_HULL_POINTS {
                out.push(point);
            }
            others += 1;
        }
    }
    out
}

fn make_shape(mesh: &VirtualMesh, kind: Kind, scale: f32) -> CollisionShape {
    let world_error = if kind == Kind::Coarse {
        COARSE_ERROR
    } else {
        MESH_ERROR
    };
    let (mut vertices, triangles) = cut(mesh, world_error / scale.max(1.0e-6));
    if kind == Kind::Hull {
        vertices = thin_hull(vertices);
        return CollisionShape {
            hull: true,
            vertices,
            triangles: Vec::new(),
        };
    }
    CollisionShape {
        hull: false,
        vertices,
        triangles,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{build_virtual_mesh, BuildOptions, Mesh, Vertex};
    use glam::{Mat4, Quat, Vec2};
    use solstice_asset::{GameFile, Instance};

    /// A rippled sheet in the XZ plane, `size` across, with `cells` squares a side.
    fn sheet(size: f32, cells: u32, ripple: f32) -> VirtualMesh {
        let mut mesh = Mesh::default();
        let row = cells + 1;
        for z in 0..row {
            for x in 0..row {
                let (u, v) = (x as f32 / cells as f32, z as f32 / cells as f32);
                let height = (u * 40.0).sin() * (v * 37.0).cos() * ripple;
                mesh.vertices.push(Vertex {
                    position: Vec3::new((u - 0.5) * size, height, (v - 0.5) * size),
                    normal: Vec3::Y,
                    uv: Vec2::new(u, v),
                });
            }
        }
        for z in 0..cells {
            for x in 0..cells {
                let a = z * row + x;
                mesh.indices
                    .extend([a, a + row, a + 1, a + 1, a + row, a + row + 1]);
            }
        }
        build_virtual_mesh(&mesh, &BuildOptions::default())
    }

    fn game() -> Vec<GameFile> {
        vec![GameFile {
            name: "scene.game".to_owned(),
            text: String::new(),
        }]
    }

    fn at(mesh: u32, scale: f32, x: f32) -> Instance {
        Instance {
            mesh,
            material: 0,
            transform: Mat4::from_scale_rotation_translation(
                Vec3::splat(scale),
                Quat::IDENTITY,
                Vec3::X * x,
            ),
        }
    }

    #[test]
    fn a_coarser_cut_has_fewer_triangles_and_still_covers_the_mesh() {
        let mesh = sheet(10.0, 64, 0.02);
        let finest = cut(&mesh, 0.0);
        assert_eq!(finest.1.len(), 64 * 64 * 2, "error zero is the mesh itself");
        let (vertices, triangles) = cut(&mesh, 0.5);
        assert!(triangles.len() < finest.1.len() / 4, "{}", triangles.len());
        // Still reaches every corner of the sheet.
        let reach = |axis: usize| {
            let values = vertices.iter().map(|v| v[axis]);
            (
                values.clone().fold(f32::MAX, f32::min),
                values.fold(f32::MIN, f32::max),
            )
        };
        for axis in [0, 2] {
            let (low, high) = reach(axis);
            assert!(
                (low + 5.0).abs() < 1.0e-2 && (high - 5.0).abs() < 1.0e-2,
                "{low} {high}"
            );
        }
        // And is one watertight surface: every inner edge has two triangles, so no border
        // but the sheet's own, whatever the clusters chosen.
        let mut edges: HashMap<(u32, u32), i32> = HashMap::new();
        for t in &triangles {
            for (a, b) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
                *edges.entry((a.min(b), a.max(b))).or_default() += 1;
            }
        }
        let on_border = |v: u32| {
            let p = vertices[v as usize];
            p[0].abs() > 4.99 || p[2].abs() > 4.99
        };
        for (&(a, b), &count) in &edges {
            if !(on_border(a) && on_border(b)) {
                assert_eq!(count, 2, "edge {a}-{b} is open");
            }
        }
    }

    #[test]
    fn no_gameplay_means_no_collision() {
        let scene = SceneAsset {
            meshes: vec![sheet(10.0, 8, 0.0)],
            instances: vec![at(0, 1.0, 0.0)],
            ..Default::default()
        };
        assert!(build_collision(&scene).is_none());
    }

    #[test]
    fn instances_share_a_shape_and_scenes_choose_how_each_collides() {
        let scene = SceneAsset {
            meshes: vec![sheet(10.0, 32, 0.02), sheet(0.4, 8, 0.0)],
            instances: vec![
                at(0, 1.0, 0.0),  // a big mesh: triangles
                at(0, 1.0, 20.0), // shares them
                at(1, 1.0, 40.0), // a small prop: a hull
                at(0, 1.0, 60.0), // collide=none
                at(0, 1.0, 80.0), // collide=hull
                at(0, 1.0, 90.0), // collide=coarse
            ],
            collide: vec![
                Collide::Auto,
                Collide::Auto,
                Collide::Auto,
                Collide::None,
                Collide::Hull,
                Collide::Coarse,
            ],
            gameplay: game(),
            ..Default::default()
        };
        let collision = build_collision(&scene).unwrap();
        let s = &collision.instance_shapes;
        assert_eq!(s[0], s[1], "a mesh's shape is shared");
        assert_eq!(s[3], NO_SHAPE);
        let shape = |i: usize| &collision.shapes[s[i] as usize];
        assert!(!shape(0).hull && !shape(0).triangles.is_empty());
        assert!(shape(2).hull && shape(2).triangles.is_empty());
        assert!(shape(4).hull);
        assert!(!shape(5).hull && shape(5).triangles.len() <= shape(0).triangles.len());
        assert_eq!(collision.shapes.len(), 4);
    }

    #[test]
    fn a_scaled_instance_is_cut_to_the_same_error_in_the_world() {
        // The same mesh, ten times the size: its cut must be finer in its own space.
        let mesh = sheet(1.0, 64, 0.02);
        let small = SceneAsset {
            meshes: vec![mesh.clone()],
            instances: vec![at(0, 1.0, 0.0)],
            collide: vec![Collide::Coarse],
            gameplay: game(),
            ..Default::default()
        };
        let large = SceneAsset {
            instances: vec![at(0, 10.0, 0.0)],
            ..small.clone()
        };
        let count = |scene: &SceneAsset| build_collision(scene).unwrap().shapes[0].triangles.len();
        assert!(
            count(&large) > count(&small),
            "{} {}",
            count(&large),
            count(&small)
        );
    }

    #[test]
    fn collide_coarse_keeps_fewer_triangles() {
        // Ripples of about the two errors' size, so the two cuts differ.
        let mesh = sheet(10.0, 128, 0.08);
        let build = |how: Collide| {
            let scene = SceneAsset {
                meshes: vec![mesh.clone()],
                instances: vec![at(0, 1.0, 0.0)],
                collide: vec![how],
                gameplay: game(),
                ..Default::default()
            };
            build_collision(&scene).unwrap().shapes[0].triangles.len()
        };
        assert!(build(Collide::Coarse) < build(Collide::Auto));
    }

    #[test]
    fn hulls_are_thinned_to_a_limit() {
        let scene = SceneAsset {
            meshes: vec![sheet(0.4, 128, 0.0)],
            instances: vec![at(0, 1.0, 0.0)],
            collide: vec![Collide::Hull],
            gameplay: game(),
            ..Default::default()
        };
        let collision = build_collision(&scene).unwrap();
        assert!(collision.shapes[0].vertices.len() <= MAX_HULL_POINTS);
        assert!(collision.shapes[0].vertices.len() >= 4);
    }

    #[test]
    fn thinning_a_hull_keeps_its_corners() {
        // A grid of points, whose four corners shape its hull, buried among thousands.
        let mut points = Vec::new();
        for z in 0..100 {
            for x in 0..100 {
                points.push([x as f32 / 99.0, 0.0, z as f32 / 99.0]);
            }
        }
        let thin = thin_hull(points);
        assert!(thin.len() <= MAX_HULL_POINTS, "{}", thin.len());
        for corner in [
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0],
            [1.0, 0.0, 1.0],
        ] {
            assert!(thin.contains(&corner), "lost {corner:?}");
        }
        assert_eq!(
            thin_hull(vec![[1.0; 3]; 3]).len(),
            3,
            "a few points are left alone"
        );
    }

    #[test]
    fn the_ground_is_carried_along() {
        let ground = solstice_asset::Heightfield {
            columns: 2,
            rows: 2,
            spacing: [1.0, 1.0],
            heights: vec![0.0; 4],
            ..Default::default()
        };
        let scene = SceneAsset {
            ground: Some(ground.clone()),
            gameplay: game(),
            ..Default::default()
        };
        assert_eq!(build_collision(&scene).unwrap().heightfield, Some(ground));
    }
}
