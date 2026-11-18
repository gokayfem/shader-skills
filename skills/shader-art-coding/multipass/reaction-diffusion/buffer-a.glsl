// Gray-Scott reaction-diffusion. Buffer A channel 0 = Buffer A, floating-point, nearest.
// The display pass is independent of the solver. Click and drag to seed the B field.
const vec2 DIFFUSION = vec2(0.16, 0.08);
const float FEED = 0.035;
const float KILL = 0.061;

vec2 fieldAt(ivec2 pixel, ivec2 size) {
    // Clamp neighbors to impose zero outward flux.
    return texelFetch(iChannel0, clamp(pixel, ivec2(0), size - 1), 0).rg;
}

void mainImage(out vec4 fragColor, in vec2 fragCoord) {
    ivec2 p = ivec2(fragCoord), size = ivec2(iResolution.xy);
    vec2 uv = fragCoord / iResolution.xy;
    vec2 chemicals;
    if (iFrame == 0) {
        vec2 grid = fragCoord / 21.0;
        vec2 id = floor(grid);
        vec3 jitter = hash32(id);
        vec2 center = mix(vec2(0.22), vec2(0.78), jitter.xy);
        float seed = step(length(fract(grid) - center), 0.12 + 0.09*jitter.z);
        chemicals = mix(vec2(1,0),vec2(0.43,0.30+0.17*jitter.x),seed);
    } else {
        chemicals = fieldAt(p,size);
        vec2 lap = fieldAt(p+ivec2(1,0),size) + fieldAt(p-ivec2(1,0),size)
                 + fieldAt(p+ivec2(0,1),size) + fieldAt(p-ivec2(0,1),size) - 4.0*chemicals;
        // A spatial feed gradient gives the left and right sides different pattern regimes.
        float feed = FEED + 0.016 * (uv.x - 0.5);
        float reaction = chemicals.x * chemicals.y * chemicals.y;
        chemicals += DIFFUSION*lap + vec2(feed*(1.0-chemicals.x)-reaction,
                                           reaction-(KILL+feed)*chemicals.y);
    }
    if (iMouse.z > 0.0 && length(fragCoord-iMouse.xy)<5.0) chemicals = vec2(0.42,0.48);
    fragColor=vec4(chemicals,0.0,1.0);
}
