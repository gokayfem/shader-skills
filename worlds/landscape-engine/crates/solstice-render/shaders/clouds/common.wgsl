// The cloud model and its ray march, shared by every cloud pass.
//
// Clouds fill a shell above a curved planet, between `clouds.base` and `clouds.top`. Their
// density follows Schneider's method (Horizon: Zero Dawn, 2015), as Unreal's volumetric clouds
// also do: a weather map says where cloud can form and of what kind; a height profile for that
// kind gives the outline from flat base to rounded or anvil top; Perlin-Worley noise carves
// that into puffs; and Worley noise erodes their edges into wisps and cauliflower billows.
// Lighting follows Hillaire's "Physically Based Sky, Atmosphere and Cloud Rendering in
// Frostbite" (2016): a short march towards the sun for self-shadowing, a dual-lobe phase
// function, and multiple scattering approximated by octaves of weaker, broader, deeper light.
//
// The including shader binds, in group 0:
//   shape_noise: texture_3d<f32>, detail_noise: texture_3d<f32>, weather_map: texture_2d<f32>,
//   noise_sampler: sampler (repeating), coverage_cdf: array<f32, 256> (clouds/coverage.wgsl),
// and includes common/environment.wgsl in group 1.

#include "common/environment.wgsl"

const PLANET_RADIUS: f32 = 6360000.0;
// Metres covered by one repeat of the weather map, the shape noise and the detail noise.
const WEATHER_TILE: f32 = 36000.0;
const SHAPE_TILE: f32 = 7000.0;
const DETAIL_TILE: f32 = 700.0;
// Extinction per metre of cloud at density 1. Real cumulus is around 0.05: a little over 50 m
// of it hides the sun.
const EXTINCTION: f32 = 0.05;
// Cloud droplets scatter nearly all the light they meet.
const ALBEDO: f32 = 0.98;
const CIRRUS_HEIGHT: f32 = 9000.0;
// Beyond this, clouds are lost in the haze.
const MAX_DISTANCE: f32 = 90000.0;
// The longest stretch of the layer one ray marches through.
const MAX_PATH: f32 = 40000.0;

// Distance along a ray from height `h` above the ground, rising at `mu` (the sine of its
// elevation), to where it leaves the sphere at height `shell` that contains it. Written to avoid
// the cancellation that planet-sized numbers cause in the textbook formula.
fn shell_exit(h: f32, mu: f32, shell: f32) -> f32 {
    let r = PLANET_RADIUS + h;
    let b = r * mu;
    let c = (h - shell) * (2.0 * PLANET_RADIUS + h + shell);
    let s = sqrt(max(b * b - c, 0.0));
    return select(-b + s, -c / (b + s), b > 0.0);
}

// Height above the ground at distance `t` along a ray from height `h` rising at `mu`.
fn height_along(h: f32, mu: f32, t: f32) -> f32 {
    let r = PLANET_RADIUS + h;
    let grow = t * t + 2.0 * r * t * mu;
    return h + grow / (sqrt(r * r + grow) + r);
}

// The stretch of a ray from height `h` rising at `mu` that lies in the cloud layer, as
// (enter, exit), or exit < enter when it misses: when the ground is in the way, or the layer is
// too far to matter.
fn layer_span(h: f32, mu: f32) -> vec2f {
    var enter = 0.0;
    var exit = 0.0;
    if h < clouds.base {
        // The ground hides the layer from a ray heading down to meet it.
        let r = PLANET_RADIUS + h;
        if mu < 0.0 && r * r * (mu * mu - 1.0) + PLANET_RADIUS * PLANET_RADIUS >= 0.0 {
            return vec2f(1.0, 0.0);
        }
        enter = shell_exit(h, mu, clouds.base);
        exit = shell_exit(h, mu, clouds.top);
    } else if h < clouds.top {
        exit = shell_exit(h, mu, clouds.top);
    } else {
        return vec2f(1.0, 0.0);
    }
    if enter > MAX_DISTANCE {
        return vec2f(1.0, 0.0);
    }
    return vec2f(enter, min(exit, enter + MAX_PATH));
}

// How far up the layer `height` is, 0 at its base and 1 at its top.
fn layer_fraction(height: f32) -> f32 {
    return clamp((height - clouds.base) / max(clouds.top - clouds.base, 1.0), 0.0, 1.0);
}

// Where `value` falls among all the weather map's coverage values, from 0 to 1.
fn coverage_rank(value: f32) -> f32 {
    let x = clamp(value, 0.0, 1.0) * 255.0;
    let i = u32(floor(x));
    return mix(coverage_cdf[i], coverage_cdf[min(i + 1u, 255u)], fract(x));
}

// The weather where the drifting map lies over `xz`.
fn weather_at(xz: vec2f) -> vec4f {
    return textureSampleLevel(weather_map, noise_sampler, (xz - clouds.wind_offset) / WEATHER_TILE, 0.0);
}

// How much of the layer's height cloud of `kind` fills at fraction `h` of the way up: a thin
// sheet low in the layer for stratus, a lumpy band for stratocumulus, and for cumulus a flat
// base rising to rounded tops.
fn height_profile(h: f32, kind: f32) -> f32 {
    let stratus = smoothstep(0.0, 0.1, h) * (1.0 - smoothstep(0.25, 0.4, h));
    let stratocumulus = smoothstep(0.0, 0.12, h) * (1.0 - smoothstep(0.45, 0.75, h));
    let cumulus = smoothstep(0.0, 0.07, h) * (1.0 - smoothstep(0.6, 1.0, h));
    let low = mix(stratus, stratocumulus, clamp(kind * 2.0, 0.0, 1.0));
    return mix(low, cumulus, clamp(kind * 2.0 - 1.0, 0.0, 1.0));
}

// Cloud density at a point `xz` across the ground and `height` above it. The cheap form skips
// the detail noise, for shadows and for finding cloud before marching it finely.
fn cloud_density(xz: vec2f, height: f32, cheap: bool) -> f32 {
    let h = layer_fraction(height);
    if h <= 0.0 || h >= 1.0 || clouds.coverage <= 0.001 {
        return 0.0;
    }
    let weather = weather_at(xz);
    let kind = clamp(clouds.cloud_type + (weather.g - 0.5) * 0.35, 0.0, 1.0);
    // Deep storm clouds spread into anvils at their tops.
    let anvil = clouds.cloud_type * smoothstep(3500.0, 6500.0, clouds.top - clouds.base) * smoothstep(0.65, 0.95, h);
    let coverage = mix(clouds.coverage, sqrt(clouds.coverage), anvil);
    // How much cloud can form here, 0..1: the weather map's rank among all its values, raised
    // or lowered by the coverage asked for. At coverage 0.5 it is the rank itself; less, and
    // only the highest ranks keep any; more, and even the lowest do. The shape noise is then
    // cut against it, so a little gives small scattered puffs and a lot gives a solid deck.
    let local = clamp(coverage_rank(weather.r) + coverage * 2.0 - 1.0, 0.0, 1.0);
    if local <= 0.0 {
        return 0.0;
    }
    var profile = height_profile(h, kind);
    profile = max(profile, anvil * (1.0 - smoothstep(0.95, 1.0, h)));

    // Shapes drift with the weather and boil slowly upwards as they evolve.
    let p = vec3f(xz.x - clouds.wind_offset.x, height - clouds.evolution, xz.y - clouds.wind_offset.y);
    let shape = textureSampleLevel(shape_noise, noise_sampler, p / SHAPE_TILE, 0.0);
    let cells = shape.g * 0.625 + shape.b * 0.25 + shape.a * 0.125;
    let base_shape = remap(shape.r, cells - 1.0, 1.0, 0.0, 1.0);
    var density = clamp(remap(base_shape * profile, 1.0 - local, 1.0, 0.0, 1.0), 0.0, 1.0) * local;
    // Even a solid deck is thicker in some places than others, which shows from below as
    // lighter and darker patches. Without this, full coverage would be a featureless sheet.
    density *= 0.35 + 0.65 * smoothstep(0.15, 0.85, base_shape);
    if cheap || density <= 0.0 {
        return density;
    }
    // Wisps at the base and billows at the top, carved from the edges inwards at two scales:
    // puffs of about 100 m and, within them, curls of about 25 m.
    let skew = vec3f(clouds.wind_direction.x, 0.0, clouds.wind_direction.y) * h * 300.0;
    let q = p + skew;
    let detail = textureSampleLevel(detail_noise, noise_sampler, q / DETAIL_TILE, 0.0);
    let curls = textureSampleLevel(detail_noise, noise_sampler, q / (DETAIL_TILE * 0.27), 0.0);
    let fine = (detail.r * 0.625 + detail.g * 0.25 + detail.b * 0.125) * 0.7
        + (curls.r * 0.625 + curls.g * 0.25 + curls.b * 0.125) * 0.3;
    let erode = mix(fine, 1.0 - fine, clamp(h * 4.0, 0.0, 1.0));
    // Stratus is smooth; heaped cloud is deeply carved.
    let strength = mix(0.25, 0.55, kind);
    density = clamp(remap(density, erode * strength, 1.0, 0.0, 1.0), 0.0, 1.0);
    return density;
}

fn henyey_greenstein(cos_theta: f32, g: f32) -> f32 {
    let d = 1.0 + g * g - 2.0 * g * cos_theta;
    return (1.0 - g * g) / (4.0 * PI * d * sqrt(d));
}

// A strong forward lobe for the silver lining towards the sun, and a weaker back lobe.
fn cloud_phase(cos_theta: f32, scale: f32) -> f32 {
    return mix(henyey_greenstein(cos_theta, -0.25 * scale), henyey_greenstein(cos_theta, 0.8 * scale), 0.7);
}

// Light from the sun scattered towards the eye after crossing `depth` of optical depth inside
// the cloud. Each octave stands for light scattered more times: weaker, spread wider, and
// reaching deeper (Wrenninge et al. 2013).
fn sun_scattering(depth: f32, cos_theta: f32) -> f32 {
    var total = 0.0;
    var strength = 1.0;
    var reach = 1.0;
    var spread = 1.0;
    for (var i = 0; i < 4; i++) {
        total += strength * cloud_phase(cos_theta, spread) * exp(-depth * reach);
        strength *= 0.5;
        reach *= 0.45;
        spread *= 0.5;
    }
    return total;
}

// Sunlight arriving at `height`, after the atmosphere above it.
fn sun_at_height(height: f32) -> vec3f {
    let r = GROUND_RADIUS + height * 0.001;
    let uv = transmittance_uv(r, frame.sun_direction.y);
    let transmittance = textureSampleLevel(transmittance_lut, clamp_sampler, uv, 0.0).rgb;
    return transmittance * frame.sun_color * frame.sun_illuminance
        * above_horizon(r, frame.sun_direction.y);
}

// Optical depth of cloud between a point and the sun, from a few samples along the way that
// grow further apart.
fn depth_towards_sun(xz: vec2f, height: f32, samples: i32) -> f32 {
    let reach = min(clouds.top - clouds.base, 2500.0);
    let to_sun = frame.sun_direction;
    var depth = 0.0;
    var travelled = 0.0;
    for (var i = 0; i < samples; i++) {
        let step = reach * (f32(i) + 1.0) / f32(samples * (samples + 1) / 2);
        let at = travelled + step * 0.5;
        travelled += step;
        let point_height = height + to_sun.y * at;
        if point_height > clouds.top {
            break;
        }
        let density = cloud_density(xz + to_sun.xz * at, point_height, i >= 2);
        depth += density * step;
    }
    return depth * EXTINCTION * clouds.density;
}

struct CloudSample {
    // Light the clouds scatter towards the eye along the ray.
    radiance: vec3f,
    // How much of what lies behind the clouds still shows through.
    transmittance: f32,
    // Roughly where along the ray the cloud is, for reprojection and haze.
    distance: f32,
}

// Marches the clouds along a ray from the camera. `steps` sets quality; `jitter` in 0..1
// offsets the samples so that successive frames, blended, cover the gaps between them.
fn march_clouds(direction: vec3f, steps: i32, light_samples: i32, jitter: f32) -> CloudSample {
    var result = CloudSample(vec3f(0.0), 1.0, MAX_DISTANCE);
    if clouds.enabled < 0.5 {
        return result;
    }
    let h0 = frame.camera_position.y;
    let mu = direction.y;
    let span = layer_span(h0, mu);
    if span.y <= span.x {
        return result;
    }
    let length = span.y - span.x;
    let cos_theta = dot(direction, frame.sun_direction);
    let darken = 1.0 - 0.6 * clouds.rain;
    // Skylight inside the cloud: from the horizon low down, from overhead higher up. The more
    // of the sky is cloud, the more of that light comes from other clouds, which are grey
    // rather than blue.
    let grey = clamp(clouds.coverage * 0.8, 0.0, 0.8);
    let horizon_sky = sky_radiance(normalize(vec3f(direction.x, 0.05, direction.z)));
    let zenith_sky = sky_radiance(vec3f(0.0, 1.0, 0.0));
    let sky_low = mix(horizon_sky, vec3f(luminance(horizon_sky)), grey) * 0.6;
    let sky_high = mix(zenith_sky, vec3f(luminance(zenith_sky)), grey);

    var weighted_distance = 0.0;
    var weight = 0.0;
    // Samples crowd towards the near end of the layer, where cloud is seen largest and in most
    // detail, and spread out into the distance.
    for (var i = 0; i < steps; i++) {
        let s0 = (f32(i) + jitter) / f32(steps);
        let t = span.x + length * pow(s0, 1.6);
        let step = length * (pow(min(s0 + 1.0 / f32(steps), 1.0), 1.6) - pow(s0, 1.6)) + 1.0;
        let height = height_along(h0, mu, t);
        let xz = frame.camera_position.xz + direction.xz * t;
        let coarse = cloud_density(xz, height, true);
        if coarse > 0.0 {
            let density = cloud_density(xz, height, false);
            if density > 0.0 {
                let sigma = density * EXTINCTION * clouds.density;
                let h = layer_fraction(height);
                let to_sun = depth_towards_sun(xz, height, light_samples);
                // Dark creases where light has had little cloud to scatter in yet: the "powder"
                // look of a sunlit cumulus, strongest looking away from the sun.
                let powder = 1.0 - exp(-sigma * 120.0);
                let powder_weight = 0.5 - 0.5 * cos_theta;
                let sun_light = sun_at_height(height);
                // Light scattered a few times keeps its direction (the octaves). Light that
                // has crossed hundreds of metres of cloud has scattered so often that it
                // diffuses, and a cloud passes far more of it than its optical depth suggests:
                // the two-stream result T = 1 / (1 + 0.75 (1 - g) tau), for g about 0.85.
                let diffused = 1.0 / (1.0 + 0.11 * to_sun) / (4.0 * PI);
                let sun = sun_light * (sun_scattering(to_sun, cos_theta) + diffused)
                    * mix(1.0, powder, powder_weight * 0.6);
                // Skylight inside a cloud also diffuses down through the cloud above it.
                let above_height = mix(height, clouds.top, 0.5);
                let above = cloud_density(xz, above_height, true) * (clouds.top - height)
                    * EXTINCTION * clouds.density * 0.5;
                let sky = mix(sky_low, sky_high, h) / (1.0 + 0.11 * above);
                // Sunlit ground lights the bases from below, a large part of why real cloud
                // bases are grey rather than black: its radiance is E * albedo / pi, for an
                // albedo of about 0.25, fading with height into the cloud.
                let ground = sun_at_height(0.0) * max(frame.sun_direction.y, 0.0) * 0.25 / PI
                    * (1.0 - h) * (1.0 - 0.7 * clouds.coverage);
                let source = (sun + sky + ground) * ALBEDO * darken;
                let through = exp(-sigma * step);
                // Integrated exactly over the step for a constant medium (Hillaire 2016).
                result.radiance += result.transmittance * source * (1.0 - through);
                weighted_distance += t * result.transmittance * (1.0 - through);
                weight += result.transmittance * (1.0 - through);
                result.transmittance *= through;
                if result.transmittance < 0.01 {
                    // What little would still show through is not worth marching for; call it
                    // none, or the sun's disc, thousands of times brighter, would show.
                    result.transmittance = 0.0;
                    break;
                }
            }
        }
    }
    if weight > 0.0 {
        result.distance = weighted_distance / weight;
    }
    return result;
}

// High cirrus, a thin sheet far above the main layer, behind it.
fn cirrus_layer(direction: vec3f, behind: CloudSample) -> CloudSample {
    var result = behind;
    if clouds.cirrus <= 0.001 || direction.y <= 0.0 || clouds.enabled < 0.5 {
        return result;
    }
    let h0 = frame.camera_position.y;
    let t = shell_exit(h0, direction.y, CIRRUS_HEIGHT);
    let xz = frame.camera_position.xz + direction.xz * t;
    // Cirrus moves faster, in the stronger winds aloft.
    let uv = (xz - clouds.wind_offset * 2.5) / (WEATHER_TILE * 0.7);
    let streaks = textureSampleLevel(weather_map, noise_sampler, uv, 0.0).b;
    let fibres = textureSampleLevel(detail_noise, noise_sampler, vec3f(uv * 9.0, 0.37), 0.0).r;
    let cover = smoothstep(1.0 - clouds.cirrus, 1.0 - clouds.cirrus + 0.35, streaks * (0.75 + 0.5 * fibres));
    let alpha = cover * 0.55 * smoothstep(0.0, 0.06, direction.y);
    if alpha <= 0.0 {
        return result;
    }
    let cos_theta = dot(direction, frame.sun_direction);
    let light = sun_at_height(CIRRUS_HEIGHT) * henyey_greenstein(cos_theta, 0.6) * 1.6
        + sky_radiance(vec3f(0.0, 1.0, 0.0)) * 0.8;
    // Behind the main layer: seen only through it.
    result.radiance += behind.transmittance * light * alpha;
    result.transmittance *= 1.0 - alpha;
    if behind.transmittance > 0.99 {
        result.distance = t;
    }
    return result;
}

// Fades distant clouds into the haze of the clear sky behind them.
fn with_aerial_perspective(sample: CloudSample, direction: vec3f) -> CloudSample {
    var result = sample;
    let clear = sky_radiance(direction);
    let visible = exp(-sample.distance / 28000.0);
    result.radiance = sample.radiance * visible + clear * (1.0 - sample.transmittance) * (1.0 - visible);
    return result;
}

// Clouds and cirrus along a ray, with aerial perspective.
fn clouds_along(direction: vec3f, steps: i32, light_samples: i32, jitter: f32) -> CloudSample {
    let main = march_clouds(direction, steps, light_samples, jitter);
    return with_aerial_perspective(cirrus_layer(direction, main), direction);
}
