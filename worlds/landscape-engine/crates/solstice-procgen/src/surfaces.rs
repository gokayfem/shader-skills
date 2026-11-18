//! Ready-made surfaces, each fully described by a name and a seed.
//!
//! A seed of 0 gives each surface its reference look. Other seeds give variations of the same
//! material, so a scene can hold `gravel 3` rather than the megabytes of texture it produces.

use crate::cracks::{CrackNetwork, Generation};
use crate::field::{
    billow, fbm, ridged, smoothstep, Blades, Cell, Cells, Facets, Field, Leaves, Stones, Strokes,
};
use crate::foliage::bark::Bark;
use crate::noise::lattice_random;
use crate::surface::{Shade, Surface, TILE_METRES};
use glam::Vec3;

/// Every surface in the library, by name.
pub const NAMES: [&str; 20] = [
    "sand",
    "wet_sand",
    "dirt",
    "dried_mud",
    "gravel",
    "asphalt",
    "grass",
    "moss",
    "forest_floor",
    "rock",
    "granite",
    "snow",
    "sandstone",
    "marble",
    "basalt",
    "oak_bark",
    "birch_bark",
    "fir_bark",
    "pine_bark",
    "beech_bark",
];

/// Finds a surface by name, or `None` if the library has no such surface.
pub fn by_name(name: &str, seed: u32) -> Option<Surface> {
    Some(match name {
        "sand" => sand(seed),
        "wet_sand" => wet_sand(seed),
        "dirt" => dirt(seed),
        "dried_mud" => dried_mud(seed),
        "gravel" => gravel(seed),
        "asphalt" => asphalt(seed),
        "grass" => grass(seed),
        "moss" => moss(seed),
        "forest_floor" => forest_floor(seed),
        "rock" => rock(seed),
        "granite" => granite(seed),
        "snow" => snow(seed),
        "sandstone" => sandstone(seed),
        "marble" => marble(seed),

        "basalt" => basalt(seed),
        _ => {
            return crate::bush::by_name(name, seed)
                .or_else(|| Bark::by_name(name).map(|bark| bark.surface(seed)))
        }
    })
}

fn mix(a: Vec3, b: Vec3, t: f32) -> Vec3 {
    a.lerp(b, t.clamp(0.0, 1.0))
}

/// Linear colour from 8-bit-style sRGB-ish values, for readable palettes.
fn tone(r: f32, g: f32, b: f32) -> Vec3 {
    Vec3::new(r, g, b)
}

/// Dry sand with wind ripples and visible grains.
pub fn sand(seed: u32) -> Surface {
    // Wind ripples: about 12 cm apart and under 1 cm high. Their direction wanders, and they fade
    // out across the tile.
    const RIPPLES_PER_TILE: f32 = 32.0;
    let wander = fbm(2, 3, seed + 200);
    // Small kinks make ripples fork and end, rather than run on like contour lines.
    let breakup = fbm(11, 3, seed + 208);
    let strength = fbm(3, 4, seed + 201).unit().smooth(0.28, 0.7);
    let mounds = fbm(2, 3, seed + 202);
    let unevenness = fbm(24, 3, seed + 203);
    let dimples = fbm(90, 2, seed + 204);
    let grains = fbm(400, 1, seed + 205);
    let dark_grains = fbm(320, 1, seed + 206);
    let tint = fbm(3, 3, seed + 207).unit();
    Surface::shaped(
        move |u, v| {
            let phase = v * RIPPLES_PER_TILE + wander.at(u, v) * 7.0 + breakup.at(u, v) * 1.1;
            // Leaning the sine forward gives a long windward slope and a shorter lee, with no
            // sharp edge anywhere for the light to draw as a line.
            let angle = phase * std::f32::consts::TAU;
            let wave = (angle + (angle.sin() * 0.55)).sin() * 0.5 + 0.5;
            let height = mounds.at(u, v) * 0.1
                + wave * 0.0042 * strength.at(u, v)
                + unevenness.at(u, v) * 0.004
                + dimples.at(u, v) * 0.0007;
            (height, 0.0)
        },
        move |u, v, _, relief| {
            // Heavy dark grains settle in the hollows, and pale ones are left on the crests.
            let hollow = smoothstep(0.001, -0.005, relief.medium);
            let speckle = smoothstep(0.32, 0.62, dark_grains.at(u, v));
            let base = mix(
                tone(0.55, 0.44, 0.29),
                tone(0.66, 0.54, 0.37),
                tint.at(u, v),
            );
            let grainy = base * (0.97 + grains.at(u, v) * 0.09);
            Shade {
                color: mix(
                    grainy,
                    tone(0.27, 0.21, 0.14),
                    hollow * 0.1 + speckle * 0.12,
                ),
                roughness: 0.92,
                cavity: 1.0 - hollow * 0.2,
            }
        },
    )
}

/// Sand just left by the tide: dark, smooth and glossy, with water in its hollows.
pub fn wet_sand(seed: u32) -> Surface {
    // Water ripple marks, about 9 cm apart and 1 cm high, sharper crested than wind ripples.
    const RIPPLES_PER_TILE: f32 = 44.0;
    // Water left behind stands at one level, so puddles fill exactly the low spots.
    const WATER_LEVEL: f32 = -0.0085;
    let wander = fbm(2, 3, seed + 210);
    let breakup = fbm(9, 3, seed + 211);
    let ripple_strength = fbm(3, 4, seed + 212).unit().smooth(0.3, 0.68);
    let lay = fbm(3, 4, seed + 213);
    // Thin channels where the last of the water drained away.
    let rills = ridged(4, 4, seed + 214).warped(fbm(14, 3, seed + 215), 0.035);
    let uneven = fbm(28, 3, seed + 216);
    let grain = fbm(300, 2, seed + 217);
    let dark_grains = fbm(420, 1, seed + 218);
    let shells = Stones {
        period: 72,
        seed: seed + 219,
        radius: 0.009,
        tile_metres: TILE_METRES,
        coverage: 0.022,
    };
    Surface::shaped(
        move |u, v| {
            let phase = v * RIPPLES_PER_TILE + wander.at(u, v) * 9.0 + breakup.at(u, v) * 1.0;
            let angle = phase * std::f32::consts::TAU;
            let wave = 1.0 - (angle.sin() * 0.5 + 0.5).powf(0.7);
            let rill = smoothstep(0.86, 0.985, rills.at(u, v));
            let ground = lay.at(u, v) * 0.02 + wave * 0.0085 * ripple_strength.at(u, v)
                - rill * 0.006
                + uneven.at(u, v) * 0.0018
                + grain.at(u, v) * 0.00025;
            if ground < WATER_LEVEL {
                return (WATER_LEVEL, -1.0 - (WATER_LEVEL - ground).min(0.05));
            }
            let shell = shells.sample(u, v);
            if shell.height > 0.0015 {
                return (ground + shell.height * 0.45, 2.0 + shell.id.min(0.99));
            }
            (ground, (ground - WATER_LEVEL).min(1.0))
        },
        move |u, v, tag, relief| {
            if tag < 0.0 {
                // Standing water: a dark glossy mirror, showing the sand through its shallows.
                let depth = -1.0 - tag;
                return Shade {
                    color: mix(
                        tone(0.095, 0.078, 0.058),
                        tone(0.03, 0.028, 0.025),
                        smoothstep(0.0, 0.012, depth),
                    ),
                    roughness: 0.05,
                    cavity: 1.0,
                };
            }
            if tag >= 2.0 {
                let id = tag - 2.0;
                return Shade {
                    color: mix(tone(0.13, 0.115, 0.095), tone(0.3, 0.27, 0.23), id),
                    roughness: 0.5,
                    cavity: 0.7,
                };
            }
            // Height above the water decides how wet the sand is: soaked by the waterline and
            // merely damp on the ripple crests, where it drains and dries first.
            let dryness = smoothstep(0.002, 0.024, tag + relief.fine * 2.0);
            let g = grain.at(u, v);
            let specks = smoothstep(0.3, 0.6, dark_grains.at(u, v)) * 0.14;
            let soaked = tone(0.115, 0.092, 0.066);
            let damp = tone(0.235, 0.19, 0.135);
            let base = mix(soaked, damp, dryness) * (0.96 + g * 0.08);
            Shade {
                color: mix(base, tone(0.05, 0.04, 0.03), specks),
                roughness: mix_f(0.26, 0.72, dryness),
                cavity: 1.0,
            }
        },
    )
}

fn mix_f(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t.clamp(0.0, 1.0)
}

/// Bare earth with clods and small stones.
pub fn dirt(seed: u32) -> Surface {
    // Dry earth is broad, soft forms covered in loose crumbs. The crumbs are real particles: creases
    // in noise follow its grid, which shows on bare ground as hatching. Colour stays in a narrow
    // range, since soil is one material, and damp changes it gradually.
    let particles = |period: i32, radius: f32, coverage: f32, offset: u32| Stones {
        period,
        seed: seed + offset,
        radius,
        tile_metres: TILE_METRES,
        coverage,
    };
    let clods = particles(44, 0.045, 0.32, 220);
    let crumbs = particles(130, 0.018, 0.7, 221);
    let grit = particles(300, 0.0085, 0.8, 222);
    let pebbles = particles(30, 0.026, 0.05, 224);
    let lay = fbm(2, 4, seed + 226);
    let forms = fbm(9, 4, seed + 223);
    let moisture = fbm(3, 5, seed + 228).unit();
    let organic = fbm(5, 5, seed + 229).unit().smooth(0.55, 0.8);
    let mottling = fbm(50, 3, seed + 235).unit();
    let fines = fbm(320, 2, seed + 234);
    Surface::shaped(
        move |u, v| {
            let soil = lay.at(u, v) * 0.05
                + forms.at(u, v) * 0.018
                + clods.sample(u, v).height * 0.55
                + crumbs.sample(u, v).height * 0.6
                + grit.sample(u, v).height * 0.6;
            let pebble = pebbles.sample(u, v);
            // Part buried, so only those that stand above the soil show.
            if pebble.height > 0.008 {
                return (soil + pebble.height * 0.5, 10.0 + pebble.id.min(0.99));
            }
            (soil, 0.0)
        },
        move |u, v, tag, relief| {
            let exposure = smoothstep(-0.01, 0.01, relief.medium);
            if tag >= 10.0 {
                let id = tag - 10.0;
                let stone = mix(tone(0.13, 0.115, 0.095), tone(0.25, 0.22, 0.18), id);
                return Shade {
                    color: mix(stone, tone(0.19, 0.145, 0.1), 0.35) * (0.8 + exposure * 0.2),
                    roughness: 0.8,
                    cavity: 0.6 + exposure * 0.4,
                };
            }
            // Tops of crumbs dry paler. Damp patches and old organic matter darken it gently.
            let dry = mix(tone(0.17, 0.122, 0.082), tone(0.235, 0.175, 0.12), exposure);
            let damp = dry * tone(0.66, 0.62, 0.6);
            let earth = mix(dry, damp, smoothstep(0.35, 0.8, moisture.at(u, v)));
            let rich = mix(earth, earth * tone(0.6, 0.56, 0.54), organic.at(u, v) * 0.7);
            Shade {
                color: rich * (0.88 + mottling.at(u, v) * 0.24) * (0.95 + fines.at(u, v) * 0.1),
                roughness: 0.96,
                cavity: 0.62 + exposure * 0.38,
            }
        },
    )
}

/// Sun-baked mud split into curling plates.
pub fn dried_mud(seed: u32) -> Surface {
    // Built from measurements of a real scan. Its cracks meet in T-junctions, 81% of them, so they
    // are grown as real ones do rather than drawn from cells. Inside a crack the colour is only
    // about a quarter darker than a plate's top: what looks black there is the lighting on its
    // walls. Each plate crowns towards its middle, so that it reads as a solid piece standing proud
    // of the cracks around it.
    let g = |spacing, width, give_up_per_metre, skip| Generation {
        spacing,
        width,
        give_up_per_metre,
        skip,
    };
    let generations = [
        g(0.26, 0.0125, 0.0, 0.0),
        g(0.16, 0.012, 0.0, 0.1),
        g(0.1, 0.0105, 0.05, 0.32),
        g(0.065, 0.007, 0.3, 0.5),
        g(0.042, 0.0024, 1.5, 0.6),
    ];
    let network = std::sync::Arc::new(CrackNetwork::grow(
        2048,
        TILE_METRES,
        seed + 230,
        &generations,
    ));
    let lay = fbm(3, 4, seed + 237);
    // Crack edges crumble, so the line they follow is broken up at small scales.
    let crumble = fbm(150, 3, seed + 241);
    let width_change = fbm(24, 3, seed + 242).unit();
    let chips = Cells {
        period: 190,
        seed: seed + 243,
    };
    // Plate tops undulate most at about 30 mm, with finer texture down to a couple of millimetres.
    let undulation = fbm(90, 3, seed + 238);
    let surface_texture = billow(340, 2, seed + 239);
    let grit = fbm(900, 1, seed + 240);
    let tint = fbm(3, 4, seed + 244).unit();
    let stains = fbm(9, 5, seed + 245).unit();
    let blotches = fbm(38, 4, seed + 247).unit();
    let dark_grains = fbm(1100, 1, seed + 246);
    Surface::shaped(
        move |u, v| {
            let (x, y) = (
                u + crumble.at(u, v) * 0.0011,
                v + crumble.at(v + 0.37, u + 0.61) * 0.0011,
            );
            let crack = network.sample(x, y);
            let half = crack.width * 0.5 * (0.55 + width_change.at(u, v) * 0.9);
            // The wall of the crack, then a crown rising towards the plate's middle.
            let inside = smoothstep(half * 1.5, half * 0.4, crack.distance);
            // Shallower than the real thing: a normal map cannot show a crack's far wall catching light,
            // so true depth reads as a band of solid shadow.
            let depth = crack.width * 0.75;
            let beyond = (crack.distance - half).max(0.0);
            let crown = smoothstep(0.0, 0.07, beyond);
            // Near a crack, small pieces have broken away, leaving the edge uneven.
            let chip = chips.sample(u, v);
            let near_edge = smoothstep(0.03, 0.006, beyond) * smoothstep(0.003, 0.012, crack.width);
            let broken = smoothstep(0.45, 0.75, chip.id) * near_edge;
            let height =
                lay.at(u, v) * 0.014 + (crack.plate - 0.5) * 0.007 + crown * crack.width * 0.9
                    - inside * depth
                    - broken * 0.0014
                    + undulation.at(u, v) * 0.0016
                    + surface_texture.at(u, v) * 0.0006
                    + grit.at(u, v) * 0.00018;
            // Packed as: which plate, then how much rim, then how far inside a crack.
            // Paleness keeps close to the crack, and comes and goes along it.
            let along_rim = smoothstep(0.042, 0.012, beyond) * smoothstep(0.0, 0.005, beyond);
            let raised = (along_rim * smoothstep(0.003, 0.011, crack.width) * 15.0).floor();
            let packed = (crack.plate * 31.0).floor() * 16.0 + raised;
            (height, packed + inside.min(0.99))
        },
        move |u, v, tag, relief| {
            let whole = tag.floor();
            let plate = (whole / 16.0).floor() / 31.0;
            let rim = (whole % 16.0) / 15.0;
            let inside = tag.fract();
            let silt = mix(
                tone(0.36, 0.215, 0.11),
                tone(0.54, 0.35, 0.185),
                tint.at(u, v) * 0.5 + plate * 0.5,
            );
            // Large stains and small blotches, darker where damp lingered longest.
            let stained = mix(
                silt,
                silt * tone(0.72, 0.68, 0.64),
                smoothstep(0.38, 0.72, stains.at(u, v)) * 0.75,
            );
            let blotched = stained * (0.8 + blotches.at(u, v) * 0.4);
            // Rims dry first and fade towards pale, dusty silt.
            let faded = mix(
                blotched,
                tone(0.66, 0.51, 0.33),
                rim * smoothstep(0.3, 0.62, blotches.at(v, u)) * 0.28,
            );
            let grained = faded
                * (0.94 + grit.at(u, v) * 0.2)
                * (1.0 - smoothstep(0.2, 0.55, dark_grains.at(u, v)) * 0.18);
            let settled = smoothstep(-0.0002, -0.0016, relief.fine);
            Shade {
                color: grained * (1.0 - settled * 0.08) * (1.0 - inside * 0.26),
                roughness: 0.97,
                cavity: (1.0 - inside * 0.3) * (1.0 - settled * 0.1),
            }
        },
    )
}

/// Loose, rounded stones in a mix of colours.
pub fn gravel(seed: u32) -> Surface {
    // Three sizes, each a separate scatter, so that small stones fill the gaps between large.
    let tile_metres = TILE_METRES;
    let large = Stones {
        period: 62,
        seed: seed + 240,
        radius: 0.055,
        tile_metres,
        coverage: 1.0,
    };
    let medium = Stones {
        period: 96,
        seed: seed + 241,
        radius: 0.035,
        tile_metres,
        coverage: 1.0,
    };
    let small = Stones {
        period: 170,
        seed: seed + 242,
        radius: 0.018,
        tile_metres,
        coverage: 1.0,
    };
    let bed = fbm(5, 4, seed + 243);
    let chips = fbm(200, 2, seed + 244);
    let mottling = fbm(70, 3, seed + 245);
    let tint = fbm(3, 3, seed + 246).unit();
    Surface::shaped(
        move |u, v| {
            let mut top = small.sample(u, v);
            for stone in [medium.sample(u, v), large.sample(u, v)] {
                if stone.height > top.height {
                    top = stone;
                }
            }
            let height = bed.at(u, v) * 0.03
                + top.height
                + chips.at(u, v) * 0.0008 * top.height.min(0.01) / 0.01;
            // Bare ground shows where no stone covers it, marked by a negative number.
            (height, if top.height > 0.0 { top.id } else { -1.0 })
        },
        move |u, v, id, relief| {
            let contact = smoothstep(-0.018, 0.0, relief.medium);
            if id < 0.0 {
                return Shade {
                    color: tone(0.05, 0.042, 0.033),
                    roughness: 0.97,
                    cavity: 0.15,
                };
            }
            // Mostly one local rock, with a few odd stones: gravel comes from one place.
            let rock = if id < 0.62 {
                mix(tone(0.27, 0.25, 0.22), tone(0.38, 0.35, 0.3), id / 0.62)
            } else if id < 0.86 {
                mix(
                    tone(0.42, 0.34, 0.25),
                    tone(0.32, 0.23, 0.16),
                    (id - 0.62) / 0.24,
                )
            } else {
                mix(
                    tone(0.14, 0.14, 0.14),
                    tone(0.5, 0.48, 0.44),
                    (id - 0.86) / 0.14,
                )
            };
            let surface = 0.88 + mottling.at(u + id * 3.0, v - id * 5.0) * 0.28;
            let dust = smoothstep(0.004, -0.012, relief.medium) * 0.35;
            let color = rock * surface * (0.9 + tint.at(u, v) * 0.2);
            Shade {
                color: mix(color, tone(0.16, 0.135, 0.105), dust) * (0.25 + contact * 0.75),
                roughness: 0.72 + id * 0.15,
                cavity: 0.2 + contact * 0.8,
            }
        },
    )
}

/// A race track's asphalt: crushed stone rolled flat in black bitumen. Close to, a fine speckle
/// of grey chips with their tops worn level; from a car, an even dark grey that is lighter
/// where traffic has polished the stone.
pub fn asphalt(seed: u32) -> Surface {
    let tile_metres = TILE_METRES;
    let chips = Stones {
        period: 300,
        seed: seed + 400,
        radius: 0.0075,
        tile_metres,
        coverage: 1.0,
    };
    let fines = Stones {
        period: 420,
        seed: seed + 401,
        radius: 0.0045,
        tile_metres,
        coverage: 1.0,
    };
    let wear = fbm(4, 4, seed + 402);
    let grain = fbm(240, 2, seed + 403);
    let tint = fbm(7, 3, seed + 404).unit();
    Surface::shaped(
        move |u, v| {
            let mut top = fines.sample(u, v);
            let chip = chips.sample(u, v);
            if chip.height > top.height {
                top = chip;
            }
            // Rolled: no stone stands more than a couple of millimetres above the binder.
            let height = top.height.min(0.0022) + grain.at(u, v) * 0.0002;
            (height, if top.height > 0.0 { top.id } else { -1.0 })
        },
        move |u, v, id, relief| {
            let worn = 0.5 + wear.at(u, v) * 0.5;
            if id < 0.0 {
                return Shade {
                    color: tone(0.022, 0.022, 0.024),
                    roughness: 0.9,
                    cavity: 0.45,
                };
            }
            let stone = mix(tone(0.07, 0.07, 0.075), tone(0.15, 0.147, 0.142), id);
            Shade {
                color: stone * (0.86 + tint.at(u, v) * 0.28) * (0.9 + worn * 0.16),
                roughness: 0.8 + id * 0.1 - worn * 0.05,
                cavity: 0.55 + smoothstep(-0.001, 0.0, relief.medium) * 0.45,
            }
        },
    )
}

/// A close-cropped lawn seen from above: many overlapping blades over dark soil.
pub fn grass(seed: u32) -> Surface {
    let lawn = Blades {
        period: 150,
        seed: seed + 250,
        density: 3,
        length: 0.07,
        width: 0.0042,
        rise: 0.013,
        flow: fbm(2, 2, seed + 251),
        spread: 0.26,
        tile_metres: TILE_METRES,
    };
    let thatch = Strokes {
        period: 90,
        seed: seed + 252,
        length: 2.2,
        width: 0.1,
        density: 2,
    };
    let bumps = fbm(4, 4, seed + 253);
    let hue = fbm(5, 5, seed + 254).unit();
    let health = fbm(3, 5, seed + 255).unit().smooth(0.22, 0.48);
    let thin = fbm(13, 4, seed + 256).unit().smooth(0.62, 0.8);
    let soil = fbm(80, 3, seed + 257);
    Surface::shaped(
        move |u, v| {
            let ground = bumps.at(u, v) * 0.02;
            let blade = lawn.sample(u, v);
            let bare = thin.at(u, v);
            // Worn patches lose their taller blades first.
            let kept = blade.height > 0.0 && blade.id > bare * 0.85;
            if kept {
                // The tag packs which blade this is with how far along it, in that order.
                let tag = (blade.id * 255.0).floor() + blade.along.min(0.99);
                return (ground + 0.012 + blade.height, tag);
            }
            let straw = thatch.sample(u, v);
            if straw.height > 0.0 {
                return (ground + straw.height * 0.012, -1.0 - straw.id);
            }
            (ground, -3.0)
        },
        move |u, v, tag, relief| {
            let shade = smoothstep(-0.012, 0.012, relief.medium);
            if tag < 0.0 {
                let color = if tag > -2.5 {
                    // Dead straw under the live blades.
                    mix(tone(0.21, 0.16, 0.08), tone(0.11, 0.08, 0.045), -tag - 1.0)
                } else {
                    tone(0.035, 0.026, 0.017) * (0.8 + soil.at(u, v) * 0.5)
                };
                return Shade {
                    color: color * (0.45 + shade * 0.55),
                    roughness: 0.95,
                    cavity: 0.3 + shade * 0.35,
                };
            }
            let id = tag.floor() / 255.0;
            let along = tag.fract();
            let h = hue.at(u, v);
            let vigour = health.at(u, v);
            let dull = mix(tone(0.08, 0.085, 0.03), tone(0.13, 0.115, 0.045), id);
            let green = mix(
                tone(0.022, 0.055, 0.012),
                tone(0.06, 0.105, 0.02),
                h * 0.6 + id * 0.4,
            );
            let body = mix(dull, green, vigour);
            // Blades are darker at the root, and tips dry out and catch the light.
            let lit = mix(body * 0.6, body * 1.1, smoothstep(0.0, 0.7, along));
            let tipped = mix(
                lit,
                tone(0.2, 0.2, 0.09),
                smoothstep(0.86, 1.0, along) * 0.3,
            );
            Shade {
                color: tipped * (0.5 + shade * 0.5),
                roughness: 0.68 + id * 0.2,
                cavity: 0.4 + shade * 0.6,
            }
        },
    )
}

/// Soft, damp moss in cushions.
pub fn moss(seed: u32) -> Surface {
    // Moss is thousands of tiny clumps, and those are what the eye reads as moss, so they are real
    // shapes with dark gaps between them. Greens drift across the patch, and each clump leans its
    // own way, so that colonies mingle at their edges with no line between them.
    let warp = fbm(20, 3, seed + 260);
    let clumps = Stones {
        period: 300,
        seed: seed + 262,
        radius: 0.0125,
        tile_metres: TILE_METRES,
        coverage: 0.95,
    };
    let small_clumps = Stones {
        period: 470,
        seed: seed + 263,
        radius: 0.007,
        tile_metres: TILE_METRES,
        coverage: 0.9,
    };
    let lay = fbm(3, 4, seed + 264);
    let hummocks = billow(11, 3, seed + 265);
    let thin = fbm(13, 4, seed + 266).unit().smooth(0.68, 0.82);
    let drift = fbm(40, 3, seed + 267).unit();
    let greens = fbm(5, 4, seed + 268)
        .unit()
        .warped(fbm(22, 3, seed + 269), 0.02);
    Surface::shaped(
        move |u, v| {
            let (x, y) = (
                u + warp.at(u, v) * 0.006,
                v + warp.at(v + 0.37, u + 0.61) * 0.006,
            );
            let large = clumps.sample(u, v);
            let small = small_clumps.sample(u, v);
            let top = if large.height >= small.height {
                large
            } else {
                small
            };
            let covered = top.height > 0.0 && top.id > thin.at(u, v) * 0.8;
            let ground = lay.at(u, v) * 0.03 + hummocks.at(x, y) * 0.032;
            let height = ground
                + if covered {
                    0.003 + top.height * 0.75
                } else {
                    0.0
                };
            (height, if covered { top.id } else { -1.0 })
        },
        move |u, v, clump, relief| {
            let lit = smoothstep(-0.006, 0.004, relief.fine);
            let crease = smoothstep(-0.003, -0.016, relief.medium);
            if clump < 0.0 {
                // Dead stems and damp soil, seen between the clumps.
                return Shade {
                    color: tone(0.026, 0.022, 0.011) * (0.7 + drift.at(u, v) * 0.6),
                    roughness: 0.95,
                    cavity: 0.25,
                };
            }
            // From yellow-green through grass green to deep olive, mixed clump by clump.
            let place = (greens.at(u, v) * 1.5 - 0.25 + (clump - 0.5) * 0.3).clamp(0.0, 1.0);
            let green = if place < 0.5 {
                mix(
                    tone(0.03, 0.07, 0.014),
                    tone(0.06, 0.115, 0.016),
                    place * 2.0,
                )
            } else {
                mix(
                    tone(0.06, 0.115, 0.016),
                    tone(0.105, 0.145, 0.018),
                    (place - 0.5) * 2.0,
                )
            };
            let varied = green * (0.84 + clump * 0.32) * (0.9 + drift.at(u, v) * 0.2);
            // New growth pales at the tips, on top of each clump.
            let tipped = mix(varied * 0.66, varied * 1.14, lit);
            Shade {
                color: mix(tipped, tone(0.014, 0.022, 0.008), crease * 0.65),
                roughness: 0.86,
                cavity: (0.4 + lit * 0.6) * (1.0 - crease * 0.55),
            }
        },
    )
}

/// Fallen leaves over dark humus.
pub fn forest_floor(seed: u32) -> Surface {
    let leaves = Leaves {
        period: 62,
        seed: seed + 270,
        density: 3,
        length: 0.095,
        depth: 0.022,
        tile_metres: TILE_METRES,
    };
    let twigs = Blades {
        period: 14,
        seed: seed + 271,
        density: 1,
        length: 0.32,
        width: 0.0065,
        rise: 0.012,
        flow: fbm(2, 2, seed + 272),
        spread: 0.5,
        tile_metres: TILE_METRES,
    };
    let lay = fbm(3, 4, seed + 273);
    let humus = billow(46, 3, seed + 274);
    let humus_grain = fbm(240, 2, seed + 275);
    let thin = fbm(7, 5, seed + 276).unit().smooth(0.52, 0.72);
    let damp = fbm(4, 5, seed + 277).unit();
    let spots = fbm(150, 3, seed + 278);
    Surface::shaped(
        move |u, v| {
            let ground =
                lay.at(u, v) * 0.035 + humus.at(u, v) * 0.008 + humus_grain.at(u, v) * 0.0005;
            let twig = twigs.sample(u, v);
            if twig.id > 0.72 && twig.height > 0.0 {
                return (ground + 0.026 + twig.height * 0.5, 30.0 + twig.id * 0.99);
            }
            let leaf = leaves.sample(u, v);
            // Bare patches keep only the heap's lowest, most rotted leaves.
            if leaf.height > 0.0 && leaf.age > thin.at(u, v) * 0.8 {
                let tag = (leaf.id * 15.0).floor() * 16.0
                    + (leaf.age * 15.0).floor()
                    + leaf.vein.min(0.99);
                return (ground + leaf.height, tag + 1000.0);
            }
            (ground, -1.0)
        },
        move |u, v, tag, relief| {
            let shelter = smoothstep(-0.012, 0.006, relief.medium);
            if tag < 0.0 {
                let soil = tone(0.026, 0.019, 0.013) * (0.75 + humus_grain.at(u, v) * 0.5);
                return Shade {
                    color: soil * (0.5 + shelter * 0.5),
                    roughness: 0.94,
                    cavity: 0.2 + shelter * 0.35,
                };
            }
            if tag < 100.0 {
                let id = tag - 30.0;
                let bark = mix(tone(0.05, 0.036, 0.024), tone(0.115, 0.085, 0.055), id);
                return Shade {
                    color: bark * (0.75 + spots.at(u, v) * 0.5) * (0.5 + shelter * 0.5),
                    roughness: 0.84,
                    cavity: 0.45 + shelter * 0.55,
                };
            }
            let packed = tag - 1000.0;
            let vein = packed.fract();
            let whole = packed.floor();
            let id = (whole / 16.0).floor() / 15.0;
            let age = (whole % 16.0) / 15.0;
            // Dry tan and rust leaves, rotting towards near black.
            let fresh = if id < 0.45 {
                mix(tone(0.14, 0.08, 0.034), tone(0.2, 0.125, 0.052), id / 0.45)
            } else if id < 0.8 {
                mix(
                    tone(0.13, 0.058, 0.023),
                    tone(0.075, 0.038, 0.018),
                    (id - 0.45) / 0.35,
                )
            } else {
                mix(
                    tone(0.17, 0.13, 0.055),
                    tone(0.22, 0.17, 0.065),
                    (id - 0.8) / 0.2,
                )
            };
            let wet = damp.at(u, v);
            let rotted = mix(
                fresh,
                tone(0.04, 0.028, 0.018),
                smoothstep(0.28, 0.9, 1.0 - age + wet * 0.5 - 0.3),
            );
            let blotched = rotted * (0.82 + spots.at(u + id, v - id) * 0.42);
            Shade {
                color: mix(blotched, blotched * 0.7, vein) * (0.42 + shelter * 0.58),
                roughness: mix_f(0.78, 0.5, smoothstep(0.45, 0.9, wet)),
                cavity: 0.3 + shelter * 0.7,
            }
        },
    )
}

/// Grey-brown cliff rock with strata and deep fractures.
pub fn rock(seed: u32) -> Surface {
    let facets = |period: i32, slope: f32, offset: u32| Facets {
        period,
        seed: seed + offset,
        slope,
        tile_metres: TILE_METRES,
    };
    // Gentle slopes on the large forms: a normal map cannot cast shadows, so anything steeper than
    // the sun's angle would light as a flat black face. Fine forms carry the visual roughness.
    let blocks = facets(5, 0.1, 280);
    let slabs = facets(14, 0.11, 281);
    let breaks = facets(38, 0.13, 282);
    let chips = facets(110, 0.13, 283);
    let coarse_warp = fbm(8, 3, seed + 284);
    let fine_warp = fbm(36, 3, seed + 294);
    let wear = fbm(9, 4, seed + 285);
    let surface = ridged(18, 5, seed + 295);
    let pitting = fbm(70, 3, seed + 296);
    let grit = fbm(240, 2, seed + 286);
    let seams = ridged(4, 4, seed + 287).warped(fbm(12, 3, seed + 288), 0.035);
    let beds = fbm(2, 3, seed + 289);
    let tint = fbm(3, 4, seed + 290).unit();
    let mottling = fbm(30, 5, seed + 291);
    let speckle = fbm(300, 1, seed + 297);
    let lichen_patches = fbm(6, 5, seed + 292).unit().smooth(0.64, 0.74);
    let lichen_grain = fbm(130, 2, seed + 293);
    Surface::shaped(
        move |u, v| {
            // Bent so that no edge between faces runs straight.
            let (x, y) = (
                u + coarse_warp.at(u, v) * 0.018 + fine_warp.at(u, v) * 0.004,
                v + coarse_warp.at(v + 0.37, u + 0.61) * 0.018
                    + fine_warp.at(v + 0.61, u + 0.37) * 0.004,
            );
            let block = blocks.sample(x, y);
            let slab = slabs.sample(x, y);
            let split = breaks.sample(x, y);
            let chip = chips.sample(x, y);
            // Weather rounds off fine detail on exposed parts, which leaves some of it smooth.
            let sheltered = 1.0 - smoothstep(-0.2, 0.4, wear.at(u, v)) * 0.7;
            let seam = smoothstep(0.94, 0.995, seams.at(u, v));
            let height = block.height
                + slab.height
                + split.height * sheltered
                + chip.height * sheltered
                + surface.at(x, y) * 0.03
                + pitting.at(u, v) * 0.0035
                + grit.at(u, v) * 0.0005
                - seam * 0.03;
            (height, block.id * 0.4 + slab.id * 0.6)
        },
        move |u, v, id, relief| {
            // Beds of slightly different stone run through the whole rock.
            let bed = ((v * 7.0 + beds.at(u, v) * 2.5).rem_euclid(1.0) * std::f32::consts::TAU)
                .sin()
                * 0.5
                + 0.5;
            let stone = mix(
                tone(0.115, 0.105, 0.095),
                tone(0.2, 0.175, 0.15),
                tint.at(u, v) * 0.55 + bed * 0.3 + id * 0.15,
            );
            // Iron staining, in patches.
            let warm = smoothstep(0.05, 0.5, mottling.at(u, v)) * 0.8;
            let grainy = stone * (0.9 + mottling.at(u, v) * 0.2) * (0.96 + speckle.at(u, v) * 0.1);
            let stained = mix(grainy, grainy * tone(1.25, 0.95, 0.68), warm);
            // Fresh edges wear pale. Hollows keep dirt and damp.
            let worn = smoothstep(0.0015, 0.008, relief.fine);
            let dirt = smoothstep(0.0, -0.022, relief.medium);
            let deep = smoothstep(-0.012, -0.05, relief.medium);
            let color = mix(stained, stained * 1.32, worn * 0.7);
            let color = mix(color, tone(0.055, 0.042, 0.03), dirt * 0.55);
            // Lichen grows on what faces up and stays dry, in thin crusty patches.
            let growth = lichen_patches.at(u, v) * smoothstep(-0.01, 0.015, relief.broad);
            let crust = smoothstep(-0.2, 0.25, lichen_grain.at(u, v));
            Shade {
                color: mix(color, tone(0.2, 0.2, 0.15), growth * crust * 0.55) * (1.0 - deep * 0.5),
                roughness: mix_f(0.9, 0.72, worn),
                cavity: (1.0 - dirt * 0.4) * (1.0 - deep * 0.6),
            }
        },
    )
}

/// Coarse speckled granite, as on a weathered boulder.
pub fn granite(seed: u32) -> Surface {
    // Granite is interlocking crystals that grew against each other as it cooled: mostly pale
    // feldspar, grey glassy quartz and a little black mica. Mica weathers out first and leaves
    // pits, and the hard feldspar stands slightly proud. Distance blends them into a speckled grey.
    let crystals = Cells {
        period: 400,
        seed: seed + 290,
    };
    // A few large feldspar crystals among the rest, as coarse granite has.
    let large = Cells {
        period: 95,
        seed: seed + 291,
    };
    let warp = fbm(140, 2, seed + 292);
    let facets = |period: i32, slope: f32, offset: u32| Facets {
        period,
        seed: seed + offset,
        slope,
        tile_metres: TILE_METRES,
    };
    let blocks = facets(6, 0.11, 293);
    let spalls = facets(19, 0.1, 294);
    let bend = fbm(9, 3, seed + 295);
    let rounding = fbm(34, 4, seed + 296);
    let joints = ridged(3, 3, seed + 297).warped(fbm(10, 3, seed + 298), 0.025);
    let feldspar_tint = fbm(5, 4, seed + 299).unit();
    let iron = fbm(3, 6, seed + 300)
        .unit()
        .warped(fbm(17, 3, seed + 305), 0.03)
        .smooth(0.5, 0.86);
    let streaks = fbm(4, 5, seed + 301).unit().smooth(0.5, 0.72);
    let lichen = fbm(11, 5, seed + 302).unit().smooth(0.7, 0.76);
    let lichen_crust = billow(150, 2, seed + 303);
    let dust = fbm(60, 3, seed + 304).unit();
    Surface::shaped(
        move |u, v| {
            let (x, y) = (
                u + warp.at(u, v) * 0.0014,
                v + warp.at(v + 0.37, u + 0.61) * 0.0014,
            );
            let (bx, by) = (
                u + bend.at(u, v) * 0.016,
                v + bend.at(v + 0.37, u + 0.61) * 0.016,
            );
            let crystal = crystals.sample(x, y);
            let big = large.sample(x, y);
            // Chosen by proportion: about a tenth mica, a third quartz, the rest feldspar. Big
            // crystals are all feldspar, and only some cells hold one.
            let is_big = big.id < 0.24 && big.border() > 0.05;
            let mineral = if is_big { 0.5 + big.id } else { crystal.id };
            let relief = if mineral < 0.1 {
                -0.0016
            } else if mineral < 0.42 {
                -0.0003
            } else {
                0.0003
            };
            let grain_edge = if is_big {
                big.border()
            } else {
                crystal.border()
            };
            let boundary = smoothstep(0.1, 0.0, grain_edge);
            let joint = smoothstep(0.965, 0.997, joints.at(u, v));
            let height = blocks.sample(bx, by).height
                + spalls.sample(bx, by).height
                + rounding.at(u, v) * 0.024
                + relief
                - boundary * 0.00035
                - joint * 0.03;
            let packed = (mineral.min(1.49) * 100.0).floor();
            (height, packed + joint.min(0.99))
        },
        move |u, v, tag, relief| {
            let mineral = tag.floor() / 100.0;
            let joint = tag.fract();
            let tint = feldspar_tint.at(u, v);
            let (crystal, roughness) = if mineral < 0.1 {
                (tone(0.028, 0.026, 0.024), 0.42)
            } else if mineral < 0.42 {
                let t = (mineral - 0.1) / 0.32;
                (
                    mix(tone(0.13, 0.13, 0.13), tone(0.23, 0.225, 0.215), t),
                    0.38,
                )
            } else {
                let t = (mineral.min(1.0) - 0.42) / 0.58;
                let pale = mix(tone(0.4, 0.385, 0.35), tone(0.53, 0.5, 0.45), t);
                let pink = mix(tone(0.4, 0.31, 0.26), tone(0.5, 0.39, 0.32), t);
                (mix(pale, pink, smoothstep(0.45, 0.75, tint) * 0.6), 0.72)
            };
            // Iron bleeds out in rusty patches, and water leaves dark streaks.
            let rusted = mix(
                crystal,
                crystal * tone(0.86, 0.62, 0.4),
                iron.at(u, v) * 0.55,
            );
            let streaked = mix(rusted, rusted * 0.58, streaks.at(u, v) * 0.6);
            // Dust gathers in hollows, softening the speckle where it lies.
            let hollow = smoothstep(0.0, -0.012, relief.medium);
            let dusted = mix(
                streaked,
                tone(0.2, 0.165, 0.125),
                hollow * 0.5 + dust.at(u, v) * 0.1,
            );
            // Crusts of pale lichen sit on what faces up and stays dry.
            let crust = lichen.at(u, v) * smoothstep(-0.012, 0.004, relief.broad);
            let patchy = smoothstep(0.08, 0.34, lichen_crust.at(u, v));
            let grown = mix(dusted, tone(0.38, 0.4, 0.3), crust * patchy * 0.85);
            let inside = smoothstep(0.1, 0.8, joint);
            Shade {
                color: grown * (1.0 - inside * 0.55),
                roughness: mix_f(roughness, 0.92, hollow.max(crust * patchy)),
                cavity: (1.0 - hollow * 0.3) * (1.0 - inside * 0.5),
            }
        },
    )
}

/// Wind-packed snow, with sparkles of ice.
pub fn snow(seed: u32) -> Surface {
    // Wind shapes snow: broad drifts, carved into long sharp-edged ridges called sastrugi that all
    // run the same way. Its surface is a crust of fused grains that glints where facets catch the
    // sun. Snow is nearly white, so its look comes almost wholly from its shape and the blue of
    // sky light in its shade, which colour here must not fight.
    let drifts = fbm(2, 4, seed + 300);
    let sastrugi = ridged(3, 4, seed + 301)
        .repeated(1, 4)
        .warped(fbm(5, 3, seed + 302), 0.05);
    let scoured = fbm(4, 4, seed + 303).unit().smooth(0.38, 0.68);
    let crust = billow(70, 3, seed + 304);
    let grain = fbm(420, 2, seed + 305);
    let glints = Cells {
        period: 520,
        seed: seed + 306,
    };
    let sparse = fbm(9, 3, seed + 307).unit();
    let aged = fbm(3, 4, seed + 308).unit();
    Surface::shaped(
        move |u, v| {
            let ridge = sastrugi.at(u, v).powi(3);
            let height = drifts.at(u, v) * 0.1
                + ridge * 0.022 * scoured.at(u, v)
                + crust.at(u, v) * 0.0022
                + grain.at(u, v) * 0.00022;
            (height, 0.0)
        },
        move |u, v, _, relief| {
            let glint = glints.sample(u, v);
            let bright = smoothstep(0.9, 0.97, glint.id)
                * smoothstep(0.42, 0.2, glint.nearest)
                * smoothstep(0.3, 0.6, sparse.at(u, v));
            // Older, wind-packed snow is a touch greyer, and hollows hold a little more shade.
            let base = mix(
                tone(0.9, 0.91, 0.92),
                tone(0.83, 0.845, 0.86),
                aged.at(u, v),
            );
            let hollow = smoothstep(0.0, -0.006, relief.medium);
            Shade {
                color: base * (1.0 - hollow * 0.03),
                // Wind-packed crust has a soft sheen, strongest at low angles to the sun and sky.
                roughness: mix_f(0.4, 0.1, bright),
                cavity: 1.0 - hollow * 0.06,
            }
        },
    )
}

/// Warm, sedimentary stone in stacked beds.
pub fn sandstone(seed: u32) -> Surface {
    // Sandstone is thick beds of sand, each with its own hardness and colour, so weather leaves
    // them as uneven ledges. Vertical joints break the beds into blocks. Faces are rough and gritty
    // in every direction. Its depth comes from broad forms and blocks, not from the beds: relief that
    // follows the beds reads as stripes, so they stay shallow, and faults stop them running far.
    // Beds run along U, so V is up the rock face.
    const BEDS: f32 = 9.0;
    let bed_wander = fbm(2, 3, seed + 10);
    let bed_edges = fbm(30, 3, seed + 11);
    let faults = Cells {
        period: 5,
        seed: seed + 28,
    };
    let broad_forms = fbm(4, 5, seed + 29);
    let bed_presence = fbm(4, 4, seed + 30).unit().smooth(0.3, 0.65);
    // A crease between beds comes and goes along its length, and varies in width.
    let crease_strength = fbm(10, 4, seed + 26).unit().smooth(0.38, 0.72);
    let crease_width = fbm(19, 3, seed + 27).unit();
    let facets = |period: i32, slope: f32, offset: u32| Facets {
        period,
        seed: seed + offset,
        slope,
        tile_metres: TILE_METRES,
    };
    let blocks = facets(7, 0.11, 12);
    let spalls = facets(23, 0.085, 13);
    let chips = facets(70, 0.06, 14);
    let bend = fbm(9, 3, seed + 15);
    let joints = ridged(4, 3, seed + 16)
        .repeated(2, 1)
        .warped(fbm(14, 3, seed + 17), 0.012);
    let erosion = billow(26, 4, seed + 18);
    let pits = fbm(60, 3, seed + 19).unit().smooth(0.62, 0.8);
    let grain = fbm(420, 2, seed + 20);
    let tint = fbm(3, 5, seed + 21).unit();
    let mottling = fbm(34, 4, seed + 22).unit();
    let varnish_streaks = fbm(5, 4, seed + 23)
        .repeated(6, 1)
        .warped(fbm(16, 2, seed + 24), 0.014)
        .unit();
    Surface::shaped(
        move |u, v| {
            let (x, y) = (
                u + bend.at(u, v) * 0.012,
                v + bend.at(v + 0.37, u + 0.61) * 0.012,
            );
            let fault = faults.sample(x, y);
            // Beds vary in thickness and wander, their edges are broken up, and each fault block
            // has slipped, so that no bed crosses the whole rock.
            let place = (v + bed_wander.at(u, v) * 0.045 + bed_edges.at(u, v) * 0.009) * BEDS
                + (v * std::f32::consts::TAU * 2.0).sin() * 0.4
                + fault.id * 3.0;
            let bed = place.floor() as i32;
            let within = place - place.floor();
            let hardness = lattice_random(bed.rem_euclid(BEDS as i32), 0, seed + 25, 0);
            // A hard bed stands out as a ledge, and the crease beneath it wears back.
            // Each bed also bulges, as weather rounds it off, so light changes across it from top to base.
            let bulge = (within * (1.0 - within) * 4.0).sqrt();
            let presence = bed_presence.at(u, v);
            let ledge = (hardness * 0.012 + bulge * 0.006) * presence;
            let width = 0.03 + crease_width.at(u, v) * 0.13;
            let crease = (smoothstep(width, 0.0, within) + smoothstep(1.0 - width, 1.0, within))
                * crease_strength.at(u, v);
            let fault_line = smoothstep(0.03, 0.006, fault.border());
            let joint = (smoothstep(0.95, 0.995, joints.at(u, v)) * crease_strength.at(v, u))
                .max(fault_line * crease_strength.at(v, u));
            let pit = pits.at(u, v);
            let height = ledge
                + blocks.sample(x, y).height
                + spalls.sample(x, y).height
                + chips.sample(x, y).height
                + erosion.at(u, v) * 0.02
                - crease.min(1.0) * 0.009 * presence
                + broad_forms.at(u, v) * 0.07
                - joint * 0.03
                - pit * 0.0025
                + grain.at(u, v) * 0.0005;
            (
                (height),
                (hardness.min(0.99) * 63.0).floor() + joint.max(crease.min(1.0) * 0.32).min(0.99),
            )
        },
        move |u, v, tag, relief| {
            let hardness = tag.floor() / 63.0;
            let dark = tag.fract();
            // Each bed differs, but only within the range of one rock: buff to rusty orange.
            let band = hardness * 0.3 + tint.at(u, v) * 0.7;
            let rock = mix(tone(0.5, 0.4, 0.27), tone(0.43, 0.27, 0.16), band);
            let mottled = rock * (0.84 + mottling.at(u, v) * 0.32) * (0.95 + grain.at(u, v) * 0.14);
            let varnish = smoothstep(0.58, 0.78, varnish_streaks.at(u, v)) * 0.42;
            let stained = mix(mottled, mottled * tone(0.56, 0.5, 0.46), varnish);
            let worn = smoothstep(0.0015, 0.009, relief.fine);
            let sheltered = smoothstep(0.002, -0.02, relief.medium);
            let recessed = smoothstep(0.0, -0.035, relief.broad);
            let inside = smoothstep(0.1, 0.7, dark);
            Shade {
                color: mix(stained, stained * 1.22, worn * 0.6)
                    * (1.0 - sheltered * 0.3)
                    * (1.0 - recessed * 0.3)
                    * (1.0 - inside * 0.4),
                roughness: 0.94,
                cavity: (1.0 - sheltered * 0.45) * (1.0 - recessed * 0.35) * (1.0 - inside * 0.45),
            }
        },
    )
}

/// Pale polished stone with flowing grey veins.
pub fn marble(seed: u32) -> Surface {
    // Marble's veins are minerals that flowed through cracks while the rock was under pressure, so
    // they share a general direction, and were folded along with it. Each is a thin dark core in a
    // soft grey cloud, and comes and goes along its length. A vein here is where a wave, bent hard
    // by noise, crosses zero, which keeps it one continuous line. Polish leaves the stone flat, so
    // its look rests on colour and reflections.
    use std::f32::consts::TAU;
    let fold = fbm(3, 5, seed + 50);
    let fine_fold = fbm(14, 4, seed + 51);
    let strength_main = fbm(4, 4, seed + 52).unit().smooth(0.34, 0.66);
    let strength_side = fbm(6, 4, seed + 53).unit().smooth(0.42, 0.68);
    let clouds = fbm(3, 6, seed + 54).unit();
    let wisps = fbm(9, 5, seed + 55).unit();
    let flecks = fbm(380, 2, seed + 56);
    let width_change = fbm(24, 3, seed + 57).unit();
    let pores = fbm(300, 2, seed + 58).unit().smooth(0.74, 0.86);
    Surface::shaped(
        move |u, v| {
            // Bent any harder, the lines close into loops like the contours of a map.
            let bent = fold.at(u, v) * 3.2 + fine_fold.at(u, v) * 0.45;
            let main = (TAU * (u * 2.0 + v * 3.0) + bent).sin().abs();
            let side = (TAU * (u * 5.0 + v * 2.0) + bent * 1.3).sin().abs();
            let fine = (TAU * (u * 7.0 + v * 12.0) + bent * 1.6).sin().abs();
            let width = 0.012 + width_change.at(u, v).powi(2) * 0.05;
            let main_strength = strength_main.at(u, v);
            let side_strength = strength_side.at(u, v);
            let core = smoothstep(width, width * 0.2, main) * main_strength
                + smoothstep(width * 0.7, width * 0.1, side) * side_strength * 0.75
                + smoothstep(width * 0.4, 0.0, fine) * side_strength * main_strength * 0.5;
            let cloud = smoothstep(0.8, 0.0, main) * main_strength
                + smoothstep(0.3, 0.0, side) * side_strength * 0.5;
            // Polish wears softer veins very slightly lower, and leaves a few tiny pores.
            let height = -core.min(1.0) * 0.00012 - pores.at(u, v) * 0.00006;
            ((height), (cloud.min(1.0) * 63.0).floor() + core.min(0.99))
        },
        move |u, v, tag, _| {
            let cloud = tag.floor() / 63.0;
            let core = tag.fract();
            let base = mix(
                tone(0.74, 0.735, 0.72),
                tone(0.85, 0.84, 0.82),
                clouds.at(u, v) * 0.6 + wisps.at(u, v) * 0.4,
            );
            let toned = mix(
                base,
                tone(0.5, 0.51, 0.52),
                cloud.powf(1.5) * (0.3 + wisps.at(u, v) * 0.7),
            );
            let flecked = toned * (0.985 + flecks.at(u, v) * 0.03);
            Shade {
                color: mix(flecked, tone(0.2, 0.205, 0.215), core * 0.85),
                roughness: mix_f(0.14, 0.24, core),
                cavity: 1.0,
            }
        },
    )
}

/// Dark volcanic rock for boulders and cliffs.
pub fn basalt(seed: u32) -> Surface {
    // Basalt is fine-grained, so it has no visible crystals. Its character comes from breaking
    // along sharp, flat faces, from the bubbles of gas frozen in it as it cooled, which collect in
    // patches, and from cracks left by shrinking. Weathering dulls it with rusty, dusty skins.
    let g = |spacing, width, give_up_per_metre, skip| Generation {
        spacing,
        width,
        give_up_per_metre,
        skip,
    };
    let network = CrackNetwork::grow(
        2048,
        TILE_METRES,
        seed + 120,
        &[
            g(0.9, 0.014, 0.0, 0.0),
            g(0.5, 0.012, 0.1, 0.25),
            g(0.28, 0.008, 0.5, 0.45),
            g(0.16, 0.0055, 1.5, 0.6),
        ],
    );
    let facets = |period: i32, slope: f32, offset: u32| Facets {
        period,
        seed: seed + offset,
        slope,
        tile_metres: TILE_METRES,
    };
    let blocks = facets(6, 0.05, 121);
    let faces = facets(17, 0.05, 122);
    let breaks = facets(52, 0.05, 123);
    let chips = facets(150, 0.05, 134);
    // Rough in every direction, so that no broken face reads as a flat, smooth plane.
    let roughness = billow(36, 4, seed + 135);
    let crumble = fbm(150, 3, seed + 136);
    let bend = fbm(7, 3, seed + 124);
    let broad_forms = fbm(4, 5, seed + 125);
    let large_bubbles = Cells {
        period: 300,
        seed: seed + 126,
    };
    let small_bubbles = Cells {
        period: 600,
        seed: seed + 127,
    };
    let bubbly = fbm(6, 4, seed + 128).unit().smooth(0.42, 0.68);
    let grit = fbm(420, 2, seed + 129);
    let tint = fbm(3, 5, seed + 130).unit();
    let mottling = fbm(30, 4, seed + 131).unit();
    let rust = fbm(5, 5, seed + 132).unit().smooth(0.55, 0.82);
    let dust = fbm(12, 4, seed + 133).unit();
    Surface::shaped(
        move |u, v| {
            let (x, y) = (
                u + bend.at(u, v) * 0.014,
                v + bend.at(v + 0.37, u + 0.61) * 0.014,
            );
            let crack = network.sample(
                x + crumble.at(u, v) * 0.0012,
                y + crumble.at(v + 0.37, u + 0.61) * 0.0012,
            );
            let half = crack.width * 0.5;
            let inside = smoothstep(half * 1.6, half * 0.4, crack.distance);
            // Only some cells hold a bubble, and more of them do in bubbly patches.
            let density = bubbly.at(u, v);
            let pit = |cell: Cell, radius: f32, share: f32| {
                if cell.id < share * density {
                    smoothstep(radius, radius * 0.8, cell.nearest)
                } else {
                    0.0
                }
            };
            let large = pit(large_bubbles.sample(u, v), 0.42, 0.3);
            let small = pit(small_bubbles.sample(u, v), 0.42, 0.4);
            let bubble = large.max(small * 0.7);
            let height = blocks.sample(x, y).height
                + faces.sample(x, y).height
                + breaks.sample(x, y).height
                + chips.sample(x, y).height
                + roughness.at(u, v) * 0.016
                + broad_forms.at(u, v) * 0.05
                - inside * crack.width * 1.4
                - large * 0.0024
                - small * 0.0012
                + grit.at(u, v) * 0.0005;
            ((height), (bubble * 15.0).floor() + inside.min(0.99))
        },
        move |u, v, tag, relief| {
            let bubble = tag.floor() / 15.0;
            let crack = tag.fract();
            let rock = mix(
                tone(0.032, 0.03, 0.029),
                tone(0.072, 0.066, 0.061),
                tint.at(u, v) * 0.6 + mottling.at(u, v) * 0.4,
            );
            let grained = rock * (0.92 + grit.at(u, v) * 0.18);
            // Iron in the rock rusts to a thin skin, and dust settles, both paling it in patches.
            let rusted = mix(grained, tone(0.105, 0.07, 0.048), rust.at(u, v) * 0.45);
            let sheltered = smoothstep(0.0, -0.012, relief.medium);
            let dusted = mix(
                rusted,
                tone(0.105, 0.09, 0.075),
                smoothstep(0.45, 0.8, dust.at(u, v)) * 0.2 + sheltered * 0.18,
            );
            let edges = smoothstep(0.001, 0.006, relief.fine);
            let hollow = bubble.max(crack);
            Shade {
                color: mix(dusted, dusted * 1.3, edges * 0.5) * (1.0 - hollow * 0.55),
                roughness: mix_f(0.74, 0.88, sheltered.max(hollow)),
                cavity: (1.0 - hollow * 0.6) * (1.0 - sheltered * 0.25),
            }
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all(seed: u32) -> Vec<(&'static str, Surface)> {
        NAMES
            .iter()
            .map(|&name| (name, by_name(name, seed).expect("listed surface exists")))
            .collect()
    }

    #[test]
    fn unknown_names_are_refused() {
        assert!(by_name("lava", 0).is_none());
    }

    #[test]
    fn no_surface_has_a_seam() {
        // Each surface's formulas must agree exactly at opposite edges of the tile. Judging seams
        // from a finished image cannot work: a real one can be fainter than the grain around it.
        const TOLERANCE: f32 = 2.0e-2;
        const SAMPLES: u32 = 400;
        for (name, surface) in all(0) {
            for i in 0..SAMPLES {
                let t = (i as f32 + 0.5) / SAMPLES as f32;
                for (a, b) in [((0.0, t), (1.0, t)), ((t, 0.0), (t, 1.0))] {
                    let (a, b) = (surface.formula_at(a.0, a.1), surface.formula_at(b.0, b.1));
                    assert!(
                        (a.height - b.height).abs() < TOLERANCE,
                        "{name} height at {t}"
                    );
                    assert!(
                        a.color.distance(b.color) < TOLERANCE,
                        "{name} colour at {t}"
                    );
                    assert!(
                        (a.roughness - b.roughness).abs() < TOLERANCE,
                        "{name} roughness at {t}"
                    );
                }
            }
        }
    }

    #[test]
    fn colours_and_properties_are_physically_plausible() {
        for (name, surface) in all(0) {
            let layer = surface.bake(64);
            let colors = &layer.color_mips[0];
            let linear = |byte: u8| (f32::from(byte) / 255.0).powf(2.2);
            let count = colors.len() / 4;
            let mean: f32 = colors
                .chunks_exact(4)
                .map(|c| linear(c[0]).max(linear(c[1])).max(linear(c[2])))
                .sum::<f32>()
                / count as f32;
            // Charcoal is about 0.02 and fresh snow about 0.9.
            assert!(mean > 0.02, "{name} is implausibly dark: {mean}");
            assert!(mean < 0.92, "{name} is implausibly bright: {mean}");
        }
    }

    #[test]
    fn most_of_each_surface_is_not_too_steep_to_light_well() {
        // Normals tipped nearly sideways turn black in sunlight, whatever the colour says. Edges
        // such as cracks and blades are meant to be steep, so this looks at the typical slope.
        const MAX_TYPICAL_SLOPE: f32 = 1.0;
        let mut too_steep = Vec::new();
        for name in NAMES {
            let layer = by_name(name, 0).unwrap().bake(512);
            let mut slopes: Vec<f32> = layer.normal_mips[0]
                .chunks_exact(2)
                .map(|n| {
                    let (x, y) = (n[0] as f32 / 127.5 - 1.0, n[1] as f32 / 127.5 - 1.0);
                    (x * x + y * y).sqrt() / (1.0 - x * x - y * y).max(1.0e-4).sqrt()
                })
                .collect();
            slopes.sort_by(f32::total_cmp);
            let typical = slopes[slopes.len() / 2];
            println!("{name}: typical slope {typical:.2}");
            if typical >= MAX_TYPICAL_SLOPE {
                too_steep.push(format!("{name}: {typical:.2}"));
            }
        }
        assert!(too_steep.is_empty(), "typical slopes: {too_steep:?}");
    }

    #[test]
    fn the_seed_alone_decides_the_result() {
        for name in NAMES {
            let bake = |seed| by_name(name, seed).unwrap().bake(32).color_mips[0].clone();
            assert_eq!(bake(4), bake(4), "{name} must be repeatable");
            assert_ne!(bake(4), bake(5), "{name} must vary with its seed");
        }
    }
}
