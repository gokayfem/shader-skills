// One view through the culling and rasterization passes: the main camera in either
// occlusion phase, a shadow cascade, or a voxelization volume. Twin of `PassView`
// in vg/culling.rs.

struct PassView {
    raster_view_proj: mat4x4f,
    occlusion_view: mat4x4f,
    planes: array<vec4f, 6>,
    camera_position: vec3f,
    // Largest allowed projected error, in pixels.
    lod_threshold: f32,
    viewport: vec2f,
    // Projection scale of the view used for the occlusion test.
    proj_scale: vec2f,
    hzb_size: vec2f,
    near: f32,
    // Pixels per world unit: at unit distance if perspective, everywhere if not.
    lod_scale: f32,
    is_ortho: u32,
    // 0 tests the previous frame's depth; 1 tests this frame's.
    phase: u32,
    occlusion_enabled: u32,
    // 0: hardware only. 1: small clusters go to software. 2: every cluster does.
    allow_software: u32,
    // For an orthographic view drawn for a camera: that camera, its pixels per world unit at
    // unit distance (0 for none), and the nearest it sees anything drawn here.
    eye_position: vec3f,
    eye_lod_scale: f32,
    eye_near: f32,
    // Further planes a cluster must lie inside, and how many of them are in use: for a shadow
    // view, the bounds of what it shadows, open towards the light.
    receiver_plane_count: u32,
    // Which instances the view draws: one of the INSTANCES_ constants.
    instance_filter: u32,
    // For a depth map drawn by the software rasterizer: which drawing this is, 1 to 255.
    stamp: u32,
    receiver_planes: array<vec4f, 6>,
    // Towards the light, for a shadow view.
    eye_light: vec3f,
    // 1 if this view sends cut-out clusters through their own lists and rasterizers; 0 if it
    // draws them with the rest, as solid or testing the mask itself.
    masked_lists: u32,
    // For the camera's own view of a streamed scene: how far around the camera pages are asked
    // for that it isn't facing, at a coarser level of detail. 0 for none.
    prefetch_radius: f32,
    // For a view kept from frame to frame, in a streamed scene: where in the page requests the
    // bits begin that it sets, one for each page it asks for. 0 for none.
    hold_requests: u32,
    // For a depth map or a voxelization: clusters under this many texels or voxels across are
    // drawn as specks, a point each. 0 for none.
    speck_texels: f32,
}

// Twins of `InstanceFilter` in vg/culling.rs.
const INSTANCES_ALL: u32 = 0u;
// Views kept from frame to frame leave out what may move, or they would keep it where it was.
const INSTANCES_STATIC: u32 = 1u;
const INSTANCES_MOVABLE: u32 = 2u;
// The GI voxels: static instances, and movable ones that stay in them.
const INSTANCES_GI: u32 = 3u;

// Written by the cull passes. Atomic view of `CullStateRead`.
struct CullState {
    read_begin: u32,
    read_end: u32,
    write_head: atomic<u32>,
    retry_count: atomic<u32>,
    sw_count: atomic<u32>,
    hw_count: atomic<u32>,
    sw_base: u32,
    hw_base: u32,
    triangle_count: atomic<u32>,
    // Queue entries and clusters that found no room in their lists, and so were lost. Nonzero
    // means the lists are too small for this view, and the image or shadow has holes.
    dropped: atomic<u32>,
    // Entries of the visible list claimed so far, from both ends together.
    reserved: atomic<u32>,
    // A depth map's specks, which fill the retry list: a view without occlusion has no other
    // use for it.
    speck_count: atomic<u32>,
    // The cut-out clusters' lists, which fill the end of the visible list from `opaque_end` on.
    masked_sw_count: atomic<u32>,
    masked_hw_count: atomic<u32>,
    masked_sw_base: u32,
    masked_hw_base: u32,
    masked_reserved: atomic<u32>,
    // Where the opaque lists stop. Set when the list is made, and kept by each frame's clear.
    opaque_end: u32,
}

// Read by the raster passes, after culling has finished.
struct CullStateRead {
    read_begin: u32,
    read_end: u32,
    write_head: u32,
    retry_count: u32,
    sw_count: u32,
    hw_count: u32,
    sw_base: u32,
    hw_base: u32,
    triangle_count: u32,
    dropped: u32,
    reserved: u32,
    speck_count: u32,
    masked_sw_count: u32,
    masked_hw_count: u32,
    masked_sw_base: u32,
    masked_hw_base: u32,
    masked_reserved: u32,
    opaque_end: u32,
}
