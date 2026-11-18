// What is drawn in the sky beyond the air: the sun, the moon, the stars and the Milky Way.
//
// The stars sit on a celestial sphere placed by the frame's pole and equinox, so they keep their
// places as the sky turns and the Milky Way stands where it does on that night. Everything here
// is seen through the atmosphere, which dims and reddens it near the horizon.
//
// Needs common/environment.wgsl, and `milky_way_tex`: the map `sky/milky_way_bake.wgsl` makes.

#include "sky/milky_way.wgsl"

// Angular radii, in radians. Both are larger than the true quarter of a degree, so that the
// sun reads at screen resolutions and the moon's seas and phase show.
const SUN_ANGULAR_RADIUS: f32 = 0.0065;
const MOON_ANGULAR_RADIUS: f32 = 0.0075;
// The sun disc's radiance per unit of sunlight. The true disc would be vastly brighter; this
// still blooms without one pixel's value dominating the exposure.
const SUN_DISC_RADIANCE: f32 = 120.0;
// The Milky Way's brightest glow, against the brightest stars.
const MILKY_WAY_RADIANCE: f32 = 0.05;

// Light that crosses the whole atmosphere to reach the viewer along `direction`: dimmed and
// reddened near the horizon, and nothing from below it.
fn air_transmittance(direction: vec3f) -> vec3f {
    let r = viewer_radius();
    let t = textureSampleLevel(transmittance_lut, clamp_sampler, transmittance_uv(r, direction.y), 0.0).rgb;
    return t * above_horizon(r, direction.y);
}

// How much of a pixel `pixel_angle` across lies inside a disc of `radius`, `angle` from its
// centre: a one-pixel ramp, so the edge is crisp at any resolution.
fn disc_coverage(angle: f32, radius: f32, pixel_angle: f32) -> f32 {
    return clamp((radius - angle) / pixel_angle + 0.5, 0.0, 1.0);
}

// Into the celestial sphere's own frame: right ascension 0 along x, 6 hours along y, and the
// north celestial pole along z.
fn to_celestial(direction: vec3f) -> vec3f {
    let pole = frame.star_pole;
    let equinox = frame.star_equinox;
    return vec3f(dot(direction, equinox), dot(direction, cross(pole, equinox)), dot(direction, pole));
}

// ---- The sun ----

fn sun_body(direction: vec3f, pixel_angle: f32) -> vec3f {
    let toward = frame.sun_disc;
    // The chord between unit vectors: the angle, for small angles, without acos's lost
    // precision near 1.
    let angle = length(direction - toward);
    let cover = disc_coverage(angle, SUN_ANGULAR_RADIUS, pixel_angle);
    if cover <= 0.0 {
        return vec3f(0.0);
    }
    // Limb darkening: towards the edge one sees the sun's cooler, outer layers, more so in blue.
    let mu = sqrt(max(1.0 - sq(min(angle / SUN_ANGULAR_RADIUS, 1.0)), 0.0));
    let limb = (vec3f(1.0) - vec3f(0.55, 0.62, 0.72) * (1.0 - mu)) / (vec3f(1.0) - vec3f(0.55, 0.62, 0.72) / 3.0);
    return air_transmittance(toward) * frame.sun_color * frame.daylight_illuminance
        * SUN_DISC_RADIANCE * limb * cover;
}

// ---- The moon ----

// Dark seas of old lava on the moon's near side, as (latitude, longitude, radius across,
// radius north-south) in degrees, sized from their true diameters: east is towards Mare
// Crisium, on the right as seen from the northern hemisphere. Overlapping, they join into the
// great connected dark regions the eye sees.
const MARIA_COUNT: u32 = 17u;
const MARIA = array<vec4f, 17>(
    vec4f(18.0, -58.0, 22.0, 32.0), // Oceanus Procellarum
    vec4f(-4.0, -43.0, 14.0, 16.0), // its southern reach, towards Mare Nubium
    vec4f(34.0, -17.0, 20.0, 16.0), // Mare Imbrium
    vec4f(27.5, 17.5, 12.0, 11.0), // Mare Serenitatis
    vec4f(8.0, 30.0, 15.0, 12.0), // Mare Tranquillitatis
    vec4f(17.0, 59.0, 9.0, 8.0), // Mare Crisium
    vec4f(-7.0, 52.0, 9.0, 13.0), // Mare Fecunditatis
    vec4f(-15.0, 35.0, 6.0, 6.0), // Mare Nectaris
    vec4f(-20.0, -15.0, 12.0, 9.0), // Mare Nubium
    vec4f(-10.0, -23.0, 6.0, 5.0), // Mare Cognitum
    vec4f(7.5, -30.0, 9.0, 7.0), // Mare Insularum
    vec4f(-24.0, -39.0, 7.0, 6.5), // Mare Humorum
    vec4f(56.0, 0.0, 40.0, 5.0), // Mare Frigoris
    vec4f(13.0, 4.0, 5.5, 4.0), // Mare Vaporum
    vec4f(2.0, 1.0, 5.0, 3.5), // Sinus Medii
    vec4f(45.0, -31.0, 5.0, 4.0), // Sinus Iridum
    vec4f(52.0, -52.0, 10.0, 6.0), // Sinus Roris
);

// A young crater's bright rays: splashed rock flung out in long thin streaks, plainest at full
// moon. `crater` is its position on the unit sphere, `reach` how far the rays run (radians).
fn crater_rays(p: vec3f, crater: vec3f, reach: f32, salt: u32) -> f32 {
    let distance = acos(clamp(dot(p, crater), -1.0, 1.0));
    // Bearing around the crater, in a frame on its tangent plane.
    let east = normalize(cross(vec3f(0.0, 1.0, 0.0), crater));
    let north = cross(crater, east);
    let around = normalize(vec2f(dot(p, east), dot(p, north)) + vec2f(1.0e-6, 0.0));
    let circle = vec3f(around, 0.0);
    // Many thin streaks of different strengths, each running its own length.
    let streaks = smoothstep(0.62, 0.85, value_noise(circle * 16.0, salt)) * 0.7
        + smoothstep(0.6, 0.9, value_noise(circle * 45.0, salt + 7u)) * 0.45;
    let length = reach * mix(0.35, 1.4, value_noise(circle * 16.0, salt + 13u));
    let fade = exp(-distance / length) * smoothstep(0.0, 0.02, distance);
    // The crater itself, and the bright blanket of rock thrown out around it.
    let halo = exp(-sq(distance / 0.025));
    return streaks * fade + halo;
}

// A point on the moon in its own frame (x towards lunar east, y north, z towards the Earth),
// to selenographic latitude and longitude, in degrees.
fn selenographic(p: vec3f) -> vec2f {
    return vec2f(asin(clamp(p.y, -1.0, 1.0)), atan2(p.x, p.z)) * (180.0 / PI);
}

// The moon's surface at `p`: its albedo, and a bump that tilts the normal across x and y.
// `full` is 1 near full moon, when the bright rays stand out.
fn moon_surface(p: vec3f, full: f32) -> vec3f {
    // The seas' outlines, warped so that no shore is a clean curve: broad bends of up to ten
    // degrees, then finer ones.
    let warp = vec2f(fbm(p * 3.0, 3u, 5u), fbm(p * 3.0, 3u, 7u)) - vec2f(0.5);
    let fine_warp = vec2f(fbm(p * 9.0, 3u, 9u), fbm(p * 9.0, 3u, 10u)) - vec2f(0.5);
    let coords = selenographic(p) + warp * 20.0 + fine_warp * 7.0;
    let shore = (value_noise(p * 40.0, 13u) - 0.5) * 0.08;
    var dry = 1.0;
    for (var i = 0u; i < MARIA_COUNT; i++) {
        let mare = MARIA[i];
        let offset = vec2f(
            (coords.y - mare.y) * cos(radians(coords.x)) / mare.z,
            (coords.x - mare.x) / mare.w,
        );
        dry *= 1.0 - smoothstep(1.0, 0.86, length(offset) + shore);
    }
    let sea = 1.0 - dry;
    // Highlands pale, seas dark, each varied in tone across their breadth, and mottled by
    // craters too small to see one by one.
    let tone = fbm(p * 4.0, 3u, 17u);
    let lava = fbm(p * 8.0, 4u, 19u);
    let mottle = fbm(p * 26.0, 4u, 23u);
    var albedo = mix(0.92 + 0.16 * tone, 0.33 + 0.2 * lava, sea) * (0.9 + 0.2 * mottle);

    // The rayed craters: Tycho in the southern highlands, Copernicus and Kepler on the seas, and
    // Aristarchus, the brightest spot on the moon.
    let tycho = vec3f(-0.1433, -0.6859, 0.7134);
    let copernicus = vec3f(-0.3385, 0.1671, 0.9260);
    let kepler = vec3f(-0.6096, 0.1412, 0.7800);
    let aristarchus = vec3f(-0.6748, 0.4024, 0.6186);
    let rays = crater_rays(p, tycho, 0.5, 31u) * 1.4
        + crater_rays(p, copernicus, 0.2, 37u) * 0.9
        + crater_rays(p, kepler, 0.14, 41u) * 0.6
        + exp(-sq(acos(clamp(dot(p, aristarchus), -1.0, 1.0)) / 0.02)) * 0.8;
    albedo += rays * mix(0.12, 0.6, full);

    // Relief: craters and hills that roughen the shadow line between day and night.
    let scale = 34.0;
    let step = 0.4;
    let h = fbm(p * scale, 4u, 53u);
    let bump = vec2f(
        fbm(p * scale + vec3f(step, 0.0, 0.0), 4u, 53u) - h,
        fbm(p * scale + vec3f(0.0, step, 0.0), 4u, 53u) - h,
    );
    return vec3f(albedo, -bump * 0.6);
}

// The moon, lit by the sun at its phase and faintly by the Earth. Returns its light, and in A how
// much of the pixel it covers, to hide the stars behind it.
fn moon_body(direction: vec3f, pixel_angle: f32) -> vec4f {
    if frame.moon_radiance <= 0.0 {
        return vec4f(0.0);
    }
    let toward = frame.moon_disc;
    let offset = direction - toward;
    let cover = disc_coverage(length(offset), MOON_ANGULAR_RADIUS, pixel_angle);
    if cover <= 0.0 {
        return vec4f(0.0);
    }
    // The moon keeps one face to the Earth, its north towards the celestial pole: to the viewer,
    // its north is up the sky towards the pole and its east to the right.
    var north = frame.star_pole - toward * dot(frame.star_pole, toward);
    north = select(vec3f(0.0, 1.0, 0.0), normalize(north), dot(north, north) > 1.0e-8);
    let right = cross(toward, north);
    var q = vec2f(dot(offset, right), dot(offset, north)) / MOON_ANGULAR_RADIUS;
    q *= min(1.0, 0.999 / max(length(q), 1.0e-6));
    let z = sqrt(max(1.0 - dot(q, q), 0.0));

    // Phase: the angle at the moon between the sun and the viewer, 0 at full.
    let sun = frame.sun_disc;
    let cos_phase = dot(sun, -toward);
    let full = smoothstep(0.6, 1.0, cos_phase);
    let surface = moon_surface(vec3f(q, z), full);
    let normal = normalize(right * (q.x + surface.y * z) + north * (q.y + surface.z * z) - toward * z);

    // Lommel-Seeliger reflection, as from dusty regolith: the full moon is as bright at its edge
    // as at its centre, and the shadow line is soft.
    let mu_sun = max(dot(normal, sun), 0.0);
    let mu_view = max(dot(normal, -toward), 0.0);
    let sunlit = mu_sun / max(mu_sun + mu_view, 1.0e-4) * 2.0;
    // Close to full the shadows between grains vanish and the moon brightens sharply.
    let surge = 1.0 + 0.2 * pow(max(cos_phase, 0.0), 60.0);
    // Earthshine: sunlight off the Earth, which from the moon is nearly full when the moon is a
    // thin crescent, lighting its dark side a dim ashen grey.
    let earth = sq(0.5 - 0.5 * cos_phase);
    let earthshine = 0.035 * earth;
    let light = surface.x * (sunlit * surge + earthshine);
    // Grey rock, a touch warm.
    let color = vec3f(1.0, 0.94, 0.86);
    let radiance = color * light * frame.moon_radiance * air_transmittance(toward);
    return vec4f(radiance * cover, cover);
}

// ---- The stars ----

// A star's colour from its surface temperature in kelvin: Planck's law at red, green and blue,
// set to equal luminance, and paled, as the eye sees star colours only faintly.
fn star_color(kelvin: f32) -> vec3f {
    let wavelength = vec3f(0.610, 0.550, 0.465); // micrometres
    let planck = 1.0 / (pow(wavelength, vec3f(5.0)) * (exp(14388.0 / (wavelength * kelvin)) - 1.0));
    return mix(vec3f(1.0), planck / luminance(planck), 0.65);
}

// Half the thickness of the shell of cells that stars are scattered through, in cells. Thin, so
// that every star lies within half a cell of the sphere and is found among the eight cells
// nearest the view direction, rather than the twenty-seven around it.
const STAR_SHELL: f32 = 0.15;

// One layer of stars from `brightest` to `faintest` magnitude, scattered through a shell of cells
// `cells` across the unit sphere, with `chance` of a star in each; `height` is the view
// direction's y, for twinkling. Each star is a soft spot about a pixel across, whatever the
// resolution, so it holds steady under jitter.
fn star_layer(c: vec3f, height: f32, cells: f32, chance: f32, brightest: f32, faintest: f32, salt: u32, pixel_angle: f32) -> vec3f {
    let p = c * cells;
    let base = floor(p - vec3f(0.5));
    // Spots end where a star might otherwise be cut off by the cells searched.
    let reach = min(pixel_angle * 3.5, (0.5 - STAR_SHELL) / cells);
    var light = vec3f(0.0);
    for (var i = 0u; i < 8u; i++) {
        let cell = base + vec3f(f32(i & 1u), f32((i >> 1u) & 1u), f32(i >> 2u));
        let ic = bitcast<vec3u>(vec3i(cell));
        let seed = pcg(ic.x ^ pcg(ic.y ^ pcg(ic.z ^ salt)));
        if hash_to_unit(seed) > chance {
            continue;
        }
        let point = cell + vec3f(hash_to_unit(seed + 1u), hash_to_unit(seed + 2u), hash_to_unit(seed + 3u));
        // Only points in the shell count, which spreads stars evenly over the sphere.
        let radius = length(point);
        if abs(radius - cells) > STAR_SHELL {
            continue;
        }
        // The chord between unit vectors: the angle, without acos's lost precision near 1.
        let angle = length(point / radius - c);
        if angle > reach {
            continue;
        }
        // Magnitude: each fainter step holds about three times as many stars.
        let u = hash_to_unit(seed + 4u);
        let range = exp2((brightest - faintest) * 0.5 * 3.3219);
        let magnitude = faintest + 2.0 * log2(mix(range, 1.0, u)) * 0.30103;
        // Shown with a gentler spread of brightness than the true 2.5 per magnitude, as the eye
        // and long exposures see them, so the faint ones still register.
        let brightness = exp2(-magnitude * 0.16 * 3.3219);
        // Mostly sun-like, some cool and red, a few hot and blue.
        let t = hash_to_unit(seed + 5u);
        let kelvin = select(mix(3200.0, 7500.0, t / 0.85), mix(9000.0, 25000.0, (t - 0.85) / 0.15), t > 0.85);
        // Twinkling, from turbulence in the air: stronger lower down, through more of it.
        let rate = mix(5.0, 11.0, hash_to_unit(seed + 6u));
        let phase = hash_to_unit(seed + 7u) * TAU;
        let strength = mix(0.4, 0.1, smoothstep(0.0, 0.6, height));
        let twinkle = 1.0 + strength * (0.6 * sin(frame.time * rate + phase) + 0.4 * sin(frame.time * rate * 2.3 + phase * 1.7));
        // Brighter stars look larger, as they spill across the eye.
        let spread = pixel_angle * (0.7 + 0.35 * sqrt(brightness));
        light += star_color(kelvin) * brightness * twinkle * exp(-(angle * angle) / (spread * spread));
    }
    return light;
}

// The stars and the Milky Way towards `direction`, through the air.
fn night_sky_bodies(direction: vec3f, pixel_angle: f32) -> vec3f {
    if frame.stars <= 0.0 || direction.y < -0.02 {
        return vec3f(0.0);
    }
    let c = to_celestial(direction);
    let baked = textureSampleLevel(milky_way_tex, repeat_sampler, celestial_uv(c), 0.0);
    let galaxy = vec4f(baked.rgb * baked.rgb * MILKY_WAY_RANGE, baked.a);
    // A few bright stars, more middling ones, and many faint ones, thickest along the Milky Way.
    // About 150, 2500 and 8000 to 13000 over the whole sky.
    var stars = star_layer(c, direction.y, 12.0, 0.28, -1.0, 2.5, 1u, pixel_angle)
        + star_layer(c, direction.y, 28.0, 0.85, 2.5, 4.5, 2u, pixel_angle)
        + star_layer(c, direction.y, 60.0, min(0.6 * (1.0 + 0.7 * galaxy.a), 1.0), 4.5, 6.5, 3u, pixel_angle);
    return (stars + galaxy.rgb * MILKY_WAY_RADIANCE) * frame.stars * air_transmittance(direction);
}

// Everything beyond the air towards `direction`, to add to the clear sky.
fn sky_bodies(direction: vec3f, pixel_angle: f32) -> vec3f {
    let moon = moon_body(direction, pixel_angle);
    // The moon hides the stars behind it.
    return sun_body(direction, pixel_angle) + moon.rgb
        + night_sky_bodies(direction, pixel_angle) * (1.0 - moon.a);
}
