// Shared depth-tested write into the visibility buffer, used by both rasterizers.
//
// Nanite packs depth and ID in one 64-bit atomic. WebGPU only has 32-bit atomics, so
// they live in separate buffers: depth is the atomic and decides who wins, and the ID
// follows. If two triangles race on one pixel, the ID may briefly belong to the loser.
// That is a one-frame, one-pixel error at worst, which temporal AA hides.

@group(1) @binding(0) var<storage, read_write> vis_depth: array<atomic<u32>>;
@group(1) @binding(1) var<storage, read_write> vis_id: array<atomic<u32>>;

// `depth` is reversed Z, greater is closer. Positive IEEE floats order the same way
// as their bit patterns, so the raw bits compare correctly as integers.
fn write_visibility(index: u32, depth: f32, id: u32) {
    let bits = bitcast<u32>(depth);
    var current = atomicLoad(&vis_depth[index]);
    loop {
        if bits <= current {
            return;
        }
        let result = atomicCompareExchangeWeak(&vis_depth[index], current, bits);
        if result.exchanged {
            break;
        }
        current = result.old_value;
    }
    atomicStore(&vis_id[index], id);
}
