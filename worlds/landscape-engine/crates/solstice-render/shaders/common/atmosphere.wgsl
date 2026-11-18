// Physical atmosphere model and parameterisations, after Hillaire 2020, "A Scalable and
// Production Ready Sky and Atmosphere Rendering Technique". Distances in kilometres.

const GROUND_RADIUS: f32 = 6360.0;
const TOP_RADIUS: f32 = 6460.0;

const RAYLEIGH_SCATTERING: vec3f = vec3f(5.802e-3, 13.558e-3, 33.1e-3);
const RAYLEIGH_SCALE_HEIGHT: f32 = 8.0;
// Several times the clear-air value: desert dust, which pales and warms the sky.
const MIE_SCATTERING: f32 = 14.0e-3;
const MIE_ABSORPTION: f32 = 4.40e-3;
const MIE_SCALE_HEIGHT: f32 = 1.6;
const MIE_G: f32 = 0.8;
const OZONE_ABSORPTION: vec3f = vec3f(0.650e-3, 1.881e-3, 0.085e-3);

struct Medium {
    rayleigh: vec3f,
    mie: f32,
    extinction: vec3f,
}

fn sample_medium(height: f32) -> Medium {
    let h = max(height, 0.0);
    let rayleigh = RAYLEIGH_SCATTERING * exp(-h / RAYLEIGH_SCALE_HEIGHT);
    let mie_density = exp(-h / MIE_SCALE_HEIGHT);
    let mie = MIE_SCATTERING * mie_density;
    let ozone = OZONE_ABSORPTION * max(0.0, 1.0 - abs(h - 25.0) / 15.0);
    return Medium(rayleigh, mie, rayleigh + vec3f(mie + MIE_ABSORPTION * mie_density) + ozone);
}

fn rayleigh_phase(cos_theta: f32) -> f32 {
    return 3.0 * (1.0 + cos_theta * cos_theta) / (16.0 * PI);
}

fn mie_phase(cos_theta: f32) -> f32 {
    // Cornette-Shanks.
    let g = MIE_G;
    let k = 3.0 / (8.0 * PI) * (1.0 - g * g) / (2.0 + g * g);
    let d = 1.0 + g * g - 2.0 * g * cos_theta;
    return k * (1.0 + cos_theta * cos_theta) / (d * sqrt(d));
}

// Distance along a ray to a sphere around the planet centre, or -1 if it misses.
fn ray_sphere(origin: vec3f, direction: vec3f, radius: f32) -> f32 {
    let b = dot(origin, direction);
    let c = dot(origin, origin) - radius * radius;
    let discriminant = b * b - c;
    if discriminant < 0.0 {
        return -1.0;
    }
    let root = sqrt(discriminant);
    if -b - root > 0.0 {
        return -b - root;
    }
    if -b + root > 0.0 {
        return -b + root;
    }
    return -1.0;
}

// Bruneton's mapping for the transmittance LUT: more resolution near the horizon.
fn transmittance_uv(r: f32, mu: f32) -> vec2f {
    let h = sqrt(TOP_RADIUS * TOP_RADIUS - GROUND_RADIUS * GROUND_RADIUS);
    let rho = sqrt(max(r * r - GROUND_RADIUS * GROUND_RADIUS, 0.0));
    let discriminant = r * r * (mu * mu - 1.0) + TOP_RADIUS * TOP_RADIUS;
    let d = max(0.0, -r * mu + sqrt(max(discriminant, 0.0)));
    let d_min = TOP_RADIUS - r;
    let d_max = rho + h;
    return vec2f((d - d_min) / (d_max - d_min), rho / h);
}

// Inverse of `transmittance_uv`; returns (r, mu).
fn transmittance_params(uv: vec2f) -> vec2f {
    let h = sqrt(TOP_RADIUS * TOP_RADIUS - GROUND_RADIUS * GROUND_RADIUS);
    let rho = h * uv.y;
    let r = sqrt(rho * rho + GROUND_RADIUS * GROUND_RADIUS);
    let d_min = TOP_RADIUS - r;
    let d_max = rho + h;
    let d = d_min + uv.x * (d_max - d_min);
    var mu = 1.0;
    if d > 0.0 {
        mu = clamp((h * h - rho * rho - d * d) / (2.0 * r * d), -1.0, 1.0);
    }
    return vec2f(r, mu);
}

// Sky-view LUT mapping: world azimuth across U, and square-rooted elevation across V so
// that the horizon, where the sky changes fastest, gets the most texels.
fn sky_view_uv(direction: vec3f) -> vec2f {
    let azimuth = atan2(direction.z, direction.x);
    let elevation = asin(clamp(direction.y, -1.0, 1.0));
    let v = 0.5 + 0.5 * sign(elevation) * sqrt(abs(elevation) / (PI * 0.5));
    return vec2f(azimuth / TAU + 0.5, 1.0 - v);
}

// Inverse of `sky_view_uv`.
fn sky_view_direction(uv: vec2f) -> vec3f {
    let azimuth = (uv.x - 0.5) * TAU;
    let s = (1.0 - uv.y) * 2.0 - 1.0;
    let elevation = sign(s) * s * s * PI * 0.5;
    return vec3f(cos(elevation) * cos(azimuth), sin(elevation), cos(elevation) * sin(azimuth));
}
