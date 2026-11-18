//! Imports a glTF 2.0 model as a [`SceneAsset`].
//!
//! Each glTF primitive becomes one virtual mesh, since the renderer has one material per
//! instance. Primitives of one model therefore cannot crack apart at different levels of
//! detail: the outer edges of a mesh never move during simplification.
//!
//! Features the renderer cannot show yet are reported in [`Imported::warnings`], never
//! silently dropped.

pub mod animation;
pub mod character;
pub mod ktx2;
mod material;
pub mod meshopt;
pub mod streamed;

use glam::{Mat3, Mat4, Vec2, Vec3};
use gltf::{buffer, Document, Node, Primitive};
use material::MaterialTable;
use rayon::prelude::*;
use solstice_asset::scene::Viewpoint;
use solstice_asset::skeleton::MAX_BONES;
use solstice_asset::{Bone, PackedSkin, Skeleton, NO_POSE};
use solstice_asset::{Instance, Material, SceneAsset, VirtualMesh};
use solstice_builder::{
    build_skinned_mesh, build_virtual_mesh, BuildOptions, Mesh, Sphere, Vertex,
};
use std::collections::HashMap;
use std::fmt;
use std::path::Path;

/// How to bring a model into a scene.
#[derive(Clone, Copy, Debug)]
pub struct ImportOptions {
    /// Edge length of all textures. They share one texture array, so every source image is
    /// resampled to this size. Must be a power of two.
    pub texture_size: u32,
    /// Uniformly scales the model so its longest side is this many metres, centres it, and
    /// stands it on its ground height (see [`ground_height`]). The GI system works in fixed
    /// world-space sizes, so models in other units, or of very different size, should use this.
    pub fit_size: Option<f32>,
    /// Adds a large flat ground plane at the model's ground height (see [`ground_height`]).
    pub ground: bool,
    /// Keeps the model's skeleton and weights, so that it can bend. The file must hold exactly
    /// one skin, which every mesh uses. Otherwise a skinned model imports as a statue.
    pub skinned: bool,
}

impl Default for ImportOptions {
    fn default() -> Self {
        Self {
            texture_size: 1024,
            fit_size: None,
            ground: false,
            skinned: false,
        }
    }
}

/// A successful import.
#[derive(Debug)]
pub struct Imported {
    pub scene: SceneAsset,
    /// The instances of each named node, at the node's own origin, for placing the pieces of
    /// a modular kit one by one. Mesh and material numbers refer to `scene`.
    pub parts: HashMap<String, Vec<Instance>>,
    /// What the node of each of `scene`'s instances is called, in their order, or nothing
    /// for one with no name: so that a thing made of named pieces, as a truck is, can have
    /// some shown and some not.
    pub names: Vec<String>,
    /// Things in the file that the result does not reproduce.
    pub warnings: Vec<String>,
}

/// Why a model could not be imported.
#[derive(Debug)]
pub enum ImportError {
    Gltf(gltf::Error),
    InvalidOptions(String),
    /// The file is readable but holds nothing that can be drawn.
    NothingToDraw,
}

impl fmt::Display for ImportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Gltf(error) => write!(f, "could not read the glTF file: {error}"),
            Self::InvalidOptions(reason) => write!(f, "invalid import options: {reason}"),
            Self::NothingToDraw => write!(f, "the default scene contains no triangle meshes"),
        }
    }
}

impl std::error::Error for ImportError {}

/// One placement found by walking the node tree.
struct Placement {
    node_name: Option<String>,
    source: MeshKey,
    /// Whether the node is bound to a skin, in which case glTF ignores where the node stands.
    bound: bool,
    material: Option<usize>,
    transform: Mat4,
}

/// Identifies a mesh to build. Mirrored placements need the opposite winding to stay
/// front-facing, so the same primitive may have to be built twice.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct MeshKey {
    pub(crate) mesh: usize,
    pub(crate) primitive: usize,
    pub(crate) mirrored: bool,
}

/// Imports the file at `path`.
pub fn import_gltf(
    path: impl AsRef<Path>,
    options: &ImportOptions,
) -> Result<Imported, ImportError> {
    if !options.texture_size.is_power_of_two() {
        return Err(ImportError::InvalidOptions(format!(
            "texture size {} is not a power of two",
            options.texture_size
        )));
    }
    let (document, buffers, images) = gltf::import(path).map_err(ImportError::Gltf)?;
    let mut warnings = document_warnings(&document, options.skinned);
    let rig = if options.skinned {
        Some(read_rig(&document, &buffers)?)
    } else {
        None
    };

    let mut placements = Vec::new();
    let mut bounds = Bounds::default();
    if let Some(scene) = document
        .default_scene()
        .or_else(|| document.scenes().next())
    {
        for node in scene.nodes() {
            walk(
                &node,
                Mat4::IDENTITY,
                options.skinned,
                &mut placements,
                &mut bounds,
                &mut warnings,
            );
        }
    }
    if placements.is_empty() {
        return Err(ImportError::NothingToDraw);
    }

    let root = root_transform(&bounds, options.fit_size);
    let world_bounds = bounds.transformed(root);

    let mut materials = MaterialTable::new(&document, &images, options.texture_size);
    let mut mesh_slots: HashMap<MeshKey, u32> = HashMap::new();
    let mut keys: Vec<MeshKey> = Vec::new();
    let mut instances = Vec::with_capacity(placements.len());
    let mut parts: HashMap<String, Vec<Instance>> = HashMap::new();
    let mut names = Vec::with_capacity(placements.len());
    for placement in &placements {
        names.push(placement.node_name.clone().unwrap_or_default());
        let material = materials.slot(&document, placement.material, &mut warnings);
        // A skinned mesh is drawn as its skeleton says: glTF ignores where its node stands.
        let skinned = options.skinned && placement.bound;
        let source = MeshKey {
            mirrored: placement.source.mirrored && !skinned,
            ..placement.source
        };
        let mesh = *mesh_slots.entry(source).or_insert_with(|| {
            keys.push(source);
            keys.len() as u32 - 1
        });
        instances.push(Instance {
            mesh,
            material,
            transform: if skinned {
                root
            } else {
                root * placement.transform
            },
        });
        if let Some(name) = &placement.node_name {
            parts.entry(name.clone()).or_default().push(Instance {
                mesh,
                material,
                transform: Mat4::IDENTITY,
            });
        }
    }

    let build_options = BuildOptions::default();
    let read = keys
        .par_iter()
        .map(|key| -> Result<_, ImportError> {
            let primitive = document
                .meshes()
                .nth(key.mesh)
                .and_then(|mesh| mesh.primitives().nth(key.primitive))
                .expect("keys come from walking this document");
            let mesh = read_primitive(&primitive, &buffers, key.mirrored);
            let skins = rig
                .as_ref()
                .map(|rig| read_skins(&primitive, &buffers, rig))
                .transpose()?;
            Ok((mesh, skins))
        })
        .collect::<Result<Vec<_>, ImportError>>()?;
    let mut meshes: Vec<VirtualMesh> = read
        .par_iter()
        .map(|(mesh, skins)| match skins {
            Some(skins) => build_skinned_mesh(mesh, skins, &build_options),
            None => build_virtual_mesh(mesh, &build_options),
        })
        .collect();
    let skeleton = rig.map(|mut rig| {
        measure_influences(&mut rig.skeleton, &read);
        rig.skeleton
    });

    let (mut material_list, textures) = materials.finish();
    if options.ground {
        // Large enough that distance haze hides where it ends.
        let extent = (world_bounds.longest_side() * 12.0).max(1500.0);
        let ground_material = material_list.len() as u32;
        material_list.push(Material {
            base_color: [0.42, 0.40, 0.37, 1.0],
            roughness: 0.9,
            ..Default::default()
        });
        instances.push(Instance {
            mesh: meshes.len() as u32,
            material: ground_material,
            transform: Mat4::from_translation(Vec3::new(
                world_bounds.center().x,
                root.transform_point3(Vec3::Y * ground_height(bounds.min.y, bounds.max.y))
                    .y,
                world_bounds.center().z,
            )),
        });
        meshes.push(build_virtual_mesh(&ground_plane(extent), &build_options));
    }

    let poses = usize::from(skeleton.is_some());
    let instance_poses = if poses > 0 {
        instances
            .iter()
            .map(|i| {
                if meshes[i.mesh as usize].skinned() {
                    0
                } else {
                    NO_POSE
                }
            })
            .collect()
    } else {
        Vec::new()
    };

    Ok(Imported {
        scene: SceneAsset {
            meshes,
            materials: material_list,
            instances,
            lights: Vec::new(),
            texture_size: options.texture_size,
            texture_encoding: Default::default(),
            textures,
            viewpoint: Some(frame_bounds(&world_bounds)),
            sea: None,
            gameplay: Vec::new(),
            collide: Vec::new(),
            ground: None,
            skeletons: skeleton.into_iter().collect(),
            poses: vec![0; poses],
            instance_poses,
            ..Default::default()
        },
        parts,
        names,
        warnings,
    })
}

/// Where to put a model, or pieces of it, in a composed scene.
#[derive(Clone, Debug, Default)]
pub struct ModelPlacements {
    /// Each entry places the whole model, with its internal layout.
    pub whole: Vec<Mat4>,
    /// Each entry places one named piece, by its own origin.
    pub parts: Vec<(String, Mat4)>,
}

/// Adds a model's meshes, materials and textures to `target` once, then instances as
/// `placements` asks. A part's name may leave off a prefix: `wall_01` finds `kit_wall_01`.
///
/// Textures in `target` and `model` must be the same size, since they share one array.
/// Returns the instances each placement became, the whole placements then the parts, or an error
/// naming any part the model does not have.
pub fn append_model(
    target: &mut SceneAsset,
    model: Imported,
    placements: &ModelPlacements,
) -> Result<Vec<std::ops::Range<u32>>, String> {
    let Imported {
        scene: source,
        parts,
        ..
    } = model;
    assert!(
        source.textures.is_empty()
            || target.textures.is_empty()
            || source.texture_size == target.texture_size,
        "all textures in a scene must be the same size"
    );
    let mesh_base = target.meshes.len() as u32;
    // Each placement of a skinned model is a character of its own, with its own pose.
    let skeleton_base = target.skeletons.len() as u32;
    let bends = !source.skeletons.is_empty();
    let skinned: Vec<bool> = source.meshes.iter().map(VirtualMesh::skinned).collect();
    target.skeletons.extend(source.skeletons);
    let new_pose = |poses: &mut Vec<u32>| -> Option<u32> {
        bends.then(|| {
            poses.push(skeleton_base);
            poses.len() as u32 - 1
        })
    };
    let follow = |instance: &Instance, pose: Option<u32>, poses: &mut Vec<u32>, at: usize| {
        let Some(pose) = pose.filter(|_| skinned[instance.mesh as usize]) else {
            return;
        };
        poses.resize(at, NO_POSE);
        poses.push(pose);
    };
    let material_base = target.materials.len() as u32;
    let layer_base = target.textures.len() as i32;

    target.meshes.extend(source.meshes);
    target
        .materials
        .extend(source.materials.into_iter().map(|mut material| {
            if material.texture_layer >= 0 {
                material.texture_layer += layer_base;
            }
            material
        }));
    target.textures.extend(source.textures);
    target.lights.extend(source.lights);

    let rebase = |instance: &Instance, transform: Mat4| Instance {
        mesh: instance.mesh + mesh_base,
        material: instance.material + material_base,
        transform,
    };
    let mut ranges = Vec::with_capacity(placements.whole.len() + placements.parts.len());
    for placement in &placements.whole {
        let first = target.instances.len() as u32;
        let pose = new_pose(&mut target.poses);
        for instance in &source.instances {
            follow(
                instance,
                pose,
                &mut target.instance_poses,
                target.instances.len(),
            );
            target
                .instances
                .push(rebase(instance, *placement * instance.transform));
        }
        ranges.push(first..target.instances.len() as u32);
    }
    for (name, placement) in &placements.parts {
        let suffix = format!("_{name}");
        let found = parts
            .iter()
            .find(|(candidate, _)| *candidate == name || candidate.ends_with(&suffix))
            .ok_or_else(|| format!("no part named '{name}'"))?;
        let first = target.instances.len() as u32;
        let pose = new_pose(&mut target.poses);
        for instance in found.1 {
            follow(
                instance,
                pose,
                &mut target.instance_poses,
                target.instances.len(),
            );
            target
                .instances
                .push(rebase(instance, *placement * instance.transform));
        }
        ranges.push(first..target.instances.len() as u32);
    }
    Ok(ranges)
}

fn document_warnings(document: &Document, skinned: bool) -> Vec<String> {
    let mut warnings = Vec::new();
    if document.animations().next().is_some() {
        warnings.push("animations are ignored: the model is imported in its rest pose".to_string());
    }
    if document.skins().next().is_some() && !skinned {
        warnings.push("skinning is ignored: skinned meshes are imported unposed".to_string());
    }
    if document
        .extensions_used()
        .any(|name| name == "KHR_lights_punctual")
    {
        warnings
            .push("KHR_lights_punctual is ignored: the file's lights are not imported".to_string());
    }
    warnings
}

fn walk(
    node: &Node<'_>,
    parent: Mat4,
    skinned: bool,
    placements: &mut Vec<Placement>,
    bounds: &mut Bounds,
    warnings: &mut Vec<String>,
) {
    let transform = parent * Mat4::from_cols_array_2d(&node.transform().matrix());
    if let Some(mesh) = node.mesh() {
        let mirrored = Mat3::from_mat4(transform).determinant() < 0.0;
        for primitive in mesh.primitives() {
            if primitive.mode() != gltf::mesh::Mode::Triangles {
                warnings.push(format!(
                    "mesh {} has a primitive of mode {:?}, which was skipped: only triangle lists are supported",
                    mesh.index(),
                    primitive.mode()
                ));
                continue;
            }
            if material::is_invisible(&primitive.material()) {
                let name = primitive.material().name().unwrap_or("unnamed").to_owned();
                let warning = format!(
                    "material '{name}': its alpha is below its cut-off everywhere, so what uses it is left out"
                );
                if !warnings.contains(&warning) {
                    warnings.push(warning);
                }
                continue;
            }
            let box_ = primitive.bounding_box();
            // A skinned mesh is drawn where its skeleton puts it, whatever its node says.
            let drawn = if skinned && node.skin().is_some() {
                Mat4::IDENTITY
            } else {
                transform
            };
            bounds.add_box(Vec3::from(box_.min), Vec3::from(box_.max), drawn);
            placements.push(Placement {
                node_name: node.name().map(str::to_string),
                bound: node.skin().is_some(),
                source: MeshKey {
                    mesh: mesh.index(),
                    primitive: primitive.index(),
                    mirrored,
                },
                material: primitive.material().index(),
                transform,
            });
        }
    }
    for child in node.children() {
        walk(&child, transform, skinned, placements, bounds, warnings);
    }
}

/// A skeleton read from a file, and what the file's joint numbers become.
struct Rig {
    skeleton: Skeleton,
    numbers: Vec<u8>,
}

/// Reads the one skin of a file that is to bend, with its bones renumbered so that parents come
/// before children.
fn read_rig(document: &Document, buffers: &[buffer::Data]) -> Result<Rig, ImportError> {
    let invalid = |reason: String| ImportError::InvalidOptions(reason);
    let skins: Vec<_> = document.skins().collect();
    let [skin] = skins.as_slice() else {
        return Err(invalid(format!(
            "a model that bends has one skin, not {}",
            skins.len()
        )));
    };
    let joints: Vec<Node<'_>> = skin.joints().collect();
    if joints.len() > MAX_BONES {
        return Err(invalid(format!(
            "the skin has {} bones, more than the {MAX_BONES} the engine takes",
            joints.len()
        )));
    }
    let reader = skin.reader(|b| buffers.get(b.index()).map(|d| &d.0[..]));
    let inverse_binds: Vec<Mat4> = reader
        .read_inverse_bind_matrices()
        .ok_or_else(|| invalid("the skin has no inverse bind matrices".to_owned()))?
        .map(|m| Mat4::from_cols_array_2d(&m))
        .collect();
    if inverse_binds.len() != joints.len() {
        return Err(invalid(
            "the skin doesn't have one inverse bind matrix per bone".to_owned(),
        ));
    }

    let mut parent_node = vec![None; document.nodes().len()];
    for node in document.nodes() {
        for child in node.children() {
            parent_node[child.index()] = Some(node.index());
        }
    }
    let local: Vec<Mat4> = document
        .nodes()
        .map(|n| Mat4::from_cols_array_2d(&n.transform().matrix()))
        .collect();
    let world_of = |mut node: usize| {
        let mut world = local[node];
        while let Some(up) = parent_node[node] {
            world = local[up] * world;
            node = up;
        }
        world
    };
    let joint_of_node: HashMap<usize, usize> = joints
        .iter()
        .enumerate()
        .map(|(joint, node)| (node.index(), joint))
        .collect();
    let parent_joint = |joint: usize| {
        let mut node = joints[joint].index();
        while let Some(up) = parent_node[node] {
            if let Some(&found) = joint_of_node.get(&up) {
                return Some(found);
            }
            node = up;
        }
        None
    };
    let depth = |mut joint: usize| {
        let mut depth = 0u32;
        while let Some(up) = parent_joint(joint) {
            depth += 1;
            joint = up;
        }
        depth
    };

    // Parents first: a bone is always deeper than the bone it hangs from.
    let mut order: Vec<usize> = (0..joints.len()).collect();
    order.sort_by_key(|&joint| depth(joint));
    let mut numbers = vec![0u8; joints.len()];
    for (number, &joint) in order.iter().enumerate() {
        numbers[joint] = number as u8;
    }

    let mut bones = Vec::with_capacity(joints.len());
    for &joint in &order {
        let world = world_of(joints[joint].index());
        let parent = parent_joint(joint);
        let local = match parent {
            Some(up) => world_of(joints[up].index()).inverse() * world,
            None => world,
        };
        let (scale, rotation, translation) = local.to_scale_rotation_translation();
        if !scale.abs_diff_eq(Vec3::ONE, 1.0e-3) {
            return Err(invalid(format!(
                "bone `{}` is scaled by {scale}: apply the scale before exporting",
                joints[joint].name().unwrap_or("(unnamed)")
            )));
        }
        bones.push(Bone {
            name: joints[joint].name().unwrap_or("").to_owned(),
            parent: parent.map(|up| u32::from(numbers[up])),
            translation,
            rotation,
            inverse_bind: inverse_binds[joint],
            center: Vec3::ZERO,
            radius: 0.0,
        });
    }
    Ok(Rig {
        skeleton: Skeleton { bones },
        numbers,
    })
}

/// Reads the weights of a primitive that bends, with its joints as the rig numbers them.
fn read_skins(
    primitive: &Primitive<'_>,
    buffers: &[buffer::Data],
    rig: &Rig,
) -> Result<Vec<PackedSkin>, ImportError> {
    let reader = primitive.reader(|b| Some(&buffers[b.index()].0));
    let (Some(joints), Some(weights)) = (reader.read_joints(0), reader.read_weights(0)) else {
        return Err(ImportError::InvalidOptions(
            "a mesh of a model that bends has no bone weights".to_owned(),
        ));
    };
    joints
        .into_u16()
        .zip(weights.into_f32())
        .map(|(joints, weights)| {
            let mut numbers = [0u8; 4];
            for (slot, (&joint, &weight)) in joints.iter().zip(&weights).enumerate() {
                if weight > 0.0 {
                    numbers[slot] = *rig.numbers.get(usize::from(joint)).ok_or_else(|| {
                        ImportError::InvalidOptions(format!(
                            "a vertex is weighted to bone {joint}, which the skin doesn't have"
                        ))
                    })?;
                }
            }
            Ok(PackedSkin::new(numbers, weights))
        })
        .collect()
}

/// Works out the sphere around all that each bone moves, which says how far a pose carries it.
fn measure_influences(skeleton: &mut Skeleton, meshes: &[(Mesh, Option<Vec<PackedSkin>>)]) {
    let mut points: Vec<Vec<Vec3>> = vec![Vec::new(); skeleton.bones.len()];
    for (mesh, skins) in meshes {
        let Some(skins) = skins else {
            continue;
        };
        for (vertex, skin) in mesh.vertices.iter().zip(skins) {
            for bone in skin.bones() {
                points[usize::from(bone)].push(vertex.position);
            }
        }
    }
    for (bone, points) in skeleton.bones.iter_mut().zip(&points) {
        if !points.is_empty() {
            let sphere = Sphere::from_points(points.iter().copied());
            bone.center = sphere.center;
            bone.radius = sphere.radius;
        }
    }
}

/// A normal as a file gives it, or straight up if it gives none: some exporters write zero
/// normals for degenerate triangles, and those cannot be encoded.
fn read_normal(normal: [f32; 3]) -> Vec3 {
    let normal = Vec3::from(normal);
    if normal.length_squared() > 1.0e-12 {
        normal
    } else {
        Vec3::Y
    }
}

pub(crate) fn read_primitive(
    primitive: &Primitive<'_>,
    buffers: &[buffer::Data],
    mirrored: bool,
) -> Mesh {
    let reader = primitive.reader(|b| Some(&buffers[b.index()].0));
    let positions: Vec<[f32; 3]> = reader
        .read_positions()
        .map_or_else(Vec::new, Iterator::collect);
    let normals: Option<Vec<[f32; 3]>> = reader.read_normals().map(Iterator::collect);
    let uvs: Option<Vec<[f32; 2]>> = reader.read_tex_coords(0).map(|uv| uv.into_f32().collect());
    let uv_transform = material::uv_transform(&primitive.material());

    let vertices = positions
        .iter()
        .enumerate()
        .map(|(i, &position)| Vertex {
            position: Vec3::from(position),
            normal: normals.as_ref().map_or(Vec3::Y, |n| read_normal(n[i])),
            uv: uvs
                .as_ref()
                .map_or(Vec2::ZERO, |uv| uv_transform.apply(Vec2::from(uv[i]))),
        })
        .collect();
    let mut indices: Vec<u32> = match reader.read_indices() {
        Some(indices) => indices.into_u32().collect(),
        None => (0..positions.len() as u32).collect(),
    };
    if mirrored {
        for triangle in indices.chunks_exact_mut(3) {
            triangle.swap(1, 2);
        }
    }

    let mut mesh = Mesh { vertices, indices };
    if normals.is_none() {
        mesh.compute_smooth_normals();
    }
    mesh
}

/// A flat grid facing +Y, centred on the origin. Subdivided so that culling can reject parts
/// of it, and so that its simplification has interior edges to work with.
fn ground_plane(extent: f32) -> Mesh {
    const CELLS: u32 = 32;
    let mut mesh = Mesh::default();
    for z in 0..=CELLS {
        for x in 0..=CELLS {
            let (u, v) = (x as f32 / CELLS as f32, z as f32 / CELLS as f32);
            mesh.vertices.push(Vertex {
                position: Vec3::new((u - 0.5) * extent, 0.0, (v - 0.5) * extent),
                normal: Vec3::Y,
                uv: Vec2::new(u, v) * extent,
            });
        }
    }
    let at = |x: u32, z: u32| z * (CELLS + 1) + x;
    for z in 0..CELLS {
        for x in 0..CELLS {
            mesh.indices.extend([
                at(x, z),
                at(x, z + 1),
                at(x + 1, z),
                at(x + 1, z),
                at(x, z + 1),
                at(x + 1, z + 1),
            ]);
        }
    }
    mesh
}

/// The height at which a model meets the ground.
///
/// Scans and other authored assets usually put the ground at Y = 0, with some of the mesh
/// buried below it. Using the lowest vertex instead would leave such a model balanced on its
/// lowest point, visibly hovering. So if zero lies in the lower half of the model, it is taken
/// to be the authored ground. Otherwise the file says nothing useful, and the lowest point is
/// the best guess.
fn ground_height(min_y: f32, max_y: f32) -> f32 {
    if min_y <= 0.0 && 0.0 < (min_y + max_y) * 0.5 {
        0.0
    } else {
        min_y
    }
}

/// An axis-aligned box that starts out empty.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Bounds {
    min: Vec3,
    max: Vec3,
}

impl Default for Bounds {
    fn default() -> Self {
        Self {
            min: Vec3::splat(f32::MAX),
            max: Vec3::splat(f32::MIN),
        }
    }
}

impl Bounds {
    fn add_box(&mut self, min: Vec3, max: Vec3, transform: Mat4) {
        for i in 0..8 {
            let corner = Vec3::new(
                if i & 1 == 0 { min.x } else { max.x },
                if i & 2 == 0 { min.y } else { max.y },
                if i & 4 == 0 { min.z } else { max.z },
            );
            let p = transform.transform_point3(corner);
            self.min = self.min.min(p);
            self.max = self.max.max(p);
        }
    }

    fn transformed(&self, transform: Mat4) -> Self {
        let mut result = Self::default();
        result.add_box(self.min, self.max, transform);
        result
    }

    fn center(&self) -> Vec3 {
        (self.min + self.max) * 0.5
    }

    fn longest_side(&self) -> f32 {
        (self.max - self.min).max_element()
    }
}

fn root_transform(bounds: &Bounds, fit_size: Option<f32>) -> Mat4 {
    let Some(size) = fit_size else {
        return Mat4::IDENTITY;
    };
    let scale = size / bounds.longest_side().max(1.0e-6);
    let center = bounds.center();
    let ground = ground_height(bounds.min.y, bounds.max.y);
    Mat4::from_scale(Vec3::splat(scale))
        * Mat4::from_translation(Vec3::new(-center.x, -ground, -center.z))
}

/// A three-quarter view from which the whole model is visible.
pub(crate) fn frame_bounds(bounds: &Bounds) -> Viewpoint {
    /// Angular radius the model's bounding sphere fills. Models rarely fill their sphere,
    /// so this exceeds half the vertical field of view and they still fit.
    const FRAMING_HALF_ANGLE: f32 = 0.62;
    let center = bounds.center();
    let radius = ((bounds.max - bounds.min).length() * 0.5).max(0.01);
    let offset = Vec3::new(0.62, 0.32, 1.0).normalize() * (radius / FRAMING_HALF_ANGLE.sin());
    let position = center + offset;
    let forward = -offset.normalize();
    Viewpoint {
        position: position.to_array(),
        yaw: (-forward.x).atan2(-forward.z),
        pitch: forward.y.asin(),
    }
}

#[cfg(test)]
mod tests;
