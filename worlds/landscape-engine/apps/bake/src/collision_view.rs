//! `--show-collision`: a scene with its meshes replaced by what the player actually bumps into,
//! to look at in any viewer. Blue is triangle meshes, green convex hulls, grey the ground, and red
//! the invisible walls.

use glam::{Mat4, Quat, Vec2, Vec3};
use solstice_asset::{CollisionShape, Heightfield, Instance, Material, SceneAsset, NO_SHAPE};
use solstice_builder::{build_virtual_mesh, BuildOptions, Mesh, Vertex};
use solstice_game::{SceneGame, GAME_FILE};

const MESH: u32 = 0;
const HULL: u32 = 1;
const GROUND: u32 = 2;
const BLOCKER: u32 = 3;

fn material(red: f32, green: f32, blue: f32) -> Material {
    Material {
        base_color: [red, green, blue, 1.0],
        roughness: 0.8,
        ..Default::default()
    }
}

fn to_mesh(vertices: &[[f32; 3]], triangles: &[[u32; 3]]) -> Mesh {
    let mut mesh = Mesh {
        vertices: vertices
            .iter()
            .map(|p| Vertex {
                position: Vec3::from_array(*p),
                normal: Vec3::Y,
                uv: Vec2::ZERO,
            })
            .collect(),
        indices: triangles.iter().flatten().copied().collect(),
    };
    mesh.compute_smooth_normals();
    mesh
}

/// A hull's points as a small cross at each point: enough to see where it reaches. A true hull
/// is computed only when the game loads.
fn hull_points(shape: &CollisionShape) -> Mesh {
    const SIZE: f32 = 0.01;
    let mut vertices = Vec::new();
    let mut triangles = Vec::new();
    for p in &shape.vertices {
        let p = Vec3::from_array(*p);
        let base = vertices.len() as u32;
        for corner in [Vec3::X, -Vec3::X, Vec3::Y, -Vec3::Y, Vec3::Z, -Vec3::Z] {
            vertices.push((p + corner * SIZE).to_array());
        }
        for [a, b, c] in [
            [0, 2, 4],
            [2, 1, 4],
            [1, 3, 4],
            [3, 0, 4],
            [2, 0, 5],
            [1, 2, 5],
            [3, 1, 5],
            [0, 3, 5],
        ] {
            triangles.push([base + a, base + b, base + c]);
        }
    }
    to_mesh(&vertices, &triangles)
}

fn ground_mesh(ground: &Heightfield) -> Mesh {
    let (rows, columns) = (ground.rows, ground.columns);
    let mut vertices = Vec::with_capacity((rows * columns) as usize);
    for row in 0..rows {
        for column in 0..columns {
            vertices.push([
                ground.origin[0] + column as f32 * ground.spacing[0],
                ground.heights[(row * columns + column) as usize],
                ground.origin[1] + row as f32 * ground.spacing[1],
            ]);
        }
    }
    let mut triangles = Vec::new();
    for row in 0..rows - 1 {
        for column in 0..columns - 1 {
            let a = row * columns + column;
            let (b, c, d) = (a + 1, a + columns, a + columns + 1);
            triangles.push([a, c, b]);
            triangles.push([b, c, d]);
        }
    }
    to_mesh(&vertices, &triangles)
}

fn unit_box() -> Mesh {
    let vertices: Vec<[f32; 3]> = (0..8)
        .map(|i| {
            [
                if i & 1 == 0 { -0.5 } else { 0.5 },
                if i & 2 == 0 { -0.5 } else { 0.5 },
                if i & 4 == 0 { -0.5 } else { 0.5 },
            ]
        })
        .collect();
    let triangles = [
        [0, 2, 1],
        [1, 2, 3],
        [4, 5, 6],
        [5, 7, 6],
        [0, 1, 4],
        [1, 5, 4],
        [2, 6, 3],
        [3, 6, 7],
        [0, 4, 2],
        [2, 4, 6],
        [1, 3, 5],
        [3, 7, 5],
    ];
    to_mesh(&vertices, &triangles)
}

/// A scene of what `scene`'s collision is, or why there is none to show.
pub fn collision_scene(scene: &SceneAsset) -> Result<SceneAsset, String> {
    let collision = solstice_builder::collision::build_collision(scene)
        .ok_or("this scene has no gameplay, so no collision to show")?;
    let options = BuildOptions::default();
    let mut view = SceneAsset {
        texture_size: scene.texture_size,
        viewpoint: scene.viewpoint,
        lights: scene.lights.clone(),
        materials: vec![
            material(0.25, 0.45, 0.85),
            material(0.3, 0.75, 0.35),
            material(0.5, 0.5, 0.5),
            material(0.9, 0.15, 0.1),
        ],
        ..Default::default()
    };
    view.meshes = collision
        .shapes
        .iter()
        .map(|shape| {
            let mesh = if shape.hull {
                hull_points(shape)
            } else {
                to_mesh(&shape.vertices, &shape.triangles)
            };
            build_virtual_mesh(&mesh, &options)
        })
        .collect();
    for (index, &shape) in collision.instance_shapes.iter().enumerate() {
        if shape != NO_SHAPE {
            view.instances.push(Instance {
                mesh: shape,
                material: if collision.shapes[shape as usize].hull {
                    HULL
                } else {
                    MESH
                },
                transform: scene.instances[index].transform,
            });
        }
    }
    if let Some(ground) = &collision.heightfield {
        view.instances.push(Instance {
            mesh: view.meshes.len() as u32,
            material: GROUND,
            transform: Mat4::IDENTITY,
        });
        view.meshes
            .push(build_virtual_mesh(&ground_mesh(ground), &options));
    }
    let game = scene.gameplay.iter().find(|f| f.name == GAME_FILE);
    let game = game.map_or(Ok(SceneGame::default()), |f| SceneGame::parse(&f.text));
    let game = game.map_err(|e| format!("{GAME_FILE}: {e}"))?;
    if !game.blockers.is_empty() {
        let mesh = view.meshes.len() as u32;
        view.meshes.push(build_virtual_mesh(&unit_box(), &options));
        for blocker in &game.blockers {
            view.instances.push(Instance {
                mesh,
                material: BLOCKER,
                transform: Mat4::from_scale_rotation_translation(
                    blocker.size,
                    Quat::from_rotation_y(blocker.yaw_degrees.to_radians()),
                    blocker.centre,
                ),
            });
        }
    }
    Ok(view)
}

#[cfg(test)]
mod tests {
    use super::*;
    use solstice_asset::GameFile;

    #[test]
    fn it_shows_each_kind_of_shape_where_it_is() {
        let square = Mesh {
            vertices: [
                [-1.0, 0.0, -1.0],
                [1.0, 0.0, -1.0],
                [1.0, 0.0, 1.0],
                [-1.0, 0.0, 1.0],
            ]
            .iter()
            .map(|p| Vertex {
                position: Vec3::from_array(*p),
                normal: Vec3::Y,
                uv: Vec2::ZERO,
            })
            .collect(),
            indices: vec![0, 2, 1, 0, 3, 2],
        };
        let at = Mat4::from_translation(Vec3::new(5.0, 0.0, 0.0));
        let scene = SceneAsset {
            meshes: vec![build_virtual_mesh(&square, &BuildOptions::default())],
            instances: vec![Instance {
                mesh: 0,
                material: 0,
                transform: at,
            }],
            gameplay: vec![GameFile {
                name: GAME_FILE.to_owned(),
                text: "blocker 1 2 3 4 5 6 90\n".to_owned(),
            }],
            ground: Some(Heightfield {
                columns: 2,
                rows: 2,
                origin: [0.0, 0.0],
                spacing: [1.0, 1.0],
                heights: vec![0.0; 4],
            }),
            ..Default::default()
        };
        let view = collision_scene(&scene).unwrap();
        let materials: Vec<u32> = view.instances.iter().map(|i| i.material).collect();
        assert_eq!(
            materials,
            [MESH, GROUND, BLOCKER],
            "a 2 m square is not a small prop"
        );
        assert_eq!(view.instances[0].transform, at);
        let blocker = view.instances[2].transform.transform_point3(Vec3::ZERO);
        assert!(blocker.abs_diff_eq(Vec3::new(1.0, 2.0, 3.0), 1.0e-5));
        // Three shapes to draw, and each instance names one that exists.
        assert_eq!(view.meshes.len(), 3);
        assert!(view
            .instances
            .iter()
            .all(|i| (i.mesh as usize) < view.meshes.len()));
        assert!(collision_scene(&SceneAsset::default()).is_err());
    }
}
