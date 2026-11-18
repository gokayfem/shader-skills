// Radiance stored in two words as 16-bit values on a square-root curve, which spends most
// of the precision on dark values, where the eye and the running average both need it.

const RADIANCE_RANGE: f32 = 512.0;

fn encode_radiance(c: vec3f) -> vec2u {
    let e = sqrt(clamp(c / RADIANCE_RANGE, vec3f(0.0), vec3f(1.0)));
    return vec2u(pack2x16unorm(e.rg), pack2x16unorm(vec2f(e.b, 0.0)));
}

fn decode_radiance(packed: vec2u) -> vec3f {
    let e = vec3f(unpack2x16unorm(packed.x), unpack2x16unorm(packed.y).x);
    return e * e * RADIANCE_RANGE;
}
