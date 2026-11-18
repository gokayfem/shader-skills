// The mean of the cloud shadow map: how much sunlight the clouds let through on average over
// the square around the camera. Beyond the map, lighting fades to this rather than to whatever
// its edge texel holds, which, stretched to the horizon, would draw streaks on the far sea.
// One workgroup reads every fourth texel each way, 16 texels a thread at 256²: the shadows
// are far broader than that, and the mean is for the horizon, where no texel can be told.

@group(0) @binding(0) var shadow_map: texture_2d<f32>;
@group(0) @binding(1) var output: texture_storage_2d<rgba16float, write>;

const THREADS: u32 = 256u;
const STRIDE: u32 = 4u;

var<workgroup> sums: array<f32, THREADS>;

@compute @workgroup_size(THREADS)
fn main(@builtin(local_invocation_index) thread: u32) {
    let size = textureDimensions(shadow_map) / STRIDE;
    let count = size.x * size.y;
    var sum = 0.0;
    for (var texel = thread; texel < count; texel += THREADS) {
        let at = vec2u(texel % size.x, texel / size.x) * STRIDE + STRIDE / 2u;
        sum += textureLoad(shadow_map, at, 0).r;
    }
    sums[thread] = sum;
    workgroupBarrier();
    for (var stride = THREADS / 2u; stride > 0u; stride /= 2u) {
        if thread < stride {
            sums[thread] += sums[thread + stride];
        }
        workgroupBarrier();
    }
    if thread == 0u {
        textureStore(output, vec2u(0u), vec4f(sums[0] / f32(count), 0.0, 0.0, 1.0));
    }
}
