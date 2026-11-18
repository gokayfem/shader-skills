// The microfacet shading model: GGX distribution, height-correlated Smith visibility
// and Schlick Fresnel. Roughness is perceptual and squared before use.

fn d_ggx(n_dot_h: f32, a: f32) -> f32 {
    let a2 = a * a;
    let d = n_dot_h * n_dot_h * (a2 - 1.0) + 1.0;
    return a2 / max(PI * d * d, 1.0e-7);
}

// Height-correlated Smith visibility, in the approximate form that avoids square roots.
fn v_smith(n_dot_v: f32, n_dot_l: f32, a: f32) -> f32 {
    let v = n_dot_l * (n_dot_v * (1.0 - a) + a);
    let l = n_dot_v * (n_dot_l * (1.0 - a) + a);
    return 0.5 / max(v + l, 1.0e-5);
}

fn f_schlick(f0: vec3f, v_dot_h: f32) -> vec3f {
    let fc = pow(1.0 - v_dot_h, 5.0);
    return f0 + (vec3f(1.0) - f0) * fc;
}

struct Surface {
    diffuse: vec3f,
    f0: vec3f,
    roughness: f32,
    normal: vec3f,
}

fn make_surface(albedo: vec3f, metallic: f32, roughness: f32, normal: vec3f) -> Surface {
    return Surface(
        albedo * (1.0 - metallic),
        mix(vec3f(0.04), albedo, metallic),
        max(roughness, 0.045),
        normal,
    );
}

// Outgoing radiance per unit of incoming irradiance at normal incidence, from one light.
fn shade_light(s: Surface, view: vec3f, light: vec3f) -> vec3f {
    let n_dot_l = dot(s.normal, light);
    if n_dot_l <= 0.0 {
        return vec3f(0.0);
    }
    let n_dot_v = max(dot(s.normal, view), 1.0e-3);
    let h = normalize(view + light);
    let n_dot_h = clamp(dot(s.normal, h), 0.0, 1.0);
    let v_dot_h = clamp(dot(view, h), 0.0, 1.0);
    let a = s.roughness * s.roughness;
    let specular = f_schlick(s.f0, v_dot_h) * d_ggx(n_dot_h, a) * v_smith(n_dot_v, n_dot_l, a);
    return (s.diffuse / PI + specular) * n_dot_l;
}

// Pre-integrated environment BRDF (Lazarov's fit of the split-sum), so that image-based
// specular does not need a lookup texture.
fn environment_brdf(f0: vec3f, roughness: f32, n_dot_v: f32) -> vec3f {
    let c0 = vec4f(-1.0, -0.0275, -0.572, 0.022);
    let c1 = vec4f(1.0, 0.0425, 1.04, -0.04);
    let r = roughness * c0 + c1;
    let a004 = min(r.x * r.x, exp2(-9.28 * n_dot_v)) * r.x + r.y;
    let ab = vec2f(-1.04, 1.04) * a004 + r.zw;
    return f0 * ab.x + vec3f(ab.y);
}
