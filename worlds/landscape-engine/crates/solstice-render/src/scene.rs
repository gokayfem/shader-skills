//! Creates a scene's GPU resources.
//!
//! WebGPU has no bindless resources. Everything geometric lives in a few big buffers that all
//! passes share. Hierarchy nodes and cluster records stay resident, while vertices and triangles
//! live in a fixed pool of page slots filled by [`crate::streaming`].

use crate::gpu::Gpu;
use crate::motion::{FrameChanges, InstanceTable};
use crate::regions::RegionPlan;
use crate::skin::{pose_addresses, GpuSkins, NO_BONES};
use crate::streaming::textures::TierPlan;
use bytemuck::{Pod, Zeroable};
use glam::{Mat3, Mat4, Vec2, Vec3, Vec4};
use solstice_asset::paged::{PagedScene, BASE_TEXTURE_EDGE, PAGE_BYTES};
use solstice_asset::{Material, TextureEncoding, TextureLayer, NO_POSE};

/// Twin of `Instance` in `common/scene.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct GpuInstance {
    world: [Vec4; 3],
    prev_world: [Vec4; 3],
    normal: [Vec4; 3],
    quant_origin: Vec3,
    quant_step: f32,
    uv_origin: Vec2,
    uv_step: Vec2,
    root_node: u32,
    /// Material index, with the `INSTANCE_` flags in its top bits.
    material: u32,
    max_scale: f32,
    texture_info: u32,
    /// How brightly the material's own light shines here: 1 as it is, 0 dark.
    glow: f32,
    /// Where this instance's bones start in the bone texture, or [`NO_BONES`].
    bones: u32,
    /// What to add to a cluster's number to find its bone list.
    bone_lists: i32,
    /// The farthest its pose carries anything, in the mesh's units.
    lod_margin: f32,
}

const _: () = assert!(std::mem::size_of::<GpuInstance>() == 208);

// Flags in `GpuInstance::material`, twins of the same constants in `common/scene.wgsl`.
/// Drawn from both sides.
pub(crate) const INSTANCE_TWO_SIDED: u32 = 1 << 31;
/// May move: drawn afresh every frame, and left out of the views that are kept from frame to
/// frame (shadow cascades, the seabed map, GI voxels), which would otherwise keep it where it
/// was.
pub(crate) const INSTANCE_MOVABLE: u32 = 1 << 30;
/// Not drawn at all.
pub(crate) const INSTANCE_HIDDEN: u32 = 1 << 29;
/// Movable, but kept in the GI voxels where it stood when they were last drawn: for things
/// that move only a little, such as a ship at anchor, whose light and shade would be missed.
pub(crate) const INSTANCE_STAYS_IN_GI: u32 = 1 << 28;
/// Cut out: goes through the masked lists and rasterizers (`docs/cutouts.md`).
pub(crate) const INSTANCE_MASKED: u32 = 1 << 27;
/// Sways in the wind (`common/sway.wgsl`), from its material's [`Material::SWAYS`].
pub(crate) const INSTANCE_SWAYS: u32 = 1 << 26;
/// Sways as a tree, from its material's [`Material::TREE`].
pub(crate) const INSTANCE_TREE: u32 = 1 << 25;
/// A tree's leaves, which flutter: [`Material::TREE`] and [`Material::FOLIAGE`].
pub(crate) const INSTANCE_FLUTTERS: u32 = 1 << 24;
/// Ground of three kinds blended, from its material's [`Material::BLENDS`]: the material and
/// the two after it, whose texture layers follow its own (`apps/bake/src/land.rs`).
pub(crate) const INSTANCE_BLENDS: u32 = 1 << 23;

impl GpuInstance {
    /// Places the instance at `world`. Where it was drawn last stays as `prev_world`.
    pub(crate) fn set_world(&mut self, world: Mat4) {
        let linear = Mat3::from_mat4(world);
        self.world = affine_rows(world);
        self.normal = affine_rows(Mat4::from_mat3(linear.inverse().transpose()));
        self.max_scale = max_scale(linear);
    }

    /// Makes this frame's place last frame's too, so the instance shows no motion.
    pub(crate) fn settle(&mut self) {
        self.prev_world = self.world;
    }

    pub(crate) fn has_flag(&self, flag: u32) -> bool {
        self.material & flag != 0
    }

    pub(crate) fn set_flag(&mut self, flag: u32, on: bool) {
        if on {
            self.material |= flag;
        } else {
            self.material &= !flag;
        }
    }

    pub(crate) fn glow(&self) -> f32 {
        self.glow
    }

    /// Sets how far its pose carries anything, in the mesh's own units.
    pub(crate) fn set_lod_margin(&mut self, margin: f32) {
        self.lod_margin = margin;
    }

    /// Whether it bends to a pose.
    pub(crate) fn is_skinned(&self) -> bool {
        self.bones != NO_BONES
    }

    #[cfg(test)]
    pub(crate) fn set_bones(&mut self, bones: u32) {
        self.bones = bones;
    }

    pub(crate) fn lod_margin(&self) -> f32 {
        self.lod_margin
    }

    pub(crate) fn set_glow(&mut self, glow: f32) {
        self.glow = glow;
    }

    pub(crate) fn max_scale(&self) -> f32 {
        self.max_scale
    }

    /// The transform the instance is drawn with this frame, and the one it was drawn with last.
    pub(crate) fn worlds(&self) -> (Mat4, Mat4) {
        (
            from_affine_rows(self.world),
            from_affine_rows(self.prev_world),
        )
    }
}

/// The largest scale along any axis of `linear`.
fn max_scale(linear: Mat3) -> f32 {
    linear
        .x_axis
        .length()
        .max(linear.y_axis.length())
        .max(linear.z_axis.length())
}

/// Stores the top three rows of an affine matrix.
fn affine_rows(m: Mat4) -> [Vec4; 3] {
    [m.row(0), m.row(1), m.row(2)]
}

/// Inverse of [`affine_rows`].
fn from_affine_rows(rows: [Vec4; 3]) -> Mat4 {
    Mat4::from_cols(rows[0], rows[1], rows[2], Vec4::W).transpose()
}

/// A scene on the GPU. Everything but the page pool is fully resident.
pub struct GpuScene {
    pub instances: wgpu::Buffer,
    pub nodes: wgpu::Buffer,
    pub clusters: wgpu::Buffer,
    /// The page pool, seen as vertices. See [`Self::triangles`].
    pub vertices: wgpu::Buffer,
    /// The page pool again, seen as triangles. Each slot holds a page's vertices and then its
    /// triangles, and cluster records hold offsets into the two views. In a scene too large for
    /// one binding (see [`RegionPlan`]) this is instead the middle band's own buffer.
    pub triangles: wgpu::Buffer,
    /// For a scene too large for one binding: how it is divided.
    pub regions: Option<RegionPlan>,
    /// The cluster records past the first binding's, if the scene has regions.
    pub clusters_more: Option<wgpu::Buffer>,
    /// The far band's page pool, if the scene has regions.
    pub pool_far: Option<wgpu::Buffer>,
    /// The bands for what the other three can't hold, if the scene has regions and uses any.
    pub pool_spares: Vec<wgpu::Buffer>,
    pub materials: wgpu::Buffer,
    pub lights: wgpu::Buffer,
    pub textures: TextureArrays,
    pub texture_summary: String,
    pub material_sampler: wgpu::Sampler,
    /// The emissive maps, for a scene that has any and a GPU that can read them.
    pub glow_maps: Option<wgpu::TextureView>,
    /// The scene's transparent glowing cards, if it has any.
    pub cards: Option<crate::cards::Cards>,
    /// Instances on the GPU: those of the scene's instances that have geometry.
    pub gpu_instance_count: u32,
    /// Whether any instance is cut out, so its lists and pipelines are needed.
    pub has_masked: bool,
    /// Whether anything bends to a pose. A scene with nothing that does has skinning compiled out
    /// of its shaders.
    pub has_skins: bool,
    /// Whether anything sways in the wind. A scene with nothing that does has swaying compiled
    /// out of its shaders, and never fills the wind field for it.
    pub has_sways: bool,
    /// The wind near the ground, around the camera, for what sways (`wind.rs`).
    pub wind_field: crate::gpu::Target,
    /// The CPU's copy of `instances`, and what has changed in it.
    pub instance_table: InstanceTable,
    /// Bones for the scene's poses, and each skinned cluster's bone list.
    pub skins: GpuSkins,
    pub light_count: u32,
    /// Longest root-to-leaf path over all meshes, which sets the traversal dispatch count.
    pub max_hierarchy_depth: u32,
    /// Triangles drawn if every instance used its finest LOD.
    pub source_triangles: u64,
    pub unique_triangles: u64,
}

impl GpuScene {
    /// What moves this scene's vertices, which the shaders that place them are compiled for.
    pub fn deform(&self) -> crate::shaders::Deform {
        crate::shaders::Deform {
            skinned: self.has_skins,
            sways: self.has_sways,
        }
    }

    /// Creates everything but the contents of the page pool, which holds `slots` pages.
    /// Texture levels are written straight away if `upper_textures` holds them all. Otherwise
    /// tiers follow `plan`, with only the base tier filled, and streaming does the rest.
    pub fn new(
        gpu: &Gpu,
        scene: &PagedScene,
        slots: u32,
        upper_textures: Option<&[TextureLayer]>,
        plan: Option<TierPlan>,
    ) -> Self {
        let (instances, gpu_index) = gpu_instances(scene);
        let skins = GpuSkins::new(gpu, scene, &gpu_index);
        let materials = starting_materials(scene);
        let (textures, texture_summary) = upload_textures(gpu, scene, upper_textures, plan);

        let material_sampler = gpu.device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("material"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            anisotropy_clamp: 8,
            ..Default::default()
        });

        let regions = RegionPlan::decide(&gpu.limits, scene, slots);
        let pool_of = |label: &str, slots: u32| {
            gpu.storage_buffer(
                label,
                u64::from(slots.max(1)) * PAGE_BYTES as u64,
                wgpu::BufferUsages::COPY_DST,
            )
        };
        let (pool, middle, far, spares) = match regions {
            Some(plan) => (
                pool_of("page pool, near", plan.band_slots[0]),
                pool_of("page pool, middle", plan.band_slots[1]),
                Some(pool_of("page pool, far", plan.band_slots[2])),
                (3..plan.pools())
                    .map(|band| pool_of("page pool, spare", plan.band_slots[band]))
                    .collect(),
            ),
            None => {
                let pool = pool_of("page pool", slots);
                (pool.clone(), pool, None, Vec::new())
            }
        };
        assert!(
            scene.external.is_none() || scene.skeletons.is_empty(),
            "a scene that keeps its cluster records outside its manifest can't be skinned"
        );
        let empty = |label: &str, records: u64| {
            // Never empty: a binding must hold at least one record.
            let bytes =
                records.max(1) * std::mem::size_of::<solstice_asset::PackedCluster>() as u64;
            gpu.storage_buffer(label, bytes, wgpu::BufferUsages::COPY_DST)
        };
        // Such a scene has records only for the pages in its pool, a block for each slot.
        let resident = scene.external.map(|_| {
            let total = regions.map_or(slots, |plan| plan.band_slots.iter().sum());
            u64::from(total) * u64::from(scene.max_page_clusters().max(1))
        });
        let (clusters, clusters_more) = match (regions, resident) {
            (Some(plan), Some(total)) => {
                let first = total.min(u64::from(plan.cluster_chunk));
                (
                    empty("clusters", first),
                    Some(empty("clusters, more", total - first)),
                )
            }
            (None, Some(total)) => (empty("clusters", total), None),
            (Some(plan), None) => {
                let (first, rest) = scene.clusters.split_at(plan.cluster_chunk as usize);
                (
                    gpu.storage_buffer_init("clusters", first),
                    Some(gpu.storage_buffer_init("clusters, more", rest)),
                )
            }
            (None, None) => (gpu.storage_buffer_init("clusters", &scene.clusters), None),
        };
        Self {
            instances: gpu.storage_buffer_init("instances", &instances),
            nodes: gpu.storage_buffer_init("nodes", &scene.nodes),
            clusters,
            vertices: pool,
            triangles: middle,
            regions,
            clusters_more,
            pool_far: far,
            pool_spares: spares,
            materials: gpu.storage_buffer_init("materials", &materials),
            lights: gpu.storage_buffer_init("lights", &scene.lights),
            textures,
            texture_summary,
            material_sampler,
            glow_maps: upload_glow_maps(gpu, scene),
            cards: crate::cards::Cards::new(gpu, scene),
            gpu_instance_count: instances.len() as u32,
            has_masked: instances.iter().any(|i| i.has_flag(INSTANCE_MASKED)),
            has_skins: instances.iter().any(GpuInstance::is_skinned),
            has_sways: instances.iter().any(|i| i.has_flag(INSTANCE_SWAYS)),
            wind_field: crate::wind::field_texture(
                gpu,
                instances.iter().any(|i| i.has_flag(INSTANCE_SWAYS)),
            ),
            instance_table: InstanceTable::new(instances, gpu_index, instance_spheres(scene)),
            skins,
            light_count: scene.lights.len() as u32,
            max_hierarchy_depth: scene.hierarchy_depth,
            source_triangles: scene.source_triangles,
            unique_triangles: scene.unique_triangles,
        }
    }

    /// The buffer holding a band of the page pool.
    pub fn pool(&self, band: usize) -> &wgpu::Buffer {
        match band {
            0 => &self.vertices,
            1 => &self.triangles,
            2 => self
                .pool_far
                .as_ref()
                .expect("only scenes with regions have a far band"),
            spare => &self.pool_spares[spare - 3],
        }
    }

    /// Bytes of GPU memory in cluster records and page pools.
    pub fn geometry_bytes(&self) -> (u64, u64) {
        let more = self.clusters_more.as_ref().map_or(0, wgpu::Buffer::size);
        let pools = match &self.pool_far {
            Some(far) => {
                let spare: u64 = self.pool_spares.iter().map(wgpu::Buffer::size).sum();
                self.vertices.size() + self.triangles.size() + far.size() + spare
            }
            None => self.vertices.size(),
        };
        (self.clusters.size() + more, pools)
    }

    /// Applies this frame's changes to instances and uploads them. Returns what changed, so that
    /// the cached views holding it can be drawn again.
    pub fn update_instances(&mut self, gpu: &Gpu) -> FrameChanges {
        self.skins.update(gpu, &mut self.instance_table);
        let changes = self.instance_table.advance();
        for range in &changes.ranges {
            let offset = (range.start * std::mem::size_of::<GpuInstance>()) as u64;
            gpu.queue.write_buffer(
                &self.instances,
                offset,
                bytemuck::cast_slice(self.instance_table.gpu_data(range.clone())),
            );
        }
        changes
    }
}

/// The emissive maps, as one array. `None` for a scene without any or a GPU that can't read
/// them, and such a scene binds nothing for them: its materials glow evenly, by their factors.
fn upload_glow_maps(gpu: &Gpu, scene: &PagedScene) -> Option<wgpu::TextureView> {
    let block = gpu
        .device
        .features()
        .contains(wgpu::Features::TEXTURE_COMPRESSION_BC);
    if scene.glow.layers.is_empty() {
        return None;
    }
    if !block || scene.glow.layers.len() as u32 > gpu.limits.max_texture_array_layers {
        log::warn!("this GPU can't hold the scene's emissive maps, so its materials glow evenly");
        return None;
    }
    let (edge, layers) = (scene.glow.edge, &scene.glow.layers);
    let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("glow maps"),
        size: wgpu::Extent3d {
            width: edge,
            height: edge,
            depth_or_array_layers: layers.len() as u32,
        },
        mip_level_count: edge.ilog2() + 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Bc7RgbaUnormSrgb,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    for (layer, mips) in layers.iter().enumerate() {
        for (level, data) in mips.iter().enumerate() {
            let blocks = (edge >> level).max(1).div_ceil(4);
            gpu.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &texture,
                    mip_level: level as u32,
                    origin: wgpu::Origin3d {
                        x: 0,
                        y: 0,
                        z: layer as u32,
                    },
                    aspect: wgpu::TextureAspect::All,
                },
                data,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(blocks * 16),
                    rows_per_image: Some(blocks),
                },
                wgpu::Extent3d {
                    width: blocks * 4,
                    height: blocks * 4,
                    depth_or_array_layers: 1,
                },
            );
        }
    }
    Some(texture.create_view(&wgpu::TextureViewDescriptor {
        dimension: Some(wgpu::TextureViewDimension::D2Array),
        ..Default::default()
    }))
}

/// The scene's materials, as they stand with every texture in the base tier.
pub fn starting_materials(scene: &PagedScene) -> Vec<Material> {
    let mut materials = scene.materials.clone();
    if materials.is_empty() {
        materials.push(Material::default());
    }
    for material in &mut materials {
        material.texture_tier = 0;
        material.texture_slot = material.texture_layer.max(0) as u32;
    }
    materials
}

/// `Instance.texture_info` in `common/scene.wgsl`: what culling needs to work out how sharp a
/// texture must be, without having to read materials.
fn texture_info(material: Option<&Material>) -> u32 {
    match material {
        Some(material) if material.texture_layer >= 0 && material.uv_scale > 0.0 => {
            let log2_scale = ((material.uv_scale.log2() + 128.0) * 256.0).clamp(0.0, 65535.0);
            (material.texture_layer as u32) << 16 | log2_scale as u32
        }
        _ => u32::MAX,
    }
}

/// Builds the GPU's view of each instance, and where each of the scene's instances went in it.
/// Instances of meshes with no geometry are left out.
fn gpu_instances(scene: &PagedScene) -> (Vec<GpuInstance>, Vec<Option<u32>>) {
    let mut out = Vec::with_capacity(scene.instances.len());
    let mut index = Vec::with_capacity(scene.instances.len());
    let (addresses, _) = pose_addresses(scene);
    for (number, instance) in scene.instances.iter().enumerate() {
        let mesh = &scene.meshes[instance.mesh as usize];
        let Some(root_node) = mesh.root_node else {
            index.push(None);
            continue;
        };
        index.push(Some(out.len() as u32));
        // The bits above hold the INSTANCE_ flags.
        let source = scene.materials.get(instance.material as usize);
        debug_assert!(instance.material < INSTANCE_BLENDS, "too many materials");
        let world = instance.transform;
        let linear = Mat3::from_mat4(world);
        let normal = Mat4::from_mat3(linear.inverse().transpose());
        out.push(GpuInstance {
            world: affine_rows(world),
            prev_world: affine_rows(world),
            normal: affine_rows(normal),
            quant_origin: mesh.quant_origin,
            quant_step: mesh.quant_step,
            uv_origin: Vec2::from_array(mesh.uv_origin),
            uv_step: Vec2::from_array(mesh.uv_step),
            root_node,
            material: instance.material
                | if source.is_some_and(|m| m.flags & Material::TWO_SIDED != 0) {
                    INSTANCE_TWO_SIDED
                } else {
                    0
                }
                // Glass is drawn where a cut-out is: by the rasterizers that leave pixels out.
                | if source
                    .is_some_and(|m| m.flags & (Material::MASKED | Material::GLASS) != 0)
                {
                    INSTANCE_MASKED
                } else {
                    0
                }
                | if source.is_some_and(|m| m.flags & Material::SWAYS != 0) {
                    INSTANCE_SWAYS
                } else {
                    0
                }
                | if source.is_some_and(|m| m.flags & Material::BLENDS != 0) {
                    INSTANCE_BLENDS
                } else {
                    0
                }
                | match source {
                    Some(m)
                        if m.flags & (Material::SWAYS | Material::TREE)
                            == Material::SWAYS | Material::TREE =>
                    {
                        INSTANCE_TREE
                            | if m.flags & Material::FOLIAGE != 0 {
                                INSTANCE_FLUTTERS
                            } else {
                                0
                            }
                    }
                    _ => 0,
                },
            max_scale: max_scale(linear),
            texture_info: texture_info(scene.materials.get(instance.material as usize)),
            glow: 1.0,
            bones: NO_BONES,
            bone_lists: 0,
            lod_margin: 0.0,
        });
        // Bends if it follows a pose and its mesh was built to. It is then always drawn afresh.
        let pose = scene.instance_poses.get(number).copied().unwrap_or(NO_POSE);
        if let (Some(lists), true) = (mesh.bone_lists, pose != NO_POSE) {
            let gpu = out.last_mut().expect("just pushed");
            gpu.bones = addresses[pose as usize];
            gpu.bone_lists = lists;
            gpu.set_flag(INSTANCE_MOVABLE, true);
        }
    }
    (out, index)
}

/// The object-space bounding sphere of each GPU instance's mesh, in the order
/// [`gpu_instances`] makes them.
fn instance_spheres(scene: &PagedScene) -> Vec<(Vec3, f32)> {
    scene
        .instances
        .iter()
        .filter_map(|instance| scene.meshes[instance.mesh as usize].root_node)
        .map(|root| {
            let node = &scene.nodes[root as usize];
            (Vec3::from_array(node.center), node.radius)
        })
        .collect()
}

/// The largest edge kept when textures must be unpacked, because the GPU can't sample BC.
/// Unpacked textures take 8 bytes per texel rather than 3, so this keeps a scene baked at a
/// larger size from taking far more memory on the devices least able to spare it.
pub const UNPACKED_MAX_EDGE: u32 = 512;

/// One tier: an array per map. A tier a scene doesn't use is a 1x1 stand-in, as WebGPU has no
/// optional bindings.
pub struct TierArrays {
    pub color: wgpu::Texture,
    pub normal: wgpu::Texture,
    pub material: wgpu::Texture,
    pub color_view: wgpu::TextureView,
    pub normal_view: wgpu::TextureView,
    pub material_view: wgpu::TextureView,
    /// Edge of the largest level.
    pub edge: u32,
}

/// Every texture array of a scene. A scene loaded whole has all of it in the base tier.
pub struct TextureArrays {
    pub base: TierArrays,
    pub middle: TierArrays,
    pub top: TierArrays,
    /// Whether the arrays hold BC blocks rather than plain texels.
    pub block: bool,
    /// Whether arriving levels hold BC blocks that must be unpacked, for lack of GPU support.
    pub unpack: bool,
    /// Bytes of GPU memory across all tiers.
    pub bytes: u64,
}

impl TierArrays {
    fn new(gpu: &Gpu, edge: u32, layers: u32, block: bool, name: &str) -> Self {
        let (color_format, data_format) = if block {
            (
                wgpu::TextureFormat::Bc7RgbaUnormSrgb,
                wgpu::TextureFormat::Bc5RgUnorm,
            )
        } else {
            (
                wgpu::TextureFormat::Rgba8UnormSrgb,
                wgpu::TextureFormat::Rg8Unorm,
            )
        };
        let create = |map: &str, format: wgpu::TextureFormat| {
            gpu.device.create_texture(&wgpu::TextureDescriptor {
                label: Some(&format!("{name} {map}")),
                size: wgpu::Extent3d {
                    width: edge,
                    height: edge,
                    depth_or_array_layers: layers,
                },
                mip_level_count: edge.ilog2() + 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_DST
                    | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            })
        };
        let view = |texture: &wgpu::Texture| {
            texture.create_view(&wgpu::TextureViewDescriptor {
                dimension: Some(wgpu::TextureViewDimension::D2Array),
                ..Default::default()
            })
        };
        let color = create("color", color_format);
        let normal = create("normal", data_format);
        let material = create("data", data_format);
        Self {
            color_view: view(&color),
            normal_view: view(&normal),
            material_view: view(&material),
            color,
            normal,
            material,
            edge,
        }
    }

    /// Writes all three maps of one level, given data in the arrays' own form.
    pub fn write_level(
        &self,
        gpu: &Gpu,
        block: bool,
        slot: u32,
        level: u32,
        [color, normal, material]: [&[u8]; 3],
    ) {
        write_level(gpu, &self.color, block, slot, level, color);
        write_level(gpu, &self.normal, block, slot, level, normal);
        write_level(gpu, &self.material, block, slot, level, material);
    }

    /// Records a copy of all of `from`'s levels of a layer into this tier's matching levels.
    pub fn copy_lower_levels(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        from: &TierArrays,
        block: bool,
        from_slot: u32,
        to_slot: u32,
    ) {
        let offset = (self.edge / from.edge).ilog2();
        for level in 0..from.edge.ilog2() + 1 {
            let edge = (from.edge >> level).max(1);
            let extent = if block { edge.div_ceil(4) * 4 } else { edge };
            for (source, target) in [
                (&from.color, &self.color),
                (&from.normal, &self.normal),
                (&from.material, &self.material),
            ] {
                encoder.copy_texture_to_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture: source,
                        mip_level: level,
                        origin: wgpu::Origin3d {
                            x: 0,
                            y: 0,
                            z: from_slot,
                        },
                        aspect: wgpu::TextureAspect::All,
                    },
                    wgpu::TexelCopyTextureInfo {
                        texture: target,
                        mip_level: level + offset,
                        origin: wgpu::Origin3d {
                            x: 0,
                            y: 0,
                            z: to_slot,
                        },
                        aspect: wgpu::TextureAspect::All,
                    },
                    wgpu::Extent3d {
                        width: extent,
                        height: extent,
                        depth_or_array_layers: 1,
                    },
                );
            }
        }
    }
}

/// Writes one level of one layer of one array.
fn write_level(
    gpu: &Gpu,
    texture: &wgpu::Texture,
    block: bool,
    slot: u32,
    level: u32,
    data: &[u8],
) {
    let edge = (texture.width() >> level).max(1);
    // Copies of block formats cover whole blocks, even when the mip itself is smaller.
    let (copy_edge, rows, bytes_per_row) = if block {
        let blocks = edge.div_ceil(4);
        (blocks * 4, blocks, blocks * 16)
    } else {
        let texel_bytes = data.len() as u32 / (edge * edge);
        (edge, edge, edge * texel_bytes)
    };
    gpu.queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: level,
            origin: wgpu::Origin3d {
                x: 0,
                y: 0,
                z: slot,
            },
            aspect: wgpu::TextureAspect::All,
        },
        data,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(bytes_per_row),
            rows_per_image: Some(rows),
        },
        wgpu::Extent3d {
            width: copy_edge,
            height: copy_edge,
            depth_or_array_layers: 1,
        },
    );
}

/// Creates the texture arrays and fills what's known, or uses 1x1 stand-ins if the scene has
/// no textures.
fn upload_textures(
    gpu: &Gpu,
    scene: &PagedScene,
    upper_textures: Option<&[TextureLayer]>,
    plan: Option<TierPlan>,
) -> (TextureArrays, String) {
    let has_textures = !scene.base_textures.is_empty();
    // The stand-ins for a scene without textures are never blocks, whatever it says it holds.
    let stored_as_blocks = has_textures && scene.texture_encoding == TextureEncoding::Block;
    let block = stored_as_blocks && gpu.block_compressed_textures;
    let unpack = stored_as_blocks && !gpu.block_compressed_textures;
    let layers = scene.base_textures.len().max(1) as u32;

    // The base tier holds the whole chain if it's all here, else the levels in the manifest.
    let largest = if upper_textures.is_some() {
        scene.texture_size
    } else {
        scene.texture_size.min(BASE_TEXTURE_EDGE)
    };
    // Unpacking can drop a chain's largest levels, so it no longer starts at `texture_size`.
    let mut skip = 0;
    if unpack {
        while largest >> skip > UNPACKED_MAX_EDGE {
            skip += 1;
        }
    }
    let size = if has_textures { largest >> skip } else { 1 };

    let base = TierArrays::new(gpu, size, layers, block, "base");
    let tier = |edge: Option<u32>, slots: u32, name: &str| match edge.filter(|_| has_textures) {
        Some(edge) => TierArrays::new(gpu, edge, slots.max(1), block, name),
        None => TierArrays::new(gpu, 1, 1, false, name),
    };
    let middle = tier(plan.and_then(|p| p.middle_edge), layers, "middle");
    let (top_edge, top_slots) = plan.and_then(|p| p.top).unzip();
    let top = tier(top_edge, top_slots.unwrap_or(1), "top");

    let mut written = 0u64;
    if has_textures {
        let empty = TextureLayer::default();
        for (layer, set) in scene.base_textures.iter().enumerate() {
            let upper = upper_textures.map_or(&empty, |upper| &upper[layer]);
            let color_chain = upper.color_mips.iter().chain(&set.color_mips);
            let normal_chain = upper.normal_mips.iter().chain(&set.normal_mips);
            let material_chain = upper.material_mips.iter().chain(&set.material_mips);
            let levels = color_chain.zip(normal_chain).zip(material_chain).skip(skip);
            for (level, ((color, normal), material)) in levels.enumerate() {
                written += (color.len() + normal.len() + material.len()) as u64;
                let edge = (size >> level).max(1);
                if unpack {
                    base.write_level(
                        gpu,
                        block,
                        layer as u32,
                        level as u32,
                        [
                            &unpack_bc7(edge, color),
                            &unpack_bc5(edge, normal),
                            &unpack_bc5(edge, material),
                        ],
                    );
                } else {
                    base.write_level(
                        gpu,
                        block,
                        layer as u32,
                        level as u32,
                        [color, normal, material],
                    );
                }
            }
        }
    } else {
        base.write_level(gpu, false, 0, 0, [&[255; 4], &[128; 2], &[128; 2]]);
    }

    let form = if block {
        "BC7 + BC5"
    } else if unpack {
        "RGBA8 + RG8 unpacked from BC"
    } else {
        "RGBA8 + RG8"
    };
    let (bytes, summary) = match plan.filter(|_| has_textures) {
        Some(plan) => {
            let encoding = if block {
                TextureEncoding::Block
            } else {
                TextureEncoding::Raw
            };
            let bytes = plan.bytes(layers, encoding);
            let tiers = format!(
                "{layers} x {}{}{}",
                plan.base_edge,
                plan.middle_edge
                    .map_or(String::new(), |edge| format!(" / {edge}")),
                plan.top.map_or(String::new(), |(edge, slots)| format!(
                    " / {slots} x {edge}"
                )),
            );
            (
                bytes,
                format!("{tiers}, {form}, {:.0} MB", bytes as f64 / 1.0e6),
            )
        }
        None => (
            written,
            format!(
                "{layers} x {size}, {form}, {:.0} MB",
                written as f64 / 1.0e6
            ),
        ),
    };
    log::info!("textures: {summary}");
    (
        TextureArrays {
            base,
            middle,
            top,
            block,
            unpack,
            bytes,
        },
        summary,
    )
}

/// Unpacks a BC7 mip level to RGBA8.
pub fn unpack_bc7(edge: u32, blocks: &[u8]) -> Vec<u8> {
    unpack_blocks::<4>(edge, blocks, block_compression::decode::decode_block_bc7)
}

/// Unpacks a BC5 mip level to RG8.
pub fn unpack_bc5(edge: u32, blocks: &[u8]) -> Vec<u8> {
    unpack_blocks::<2>(edge, blocks, block_compression::decode::decode_block_bc5)
}

/// Runs a 16-byte block decoder over a whole mip level, cropping the blocks that a mip
/// smaller than 4x4 only partly uses.
fn unpack_blocks<const TEXEL_BYTES: usize>(
    edge: u32,
    blocks: &[u8],
    decode: impl Fn(&[u8], &mut [u8], usize),
) -> Vec<u8> {
    let edge = edge as usize;
    let blocks_wide = edge.div_ceil(4);
    let pitch = edge * TEXEL_BYTES;
    let mut out = vec![0; edge * pitch];
    let mut texels = [0u8; 64];
    for (index, block) in blocks.chunks_exact(16).enumerate() {
        let (x, y) = (index % blocks_wide * 4, index / blocks_wide * 4);
        decode(block, &mut texels, 4 * TEXEL_BYTES);
        let row_bytes = (edge - x).min(4) * TEXEL_BYTES;
        for row in 0..(edge - y).min(4) {
            let at = (y + row) * pitch + x * TEXEL_BYTES;
            let from = row * 4 * TEXEL_BYTES;
            out[at..at + row_bytes].copy_from_slice(&texels[from..from + row_bytes]);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_instance_lays_out_as_its_shader_twin() {
        crate::shaders::assert_twin!(
            "vg/cull.wgsl",
            "Instance",
            GpuInstance,
            [
                world,
                world + 16,
                world + 32,
                prev_world,
                prev_world + 16,
                prev_world + 32,
                normal,
                normal + 16,
                normal + 32,
                quant_origin,
                quant_step,
                uv_origin,
                uv_step,
                root_node,
                material,
                max_scale,
                texture_info,
                glow,
                bones,
                bone_lists,
                lod_margin,
            ]
        );
    }

    #[test]
    fn a_transform_comes_back_as_it_went_in() {
        let world = Mat4::from_scale_rotation_translation(
            Vec3::new(1.0, 2.0, 3.0),
            glam::Quat::from_rotation_y(0.7),
            Vec3::new(4.0, -5.0, 6.0),
        );
        assert!(from_affine_rows(affine_rows(world)).abs_diff_eq(world, 1.0e-6));
    }

    /// A BC5 block whose two channels are constant: both endpoints equal, all indices zero.
    fn flat_bc5_block(red: u8, green: u8) -> [u8; 16] {
        let mut block = [0; 16];
        block[0] = red;
        block[1] = red;
        block[8] = green;
        block[9] = green;
        block
    }

    #[test]
    fn materials_start_in_the_base_tier_at_their_own_layer() {
        let material = |texture_layer| Material {
            texture_layer,
            texture_tier: 2,
            texture_slot: 9,
            ..Default::default()
        };
        let scene = PagedScene {
            materials: vec![material(3), material(-1)],
            ..Default::default()
        };
        let started = starting_materials(&scene);
        assert_eq!((started[0].texture_tier, started[0].texture_slot), (0, 3));
        assert_eq!(started[1].texture_tier, 0);
        assert_eq!(starting_materials(&PagedScene::default()).len(), 1);
    }

    #[test]
    fn unpacking_places_blocks_and_crops_small_mips() {
        let row = [flat_bc5_block(10, 20), flat_bc5_block(30, 40)].concat();
        let unpacked = unpack_bc5(8, &[row.clone(), row].concat());
        assert_eq!(unpacked.len(), 8 * 8 * 2);
        assert_eq!(&unpacked[..2], &[10, 20]);
        assert_eq!(&unpacked[4 * 2..4 * 2 + 2], &[30, 40]);
        assert_eq!(&unpacked[(7 * 8 + 7) * 2..], &[30, 40]);

        let cropped = unpack_bc5(2, &flat_bc5_block(50, 60));
        assert_eq!(cropped, [50, 60, 50, 60, 50, 60, 50, 60]);
    }
}
