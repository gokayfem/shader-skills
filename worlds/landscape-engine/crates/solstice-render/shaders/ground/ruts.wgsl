// The ground as tyres have left it, and mud as it lies (docs/ruts.md).
//
// The land is baked: its points are half a metre apart, an eighth along a track, and none of
// them move. A rut is a hand deep and a tyre wide, and its tread's print a centimetre, so a
// rut is not made of the land's points. It is kept as a map of how far the ground has been
// pressed down or heaped up at each place (ruts.rs), and drawn here, where each pixel's
// material is worked out:
//
//   1. The eye's ray is followed through the pressed ground to where it meets it
//      (`rut_trace`): relief mapping, with the land's own surface as the level nothing has
//      been pressed from. So a rut is seen into, its near wall hides its floor, and a berm
//      stands in front of what is behind it.
//   2. There the ground's textures are laid, its facing is the pressed ground's, and it is
//      shaded as what it is (`rut_shade`): churned soil where the ground was broken, the
//      tread's print on the floor, darker where it lies low and where the sun does not
//      reach in, wet where it is wet and under water where water stands.
//
// And mud nobody has driven on is drawn as mud (`mud_lies`): wet clay with a sheen and
// water standing in its hollows at its heart, drying to a pale cracked crust at its rim.
//
// The including shader declares `frame: Frame`, `material_sampler`, `rut_map` (a
// texture_2d_array<f32>: ruts.rs) and includes common/noise.wgsl.

// Twins of HEIGHT_UNIT, HEIGHT_ZERO and ACROSS_REACH in solstice-vehicle's rut.rs.
const RUT_HEIGHT_UNIT: f32 = 0.00002;
const RUT_HEIGHT_ZERO: f32 = 32768.0;
const RUT_ACROSS_REACH: f32 = 0.32;
// The smallest size the map is kept at, which is where it is asked whether anything is near.
const RUT_LAST_LEVEL: f32 = 5.0;
// How far above the ground as it was made, and below it, the eye's ray is followed, in
// metres: the highest berm and all but the deepest rut.
const RUT_ABOVE: f32 = 0.13;
const RUT_BELOW: f32 = 0.42;
// How far along the ground it is followed at most, in metres, either way of the pixel.
const RUT_REACH: f32 = 1.6;
// How far off ruts are still seen into, in metres: beyond it they are shaded where they lie.
const RUT_TRACED_TO: f32 = 45.0;
// How deep a tread's print is, in metres, and how far off it is still drawn.
const RUT_PRINT_DEPTH: f32 = 0.007;
const RUT_PRINT_TO: f32 = 14.0;
// How far over its last metres the held square of ruts fades into ground nobody pressed.
const RUT_EDGE: f32 = 3.0;

// Twin of Material::MUD: ground that is mud, to be drawn as mud.
const MATERIAL_MUD: u32 = 512u;

// The pressed ground where the eye's ray meets it.
struct Rut {
    // Whether anything pressed is near: if not, nothing else here means anything.
    near: bool,
    // Where the ray meets it, in the world.
    at: vec3f,
    // Its height there against the ground as it was made: below zero in a rut.
    height: f32,
    // The way it faces.
    normal: vec3f,
    // How much of the sky and of the sun reach it, from 0 to 1.
    open: f32,
    lit: f32,
    // How far across the tyre that pressed it the place is, in metres.
    across: f32,
    // How wet it is and how much churned soil shows, from 0 to 1.
    wet: f32,
    churn: f32,
    // The tread's print: how crisp, from 0 to 1, and where in the tread's pattern, in turns.
    print: f32,
    phase: f32,
    // The size the map was read at, and how much of all this there is this far off and
    // this near the edge of what is held.
    lod: f32,
    fade: f32,
}

fn rut_uv(xz: vec2f) -> vec2f {
    return xz * frame.rut_window.w;
}

// How much of the map counts at a place: all of it well inside the square that is held,
// none of it at its edge, where what the texture holds is from the far side.
fn rut_fade(xz: vec2f) -> f32 {
    let from_edge = frame.rut_window.z - max(abs(xz.x - frame.rut_window.x), abs(xz.y - frame.rut_window.y));
    return clamp(from_edge / RUT_EDGE, 0.0, 1.0);
}

// The ground's height at a place against how it was made, in metres.
fn rut_height(xz: vec2f, lod: f32) -> f32 {
    let t = textureSampleLevel(rut_map, material_sampler, rut_uv(xz), 0, lod);
    // The height's two bytes are blended apart and put together here.
    return ((t.r * 65280.0 + t.g * 255.0) - RUT_HEIGHT_ZERO) * RUT_HEIGHT_UNIT;
}

// A tyre's tread as it prints: how far a place is pressed in, from 0, a groove's ridge, to
// 1, under a lug. `turns` counts lugs along the way the tyre rolled, and `across` is metres
// from its middle. A mud tyre's: blocks down the middle, staggered side to side, and
// lugs at each shoulder that reach in between them.
fn rut_tread(turns: f32, across: f32) -> f32 {
    let v = across / 0.15;
    let side = select(0.5, 0.0, v > 0.0);
    let a = abs(v);
    // The blocks down the middle lean back from the centre line; the shoulder's lean
    // forward, and sit in the gaps.
    let middle = fract(turns + side + 0.3 * a);
    let shoulder = fract(turns + side + 0.5 - 0.45 * a);
    let block = smoothstep(0.0, 0.1, middle) * (1.0 - smoothstep(0.56, 0.66, middle));
    let lug = smoothstep(0.0, 0.1, shoulder) * (1.0 - smoothstep(0.5, 0.6, shoulder));
    // The grooves that run round the tyre: one down its middle, one between the blocks
    // and the lugs.
    let rings = smoothstep(0.03, 0.09, a) * smoothstep(0.035, 0.09, abs(a - 0.5));
    return mix(block, lug, smoothstep(0.46, 0.54, a)) * rings * (1.0 - smoothstep(0.95, 1.1, a));
}

// What the tread printed at a place: how deep, in metres, and how crisp.
fn rut_print(xz: vec2f, lod: f32) -> vec2f {
    let uv = rut_uv(xz);
    let ground = textureSampleLevel(rut_map, material_sampler, uv, 0, lod);
    let marks = textureSampleLevel(rut_map, material_sampler, uv, 1, lod);
    let wave = (marks.rg * 255.0 - vec2f(128.0)) / 127.0;
    let crisp = min(length(wave), 1.0);
    if crisp < 0.02 {
        return vec2f(0.0);
    }
    let turns = atan2(wave.y, wave.x) / 6.2831853;
    let across = (ground.b * 2.0 - 1.0) * RUT_ACROSS_REACH;
    return vec2f(rut_tread(turns, across) * crisp * RUT_PRINT_DEPTH, crisp);
}

// Follows the eye's ray to the pressed ground. `world` is the pixel's place on the land as
// it was made, `n` the way the land faces there, and `footprint` a pixel's width on it.
fn rut_trace(world: vec3f, n: vec3f, footprint: f32) -> Rut {
    var rut = Rut(false, world, 0.0, n, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0);
    let fade = rut_fade(world.xz);
    if fade <= 0.0 || n.y < 0.5 {
        return rut;
    }
    // Whether anything pressed is within a metre or so: one look at the map at its
    // smallest, where a square with one rut in it still says so.
    let near = textureSampleLevel(rut_map, material_sampler, rut_uv(world.xz), 1, RUT_LAST_LEVEL).a;
    if near <= 0.0 {
        return rut;
    }
    let cell = frame.rut_cell;
    let to_eye = frame.camera_position - world;
    let range = length(to_eye);
    let v = -to_eye / max(range, 1.0e-4);
    let lod = clamp(log2(max(footprint / cell, 1.0)), 0.0, RUT_LAST_LEVEL - 1.0);
    rut.near = true;
    rut.lod = lod;
    rut.fade = fade;

    // The land near the pixel is taken as flat, facing `n`. Along the ray, `t` metres on
    // from the pixel, the ray stands `-t * fall` above it, straight up.
    let fall = -dot(v, n) / n.y;
    var at = world;
    if fall > 0.02 && range < RUT_TRACED_TO {
        let level = v.xz;
        let per_metre = max(length(level), 1.0e-3);
        // From where the ray comes down through the highest a berm stands to where it
        // would go under the deepest rut, and no further along the ground than its reach.
        let first = -min(RUT_ABOVE / fall, RUT_REACH / per_metre);
        let last = min(RUT_BELOW / fall, RUT_REACH / per_metre);
        // Fewer steps far off, where a step is under a pixel anyway.
        let steps = u32(mix(22.0, 8.0, clamp(range / RUT_TRACED_TO, 0.0, 1.0)));
        // Each pixel starts a little differently along its first step, which turns the
        // steps' banding into grain the temporal filter clears.
        let jitter = interleaved_gradient_noise(world.xz * 173.0, frame.frame_index % 8u);
        let stride = (last - first) / f32(steps);
        var before = first;
        var t = first + stride * jitter;
        var hit = false;
        for (var k = 0u; k <= steps; k++) {
            let p = world + v * t;
            let above = -t * fall - rut_height(p.xz, lod) * fade;
            if above <= 0.0 {
                hit = true;
                break;
            }
            before = t;
            t += stride;
        }
        if hit {
            // Between the last step above the ground and the first below it, by halves.
            var low = before;
            var high = t;
            for (var k = 0u; k < 5u; k++) {
                let middle = 0.5 * (low + high);
                let p = world + v * middle;
                if -middle * fall - rut_height(p.xz, lod) * fade <= 0.0 {
                    high = middle;
                } else {
                    low = middle;
                }
            }
            at = world + v * (0.5 * (low + high));
        }
    }
    rut.at = at;
    rut.height = rut_height(at.xz, lod) * fade;

    // The way it faces: the land's own facing, tilted by how the pressed ground slopes.
    let reach = max(cell, footprint) * 0.75;
    let slope = vec2f(
        rut_height(at.xz + vec2f(reach, 0.0), lod) - rut_height(at.xz - vec2f(reach, 0.0), lod),
        rut_height(at.xz + vec2f(0.0, reach), lod) - rut_height(at.xz - vec2f(0.0, reach), lod),
    ) * fade / (2.0 * reach);
    rut.normal = normalize(vec3f(n.x - slope.x * n.y, n.y, n.z - slope.y * n.y));

    // How much of the sky reaches it: less the lower it lies below the ground about it.
    let about = rut_height(at.xz, min(lod + 3.0, RUT_LAST_LEVEL)) * fade;
    rut.open = 1.0 - 0.45 * smoothstep(0.01, 0.16, about - rut.height);
    // And of the sun: a few steps towards it over the ground, for what stands in its way.
    let sun = frame.sun_direction;
    if sun.y > 0.02 && range < RUT_TRACED_TO * 0.6 {
        let level = length(sun.xz);
        // As far as the highest berm could still shade the deepest rut, and no further.
        let far = min(0.9, (RUT_ABOVE - rut.height) * level / max(sun.y, 0.05));
        var shade = 0.0;
        for (var k = 1u; k <= 6u; k++) {
            let s = far * f32(k) / 6.0;
            let p = at.xz + sun.xz / max(level, 1.0e-3) * s;
            let ray = rut.height + s * sun.y / max(level, 1.0e-3);
            let over = rut_height(p, lod) * fade - ray;
            // What stands well over the ray shades wholly; what just reaches it, softly.
            shade = max(shade, smoothstep(0.0, 0.015 + 0.03 * s, over));
        }
        rut.lit = 1.0 - shade;
    }

    let uv = rut_uv(at.xz);
    let ground = textureSampleLevel(rut_map, material_sampler, uv, 0, lod);
    let marks = textureSampleLevel(rut_map, material_sampler, uv, 1, lod);
    rut.across = (ground.b * 2.0 - 1.0) * RUT_ACROSS_REACH;
    rut.wet = ground.a * fade;
    rut.churn = marks.b * fade;
    let wave = (marks.rg * 255.0 - vec2f(128.0)) / 127.0;
    rut.print = min(length(wave), 1.0) * fade * (1.0 - smoothstep(RUT_PRINT_TO * 0.5, RUT_PRINT_TO, range));
    rut.phase = atan2(wave.y, wave.x) / 6.2831853;
    return rut;
}

// A surface of the ground before it meets the light, and the water on it.
struct Ground {
    albedo: vec3f,
    roughness: f32,
    cavity: f32,
    // The way it faces with its fine bumps, and without them.
    normal: vec3f,
    surface_normal: vec3f,
    // How wet it is, from 0 to 1, and how much of it is under standing water.
    wet: f32,
    pool: f32,
    // How much of the light that falls on the ground about it reaches it, from 0 to 1: laid
    // on its colour once that is as wet as it is, since wetting darkens what is dark most.
    shade: f32,
}

// The ground where it has been pressed: churned soil, the tread's print, and the shade of
// lying low. `mud` is how much of the ground here is mud already, and `rain` how full the
// rain has left the puddles, each from 0 to 1.
fn rut_shade(surface: Ground, rut: Rut, mud: f32, rain: f32) -> Ground {
    var out = surface;
    if !rut.near {
        return out;
    }
    let depth = max(-rut.height, 0.0);
    let heaped = max(rut.height, 0.0);
    let p = vec3f(rut.at.x, 0.0, rut.at.z);

    // Churned soil in place of whatever covered the ground: clods and smears of it, never
    // one flat colour.
    if rut.churn > 0.004 {
        let clods = gradient_noise(p * 9.0, 0x51a3u);
        let smears = gradient_noise(p * 2.3, 0x51a4u);
        let grit = gradient_noise(p * 37.0, 0x51a5u);
        let shows = clamp(rut.churn * (1.15 + 0.5 * clods.x + 0.3 * smears.x), 0.0, 1.0);
        // What is churned up is what the ground is made of: mud's own colour where it is
        // mud, and under anything else the soil of the country.
        let made_of = mix(frame.rut_soil, out.albedo * 0.92, clamp(mud, 0.0, 1.0));
        let soil = made_of * (0.86 + 0.26 * smears.x + 0.2 * clods.x + 0.12 * grit.x);
        // A berm is crumbs of what was pressed out: drier and paler at its crest.
        let crest = smoothstep(0.01, 0.07, heaped);
        out.albedo = mix(out.albedo, soil * (1.0 + 0.1 * crest), shows);
        out.roughness = mix(out.roughness, 0.92, shows);
        out.cavity = mix(out.cavity, clamp(0.72 + 0.5 * clods.x, 0.0, 1.0), shows * 0.6);
        // Lumps: more of them on the berm, fewer on a floor a tyre has smoothed.
        let lumps = (clods.yw * 0.11 + grit.yw * 0.035) * shows * (0.55 + 3.0 * crest);
        out.normal = normalize(out.normal + vec3f(-lumps.x, 0.0, -lumps.y));
    }

    // The tread's print on the floor: each lug pressed in, each groove a ridge.
    if rut.print > 0.03 {
        let nudge = max(frame.rut_cell * 0.3, 0.008);
        let here = rut_print(rut.at.xz, rut.lod);
        let east = rut_print(rut.at.xz + vec2f(nudge, 0.0), rut.lod);
        let south = rut_print(rut.at.xz + vec2f(0.0, nudge), rut.lod);
        let slope = vec2f(east.x - here.x, south.x - here.x) / nudge * rut.print;
        out.normal = normalize(out.normal + vec3f(slope.x, 0.0, slope.y));
        // Under a lug the soil is packed, and darker for it; a ridge catches the light.
        let pressed = here.x / RUT_PRINT_DEPTH;
        out.albedo *= 1.0 - 0.3 * rut.print * (pressed - 0.45);
        out.cavity *= 1.0 - 0.35 * rut.print * pressed;
        out.roughness = mix(out.roughness, out.roughness * 0.8, rut.print * pressed);
    }

    // The geometry is the pressed ground's: water lies level in it, not on the land above.
    out.surface_normal = rut.normal;
    // Low ground sees less of the sky; and less of the sun where a wall stands in its way.
    out.cavity *= rut.open;
    out.shade = mix(0.62, 1.0, rut.lit) * mix(0.9, 1.0, rut.open);

    // Wet where it was churned wet, or a wet tyre ran.
    out.wet = max(out.wet, rut.wet);
    // Water stands in what is deep enough: the wetter the ground, and the fuller the rain
    // has left the puddles, the shallower a rut need be to hold it.
    let soaked = clamp((rut.wet - 0.55) / 0.45 + rain, 0.0, 1.0);
    if soaked > 0.0 && depth > 0.0 {
        let brim = mix(0.22, 0.025, soaked);
        // Its edge wanders, as a puddle's does.
        let wander = gradient_noise(p * 3.1, 0x51a6u).x * 0.012;
        out.pool = max(out.pool, smoothstep(0.0, 0.012, depth - brim + wander) * rut.fade);
    }
    return out;
}

// The distance from a place to the nearest crack in dried mud, in cells, and a number for
// the plate it is on: the plates are the cells of points scattered one to a square.
fn mud_cracks(at: vec2f) -> vec2f {
    let square = floor(at);
    let within = at - square;
    var nearest = 8.0;
    var next = 8.0;
    var plate = 0.0;
    for (var j = -1; j <= 1; j++) {
        for (var i = -1; i <= 1; i++) {
            let other = vec2f(f32(i), f32(j));
            let key = vec2i(square + other);
            let h = pcg((u32(key.x) * 0x9e3779b1u) ^ pcg(u32(key.y) * 0x85ebca77u));
            let point = other + vec2f(f32(h & 0xffffu), f32(h >> 16u)) / 65535.0 * 0.8 + vec2f(0.1);
            let d = length(point - within);
            if d < nearest {
                next = nearest;
                nearest = d;
                plate = f32(h & 0xffu) / 255.0;
            } else if d < next {
                next = d;
            }
        }
    }
    return vec2f(next - nearest, plate);
}

// Mud as it lies where nobody has driven: `share` of the surface is mud. At its heart it
// is wet clay, dark, with a sheen, and water standing in its hollows; towards its rim it
// has dried to a paler crust that has cracked into plates. `rained` is how wet the rain has
// left the ground, and `rain` how full its puddles are, each from 0 to 1.
fn mud_lies(surface: Ground, share: f32, at: vec3f, footprint: f32, rained: f32, rain: f32) -> Ground {
    var out = surface;
    if share <= 0.02 {
        return out;
    }
    let p = vec3f(at.x, 0.0, at.z);
    let broad = gradient_noise(p * 0.21, 0x6d01u).x;
    let patches = gradient_noise(p * 0.9, 0x6d02u).x;
    // How wet: mud is wet where there is most of it, which is where the water gathers,
    // and after rain all of it is.
    let heart = smoothstep(0.5, 0.95, share + 0.45 * broad + 0.2 * patches);
    let damp = max(heart, rained);

    // The dried rim: paler, and cracked into plates a hand or two across.
    let crust = share * (1.0 - damp);
    if crust > 0.01 {
        // Far off the cracks are finer than a pixel: they fade out before they sparkle.
        let seen = 1.0 - smoothstep(0.02, 0.07, footprint);
        let warp = vec2f(gradient_noise(p * 2.7, 0x6d03u).x, gradient_noise(p * 2.7 + 9.0, 0x6d04u).x) * 0.35;
        let cells = mud_cracks(at.xz * 6.5 + warp);
        let fine = mud_cracks(at.xz * 17.0 - warp);
        let crack = (1.0 - smoothstep(0.02, 0.09, cells.x)) * seen;
        let hair = (1.0 - smoothstep(0.015, 0.06, fine.x)) * seen * 0.45;
        // Each plate its own shade, bleached by the sun, a little curled at its edges.
        let pale = out.albedo * (1.4 + 0.28 * cells.y) + vec3f(0.05, 0.043, 0.034);
        out.albedo = mix(out.albedo, mix(pale, out.albedo * 0.4, max(crack, hair)), crust);
        out.roughness = mix(out.roughness, 0.95, crust);
        out.cavity *= 1.0 - 0.7 * max(crack, hair) * crust;
        let curl = (1.0 - smoothstep(0.05, 0.3, cells.x)) * 0.12 * seen;
        let lean = gradient_noise(vec3f(cells.y * 91.0, 0.0, 0.0), 0x6d05u).yw;
        out.normal = normalize(out.normal + vec3f(lean.x, 0.0, lean.y) * (curl + 0.5 * crack) * crust);
    }

    // The wet heart: clay that has taken up all the water it can, with a sheen that comes
    // and goes across it, and water standing in its hollows.
    let wet = share * damp * clamp(0.62 + 0.42 * patches + 0.2 * broad, 0.42, 0.86);
    out.wet = max(out.wet, wet);
    // Wet clay is smooth under its sheen: its grain is filled.
    out.roughness = mix(out.roughness, out.roughness * 0.82, share * damp);
    let hollows = gradient_noise(p * 1.45, 0x6d06u).x + 0.35 * gradient_noise(p * 4.3, 0x6d07u).x;
    // The hollows of the texture's own relief fill first.
    let low = hollows + 0.45 * (0.6 - out.cavity);
    let brim = mix(0.4, 0.05, clamp(0.25 * heart + rain, 0.0, 1.0));
    out.pool = max(out.pool, smoothstep(brim, brim + 0.05, low) * share * smoothstep(0.55, 0.9, damp));
    return out;
}

// Standing water on the ground: muddy, level and smooth. `pool` of the surface is under it.
fn pooled(albedo: vec3f, pool: f32) -> vec3f {
    // Water with clay in it: the ground under it shows through darker and browner.
    let silt = frame.rut_soil * 0.34;
    return mix(albedo, mix(albedo, silt, 0.65), pool);
}
