// The Milky Way, fixed on the celestial sphere, and the noise it and the moon are made from.
// The Milky Way costs too much to work out for every pixel of sky, so `sky/milky_way_bake.wgsl`
// works it out once into a map of the celestial sphere, which the sky samples.
//
// Needs common/math.wgsl.

// The galactic frame, in equatorial coordinates (J2000): towards the galactic centre in
// Sagittarius, towards galactic longitude 90 degrees in Cygnus, and the north galactic pole.
const GALACTIC_CENTRE: vec3f = vec3f(-0.0548756, -0.8734371, -0.4838350);
const GALACTIC_90: vec3f = vec3f(0.4941094, -0.4448296, 0.7469822);
const GALACTIC_POLE: vec3f = vec3f(-0.8676661, -0.1980764, 0.4559838);

// A hash of an integer lattice point, to [0, 1).
fn lattice_hash(cell: vec3i, salt: u32) -> f32 {
    let c = bitcast<vec3u>(cell);
    return hash_to_unit(pcg(c.x ^ pcg(c.y ^ pcg(c.z ^ salt))));
}

// Smooth value noise in [0, 1].
fn value_noise(p: vec3f, salt: u32) -> f32 {
    let cell = vec3i(floor(p));
    let f = fract(p);
    let s = f * f * (3.0 - 2.0 * f);
    let x00 = mix(lattice_hash(cell, salt), lattice_hash(cell + vec3i(1, 0, 0), salt), s.x);
    let x10 = mix(lattice_hash(cell + vec3i(0, 1, 0), salt), lattice_hash(cell + vec3i(1, 1, 0), salt), s.x);
    let x01 = mix(lattice_hash(cell + vec3i(0, 0, 1), salt), lattice_hash(cell + vec3i(1, 0, 1), salt), s.x);
    let x11 = mix(lattice_hash(cell + vec3i(0, 1, 1), salt), lattice_hash(cell + vec3i(1, 1, 1), salt), s.x);
    return mix(mix(x00, x10, s.y), mix(x01, x11, s.y), s.z);
}

// Fractal noise in [0, 1]: `octaves` layers of value noise, each twice as fine and half as
// strong.
fn fbm(p: vec3f, octaves: u32, salt: u32) -> f32 {
    var sum = 0.0;
    var amplitude = 0.5;
    var total = 0.0;
    var q = p;
    for (var i = 0u; i < octaves; i++) {
        sum += amplitude * value_noise(q, salt + i * 101u);
        total += amplitude;
        amplitude *= 0.5;
        q = q * 2.03 + vec3f(1.7, 9.2, 4.1);
    }
    return sum / total;
}

// The Milky Way's glow towards `c` (celestial), and in A how dense its faint stars are there.
fn milky_way(c: vec3f) -> vec4f {
    let g = vec3f(dot(c, GALACTIC_CENTRE), dot(c, GALACTIC_90), dot(c, GALACTIC_POLE));
    let latitude = asin(clamp(g.z, -1.0, 1.0));
    let longitude = atan2(g.y, g.x);
    let toward_centre = 0.5 + 0.5 * cos(longitude);
    // The disc, thickening and brightening towards the bulge round the centre.
    let width = 0.07 + 0.07 * pow(toward_centre, 4.0);
    let disc = exp(-sq(latitude / width)) * (0.2 + 0.8 * pow(toward_centre, 2.5));
    let bulge = exp(-(sq(longitude / 0.3) + sq(latitude / 0.2)));
    // Star clouds, mottled at several scales, and grainy with stars too faint to see one by one.
    let clouds = fbm(g * 7.0, 7u, 71u);
    let grain = 0.75 + 0.5 * value_noise(g * 260.0, 73u);
    var glow = (disc * (0.3 + 1.4 * clouds * clouds) + bulge * 0.8) * grain;
    // Dust: dark lanes along the plane, deepest in the Great Rift from Cygnus to Sagittarius.
    // The lane wanders and swells along its length, thickens and thins into clumps, frays at
    // its edges, and throws off fainter side lanes.
    let rift = smoothstep(-0.5, -0.1, longitude) * smoothstep(1.6, 1.1, longitude);
    let lane_offset = 0.012 + 0.03 * (fbm(g * 5.0, 5u, 83u) - 0.5);
    let lane_width = 0.01 + 0.024 * fbm(g * 8.0, 4u, 87u);
    let ragged = (fbm(g * 30.0, 5u, 89u) - 0.5) * 0.014;
    let opacity = smoothstep(0.25, 0.75, fbm(g * 11.0, 5u, 91u));
    let lane = exp(-sq((latitude - lane_offset + ragged) / lane_width)) * rift * mix(0.35, 0.95, opacity);
    let side_offset = -0.035 + 0.04 * (fbm(g * 6.0, 4u, 93u) - 0.5);
    let side = exp(-sq((latitude - side_offset + ragged) / 0.008)) * rift * smoothstep(0.5, 0.8, fbm(g * 14.0, 4u, 95u));
    let filaments = smoothstep(0.52, 0.78, fbm(g * 45.0, 5u, 97u)) * exp(-sq(latitude / 0.08));
    let dust = clamp(lane + side * 0.6 + filaments * 0.4, 0.0, 0.9);
    glow *= 1.0 - dust;
    // Old yellow stars towards the core, young blue ones out along the arms; dust reddens.
    let color = mix(vec3f(0.78, 0.86, 1.0), vec3f(1.0, 0.86, 0.68), pow(toward_centre, 3.0))
        * mix(vec3f(1.0), vec3f(1.0, 0.78, 0.6), dust);
    return vec4f(color * glow, disc * (1.0 - dust));
}

// Where `c`, a direction on the celestial sphere, falls on the baked map: right ascension across,
// from 0 to 24 hours, and declination down, from the north pole.
fn celestial_uv(c: vec3f) -> vec2f {
    return vec2f(atan2(c.y, c.x) / TAU + 0.5, acos(clamp(c.z, -1.0, 1.0)) / PI);
}

// The direction on the celestial sphere at `uv` on the baked map.
fn celestial_direction(uv: vec2f) -> vec3f {
    let ra = (uv.x - 0.5) * TAU;
    let polar = uv.y * PI;
    return vec3f(sin(polar) * cos(ra), sin(polar) * sin(ra), cos(polar));
}

// The map stores the square root of the glow over this range, for precision in the faint parts
// of eight-bit channels.
const MILKY_WAY_RANGE: f32 = 4.0;
