// What is under one pixel of the screen, for the editor to select: the instance there and its
// depth, read from the visibility buffer after the frame resolves. One thread, and only on a
// frame the editor asks.

// As `VisibleCluster` in `common/scene.wgsl`: what each visibility id indexes.
struct VisibleCluster {
    instance: u32,
    cluster: u32,
}

struct Params {
    pixel: vec2u,
    width: u32,
    _pad: u32,
}

struct Picked {
    // 1 if something was there, else 0 (sky, or outside the screen).
    found: u32,
    instance: u32,
    // Reversed depth, as the visibility buffer holds it.
    depth: f32,
    _pad: u32,
}

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> vis_depth: array<u32>;
@group(0) @binding(2) var<storage, read> vis_id: array<u32>;
@group(0) @binding(3) var<storage, read> visible: array<VisibleCluster>;
@group(0) @binding(4) var<storage, read_write> picked: Picked;

@compute @workgroup_size(1)
fn main() {
    let index = params.pixel.y * params.width + params.pixel.x;
    let depth = bitcast<f32>(vis_depth[index]);
    if depth <= 0.0 {
        picked = Picked(0u, 0u, 0.0, 0u);
        return;
    }
    let entry = visible[vis_id[index] >> 7u];
    picked = Picked(1u, entry.instance, depth, 0u);
}
