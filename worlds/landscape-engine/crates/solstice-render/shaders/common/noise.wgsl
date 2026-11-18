// Gradient noise, shared by particles and the wind. Twin of `gradient_noise` in
// particles/reference.rs.

#include "common/math.wgsl"

// One of the hashed gradients of the noise lattice.
fn lattice_gradient(cell: vec3i, seed: u32) -> vec3f {
    let h = pcg(u32(cell.x) + pcg(u32(cell.y) + pcg(u32(cell.z) + seed)));
    return vec3f(hash_to_unit(h), hash_to_unit(pcg(h)), hash_to_unit(pcg(pcg(h)))) * 2.0 - vec3f(1.0);
}

// Gradient noise at `p`, in x, and its gradient, in yzw, with quintic smoothing so the gradient
// is continuous.
fn gradient_noise(p: vec3f, seed: u32) -> vec4f {
    let i = floor(p);
    let f = p - i;
    let u = f * f * f * (f * (f * 6.0 - vec3f(15.0)) + vec3f(10.0));
    let du = f * f * 30.0 * (f * (f - vec3f(2.0)) + vec3f(1.0));
    let c = vec3i(i);
    let ga = lattice_gradient(c, seed);
    let gb = lattice_gradient(c + vec3i(1, 0, 0), seed);
    let gc = lattice_gradient(c + vec3i(0, 1, 0), seed);
    let gd = lattice_gradient(c + vec3i(1, 1, 0), seed);
    let ge = lattice_gradient(c + vec3i(0, 0, 1), seed);
    let gf = lattice_gradient(c + vec3i(1, 0, 1), seed);
    let gg = lattice_gradient(c + vec3i(0, 1, 1), seed);
    let gh = lattice_gradient(c + vec3i(1, 1, 1), seed);
    let va = dot(ga, f);
    let vb = dot(gb, f - vec3f(1.0, 0.0, 0.0));
    let vc = dot(gc, f - vec3f(0.0, 1.0, 0.0));
    let vd = dot(gd, f - vec3f(1.0, 1.0, 0.0));
    let ve = dot(ge, f - vec3f(0.0, 0.0, 1.0));
    let vf = dot(gf, f - vec3f(1.0, 0.0, 1.0));
    let vg = dot(gg, f - vec3f(0.0, 1.0, 1.0));
    let vh = dot(gh, f - vec3f(1.0, 1.0, 1.0));
    let k1 = vb - va;
    let k2 = vc - va;
    let k3 = ve - va;
    let k4 = va - vb - vc + vd;
    let k5 = va - vc - ve + vg;
    let k6 = va - vb - ve + vf;
    let k7 = -va + vb + vc - vd + ve - vf - vg + vh;
    let value = va + u.x * k1 + u.y * k2 + u.z * k3 + u.x * u.y * k4 + u.y * u.z * k5
        + u.z * u.x * k6 + u.x * u.y * u.z * k7;
    let gradient = ga + u.x * (gb - ga) + u.y * (gc - ga) + u.z * (ge - ga)
        + u.x * u.y * (ga - gb - gc + gd) + u.y * u.z * (ga - gc - ge + gg)
        + u.z * u.x * (ga - gb - ge + gf) + u.x * u.y * u.z * (-ga + gb + gc - gd + ge - gf - gg + gh)
        + du * (vec3f(k1, k2, k3) + u.yzx * vec3f(k4, k5, k6) + u.zxy * vec3f(k6, k4, k5)
            + u.yzx * u.zxy * k7);
    return vec4f(value, gradient);
}
