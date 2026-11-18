//! Bones on the GPU: each pose's skinning matrices, this frame's and last frame's, and each
//! skinned cluster's bone list.
//!
//! A bone's skinning matrix takes a vertex from the mesh's bind pose to where the bone has
//! carried it. Vertices are skinned where they are used, so nothing else depends on the pose.
//!
//! WebGPU allows eight storage buffers per shader stage, and culling and rasterizing use all of
//! them. So the matrices and the lists live in textures, read texel by texel, which need no new
//! storage binding.

use crate::gpu::Gpu;
use crate::motion::InstanceTable;
use glam::{Mat3, Mat4, Vec3, Vec4};
use solstice_asset::paged::PagedScene;
use solstice_asset::{Skeleton, NO_POSE};
use std::ops::Range;

/// `Instance.bones` of an instance that isn't skinned. Twin of `NO_BONES` in `common/skin.wgsl`.
pub const NO_BONES: u32 = u32::MAX;
/// Bone texels per row. Twin of `BONE_TEXTURE_WIDTH` in `common/skin.wgsl`, and of the row used
/// by the list texture.
pub const BONE_TEXTURE_WIDTH: u32 = 1024;
/// A skinning matrix takes three texels: its top three rows.
pub const TEXELS_PER_BONE: u32 = 3;

/// Where each pose's first bone goes in the bone texture, as a texel number, and the texels the
/// poses take in all.
pub fn pose_addresses(scene: &PagedScene) -> (Vec<u32>, u32) {
    let mut next = 0;
    let addresses = scene
        .poses
        .iter()
        .map(|&skeleton| {
            let here = next;
            next += scene.skeletons[skeleton as usize].bones.len() as u32 * TEXELS_PER_BONE;
            here
        })
        .collect();
    (addresses, next)
}

/// One pose's place in the bone texture, and what draws with it.
struct Pose {
    skeleton: u32,
    address: u32,
    /// Those of the GPU's instances that follow this pose.
    instances: Vec<u32>,
    /// Whether it has been given a pose yet. Until it has, it shows the bind pose, and the first
    /// real pose must not look like a sudden movement.
    started: bool,
}

/// One pose, waiting for the next frame.
struct PendingPose {
    pose: u32,
    bones: Vec<Mat4>,
    jump: bool,
}

/// Bones for a scene's poses.
pub struct GpuSkins {
    texture: wgpu::Texture,
    pub bones_view: wgpu::TextureView,
    pub lists_view: wgpu::TextureView,
    /// Texture rows of one frame's matrices. This frame's are in the top half, last frame's in
    /// the bottom.
    rows: u32,
    current: Vec<Vec4>,
    previous: Vec<Vec4>,
    poses: Vec<Pose>,
    skeletons: Vec<Skeleton>,
    pending: Vec<PendingPose>,
    moved_last_frame: Vec<u32>,
}

impl GpuSkins {
    pub fn new(gpu: &Gpu, scene: &PagedScene, gpu_index: &[Option<u32>]) -> Self {
        let (addresses, texels) = pose_addresses(scene);
        let rows = texels.div_ceil(BONE_TEXTURE_WIDTH).max(1);
        let identity = Mat4::IDENTITY;
        let per_frame = (BONE_TEXTURE_WIDTH * rows) as usize;
        let mut current = vec![Vec4::ZERO; per_frame];
        for chunk in current.chunks_exact_mut(TEXELS_PER_BONE as usize) {
            chunk.copy_from_slice(&[identity.row(0), identity.row(1), identity.row(2)]);
        }

        let mut poses: Vec<Pose> = scene
            .poses
            .iter()
            .zip(&addresses)
            .map(|(&skeleton, &address)| Pose {
                skeleton,
                address,
                instances: Vec::new(),
                started: false,
            })
            .collect();
        for (number, (&pose, slot)) in scene.instance_poses.iter().zip(gpu_index).enumerate() {
            let mesh = scene.instances[number].mesh as usize;
            let bends = scene.meshes[mesh].bone_lists.is_some();
            if let (true, true, Some(i)) = (bends, pose != NO_POSE, slot) {
                poses[pose as usize].instances.push(*i);
            }
        }

        assert!(
            rows * 2 <= gpu.device.limits().max_texture_dimension_2d,
            "this scene has too many posed bones for one texture"
        );
        let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("bones"),
            size: wgpu::Extent3d {
                width: BONE_TEXTURE_WIDTH,
                height: rows * 2,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba32Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let lists = cluster_list_texture(gpu, scene);
        let this = Self {
            bones_view: texture.create_view(&Default::default()),
            lists_view: lists.create_view(&Default::default()),
            texture,
            rows,
            previous: current.clone(),
            current,
            poses,
            skeletons: scene.skeletons.clone(),
            pending: Vec::new(),
            moved_last_frame: Vec::new(),
        };
        this.upload(gpu, 0..rows);
        this
    }

    pub fn pose_count(&self) -> u32 {
        self.poses.len() as u32
    }

    /// The skeleton of a pose.
    pub fn skeleton(&self, pose: u32) -> Option<&Skeleton> {
        let pose = self.poses.get(pose as usize)?;
        self.skeletons.get(pose.skeleton as usize)
    }

    /// Queues a pose, each bone's place in the mesh's own space, for the next frame. Returns false
    /// if there is no such pose or `bones` doesn't match its skeleton.
    pub fn set(&mut self, pose: u32, bones: &[Mat4], jump: bool) -> bool {
        let Some(skeleton) = self.skeleton(pose) else {
            return false;
        };
        if skeleton.bones.len() != bones.len() {
            return false;
        }
        self.pending.push(PendingPose {
            pose,
            bones: bones.to_vec(),
            jump,
        });
        true
    }

    /// Applies this frame's poses, lets last frame's catch up, and uploads what changed. Sets each
    /// changed pose's instances' LOD margins.
    pub fn update(&mut self, gpu: &Gpu, instances: &mut InstanceTable) {
        let mut touched = Vec::new();
        for pose in std::mem::take(&mut self.moved_last_frame) {
            self.settle(pose);
            touched.push(self.rows_of(pose));
        }
        // A stable sort keeps each pose's entries in the order given, so the last of each run is
        // the one that counts.
        let mut pending = std::mem::take(&mut self.pending);
        pending.sort_by_key(|p| p.pose);
        for (index, entry) in pending.iter().enumerate() {
            if pending.get(index + 1).is_some_and(|p| p.pose == entry.pose) {
                continue;
            }
            self.settle(entry.pose);
            let margin = self.apply(entry.pose, &entry.bones);
            let pose = &mut self.poses[entry.pose as usize];
            let first = !std::mem::replace(&mut pose.started, true);
            for &i in &pose.instances {
                instances.set_lod_margin(i, margin);
            }
            if entry.jump || first {
                self.settle(entry.pose);
            } else {
                self.moved_last_frame.push(entry.pose);
            }
            touched.push(self.rows_of(entry.pose));
        }
        for rows in merged_rows(touched) {
            self.upload(gpu, rows);
        }
    }

    /// The rows of one frame's matrices a pose's bones lie on.
    fn rows_of(&self, pose: u32) -> Range<u32> {
        rows_of_texels(self.range(pose))
    }

    /// Makes last frame's matrices for a pose the same as this frame's, so that it shows no motion.
    fn settle(&mut self, pose: u32) {
        let range = self.range(pose);
        self.previous[range.clone()].copy_from_slice(&self.current[range]);
    }

    fn range(&self, pose: u32) -> std::ops::Range<usize> {
        let slot = &self.poses[pose as usize];
        let start = slot.address as usize;
        let bones = self.skeletons[slot.skeleton as usize].bones.len();
        start..start + bones * TEXELS_PER_BONE as usize
    }

    /// Writes a pose's matrices, and returns the farthest the pose carries anything, in the mesh's
    /// units.
    fn apply(&mut self, pose: u32, bones: &[Mat4]) -> f32 {
        let start = self.range(pose).start;
        let skeleton = &self.skeletons[self.poses[pose as usize].skeleton as usize];
        let mut margin = 0.0f32;
        for (b, (bone, placed)) in skeleton.bones.iter().zip(bones).enumerate() {
            let skinning = *placed * bone.inverse_bind;
            let at = start + b * TEXELS_PER_BONE as usize;
            self.current[at] = skinning.row(0);
            self.current[at + 1] = skinning.row(1);
            self.current[at + 2] = skinning.row(2);
            margin = margin.max(bone_reach(skinning, bone.center, bone.radius));
        }
        margin
    }

    /// Uploads some rows of this frame's matrices and of last frame's.
    fn upload(&self, gpu: &Gpu, rows: std::ops::Range<u32>) {
        if rows.is_empty() {
            return;
        }
        let height = rows.end - rows.start;
        let texels =
            (rows.start * BONE_TEXTURE_WIDTH) as usize..(rows.end * BONE_TEXTURE_WIDTH) as usize;
        for (data, top) in [(&self.current, 0), (&self.previous, self.rows)] {
            gpu.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &self.texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d {
                        x: 0,
                        y: top + rows.start,
                        z: 0,
                    },
                    aspect: wgpu::TextureAspect::All,
                },
                bytemuck::cast_slice(&data[texels.clone()]),
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(BONE_TEXTURE_WIDTH * 16),
                    rows_per_image: Some(height),
                },
                wgpu::Extent3d {
                    width: BONE_TEXTURE_WIDTH,
                    height,
                    depth_or_array_layers: 1,
                },
            );
        }
    }
}

/// The rows of the bone texture a run of texels lies on.
fn rows_of_texels(texels: Range<usize>) -> Range<u32> {
    texels.start as u32 / BONE_TEXTURE_WIDTH..(texels.end as u32).div_ceil(BONE_TEXTURE_WIDTH)
}

/// Sorts runs of rows and joins those that touch, dropping empty ones, to upload each once.
fn merged_rows(mut rows: Vec<Range<u32>>) -> Vec<Range<u32>> {
    rows.retain(|r| !r.is_empty());
    rows.sort_by_key(|r| r.start);
    let mut merged: Vec<Range<u32>> = Vec::with_capacity(rows.len());
    for run in rows {
        match merged.last_mut() {
            Some(last) if run.start <= last.end => last.end = last.end.max(run.end),
            _ => merged.push(run),
        }
    }
    merged
}

/// How far a skinning matrix carries anything inside the sphere at `center`: how far the centre
/// moves, plus how far the turn moves a point at the sphere's edge. A turn by an angle moves a
/// point at radius r by `2 r sin(angle / 2)`, and `2 sin(angle / 2)` is `sqrt(3 - trace)`.
fn bone_reach(skinning: Mat4, center: Vec3, radius: f32) -> f32 {
    let moved = skinning.transform_point3(center) - center;
    let turn = Mat3::from_mat4(skinning);
    let trace = turn.x_axis.x + turn.y_axis.y + turn.z_axis.z;
    moved.length() + (3.0 - trace).max(0.0).sqrt() * radius
}

/// The texture holding each skinned cluster's bone list: one texel a cluster, its sixteen bytes
/// in order, so that cull.wgsl can read it as sixteen bones in four words.
fn cluster_list_texture(gpu: &Gpu, scene: &PagedScene) -> wgpu::Texture {
    let count = scene.cluster_bones.len() as u32;
    let rows = count.div_ceil(BONE_TEXTURE_WIDTH).max(1);
    assert!(
        rows <= gpu.device.limits().max_texture_dimension_2d,
        "this scene has too many skinned clusters for one texture"
    );
    let mut texels = scene.cluster_bones.clone();
    texels.resize((BONE_TEXTURE_WIDTH * rows) as usize, [0; 16]);
    let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("cluster bone lists"),
        size: wgpu::Extent3d {
            width: BONE_TEXTURE_WIDTH,
            height: rows,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba32Uint,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    gpu.queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        bytemuck::cast_slice(&texels),
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(BONE_TEXTURE_WIDTH * 16),
            rows_per_image: Some(rows),
        },
        wgpu::Extent3d {
            width: BONE_TEXTURE_WIDTH,
            height: rows,
            depth_or_array_layers: 1,
        },
    );
    texture
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Quat;

    #[test]
    fn a_run_of_texels_lies_on_the_rows_it_touches() {
        let width = BONE_TEXTURE_WIDTH as usize;
        assert_eq!(rows_of_texels(0..3), 0..1);
        assert_eq!(rows_of_texels(0..width), 0..1);
        assert_eq!(rows_of_texels(width - 1..width + 2), 0..2);
        assert_eq!(rows_of_texels(width..width + 3), 1..2);
        assert_eq!(rows_of_texels(2 * width..3 * width), 2..3);
        // An empty skeleton's run comes to nothing to upload.
        assert_eq!(merged_rows(vec![rows_of_texels(width..width)]), vec![]);
    }

    #[test]
    fn runs_of_rows_are_joined_only_where_they_touch() {
        assert_eq!(merged_rows(vec![5..6, 0..1, 1..2, 0..2]), vec![0..2, 5..6]);
        assert_eq!(merged_rows(vec![0..4, 1..2]), vec![0..4]);
        assert_eq!(merged_rows(vec![2..2, 3..3]), vec![]);
    }

    #[test]
    fn a_still_bone_reaches_nothing() {
        assert_eq!(bone_reach(Mat4::IDENTITY, Vec3::ONE, 2.0), 0.0);
    }

    #[test]
    fn a_bones_reach_counts_its_slide_and_its_turn() {
        let slide = Mat4::from_translation(Vec3::new(0.0, 3.0, 0.0));
        assert!((bone_reach(slide, Vec3::ZERO, 1.0) - 3.0).abs() < 1.0e-5);
        // A quarter turn about the centre moves a point on the sphere by sqrt(2) r.
        let turn = Mat4::from_quat(Quat::from_rotation_y(std::f32::consts::FRAC_PI_2));
        let reach = bone_reach(turn, Vec3::ZERO, 2.0);
        assert!(
            (reach - 2.0 * std::f32::consts::SQRT_2).abs() < 1.0e-4,
            "{reach}"
        );
    }

    #[test]
    fn poses_are_given_their_own_runs_of_texels() {
        let skeleton = |bones: usize| Skeleton {
            bones: vec![Default::default(); bones],
        };
        let scene = PagedScene {
            skeletons: vec![skeleton(5), skeleton(2)],
            poses: vec![1, 0, 1],
            ..Default::default()
        };
        let (addresses, texels) = pose_addresses(&scene);
        assert_eq!(addresses, [0, 6, 21]);
        assert_eq!(texels, 27);
    }
}
