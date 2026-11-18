//! The ground of the Australian bush: what a track through dry eucalypt forest is made of, and
//! what lies either side of it.
//!
//! * [`red_clay`]: a bush track's surface. Hard orange-red clay with ironstone gravel pressed
//!   into it, washed fines in its hollows, and a crazing of fine cracks where it has baked.
//! * [`bush_mud`]: the same clay, wet and churned by wheels: dark, slick, in clods and smears,
//!   with water standing in its lowest hollows.
//! * [`gum_litter`]: the floor of a eucalypt forest. Gum trees shed all year: long sickle
//!   leaves that dry tan, grey and rust, ribbons of bark, twigs and gumnuts, over pale, dry,
//!   stony soil. Nothing rots as it does in a damp oak wood, so the litter is pale, not black.
//! * [`tussock`]: the ground of a grassy clearing in summer: straw-pale dead grass lying in
//!   swirls over the same soil, bare in patches.
//!
//! Each is built as the surfaces of [`crate::surfaces`] are: its real shape first, in metres,
//! and its colour from that shape.

use crate::field::{billow, fbm, smoothstep, Blades, Cells, Field, Leaves, Stones};
use crate::surface::{Shade, Surface, TILE_METRES};
use glam::Vec3;

fn mix(a: Vec3, b: Vec3, t: f32) -> Vec3 {
    a + (b - a) * t.clamp(0.0, 1.0)
}

fn mix_f(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t.clamp(0.0, 1.0)
}

/// A linear albedo.
fn tone(r: f32, g: f32, b: f32) -> Vec3 {
    Vec3::new(r, g, b)
}

/// The surfaces of this module, by name.
pub fn by_name(name: &str, seed: u32) -> Option<Surface> {
    Some(match name {
        "red_clay" => red_clay(seed),
        "bush_mud" => bush_mud(seed),
        "gum_litter" => gum_litter(seed),
        "tussock" => tussock(seed),
        _ => return None,
    })
}

/// The names [`by_name`] knows.
pub const NAMES: [&str; 4] = ["red_clay", "bush_mud", "gum_litter", "tussock"];

/// A bush track's hard clay: orange-red, gravelly, dusty, cracked where it has baked.
pub fn red_clay(seed: u32) -> Surface {
    let stones = |period: i32, radius: f32, coverage: f32, offset: u32| Stones {
        period,
        seed: seed + offset,
        radius,
        tile_metres: TILE_METRES,
        coverage,
    };
    // Ironstone gravel, the "buckshot" of lateritic clay: dark red-brown, rounded, mostly
    // pressed in level with the surface by the wheels. And a few larger stones standing proud.
    let gravel = stones(110, 0.014, 0.42, 300);
    let grit = stones(260, 0.0065, 0.7, 301);
    let cobbles = stones(22, 0.045, 0.1, 302);
    let lay = fbm(2, 4, seed + 303);
    let forms = fbm(11, 4, seed + 304);
    // Fines washed into the hollows by the last rain, smooth and paler; dust lying on the rest.
    let washed = fbm(7, 4, seed + 305).unit().smooth(0.5, 0.68);
    let dust = fbm(160, 2, seed + 306);
    let crazing = Cells {
        period: 46,
        seed: seed + 307,
    };
    let baked = fbm(4, 4, seed + 308).unit().smooth(0.42, 0.7);
    let hue = fbm(3, 5, seed + 309).unit();
    let stain = fbm(9, 4, seed + 310).unit();
    let mottle = fbm(60, 3, seed + 311).unit();
    Surface::shaped(
        move |u, v| {
            let smooth = washed.at(u, v);
            // The clay itself: packed, with shallow forms a wheel's width across.
            let clay = lay.at(u, v) * 0.03 + forms.at(u, v) * 0.012;
            // Fine cracks, a few millimetres deep, where it has dried hard.
            let crack = crazing.sample(u, v).border();
            let cracked = (1.0 - smoothstep(0.0, 0.05, crack)) * baked.at(u, v) * (1.0 - smooth);
            let ground = clay - cracked * 0.004 + dust.at(u, v) * 0.0006 * (1.0 - smooth);
            let cobble = cobbles.sample(u, v);
            if cobble.height > 0.022 {
                return (
                    ground + (cobble.height - 0.022) * 0.9,
                    20.0 + cobble.id.min(0.99),
                );
            }
            // Gravel is worn in almost flush, and silted over in the washed hollows.
            let pebble = gravel.sample(u, v);
            if pebble.height > 0.004 && smooth < 0.75 {
                return (ground + pebble.height * 0.45, 10.0 + pebble.id.min(0.99));
            }
            let speck = grit.sample(u, v);
            if speck.height > 0.002 && smooth < 0.5 {
                return (ground + speck.height * 0.5, 10.0 + speck.id.min(0.99));
            }
            (
                ground,
                cracked.min(0.99) + if smooth > 0.5 { 2.0 } else { 0.0 },
            )
        },
        move |u, v, tag, relief| {
            let exposure = smoothstep(-0.006, 0.006, relief.medium);
            if tag >= 20.0 {
                // A stone: grey-brown quartzite or rusty ironstone, dusted with the clay.
                let id = tag - 20.0;
                let stone = if id < 0.6 {
                    mix(tone(0.2, 0.16, 0.13), tone(0.34, 0.29, 0.24), id / 0.6)
                } else {
                    mix(
                        tone(0.17, 0.07, 0.04),
                        tone(0.26, 0.11, 0.05),
                        (id - 0.6) / 0.4,
                    )
                };
                return Shade {
                    color: mix(stone, tone(0.36, 0.17, 0.07), 0.3 * (1.0 - exposure))
                        * (0.75 + exposure * 0.25),
                    roughness: 0.82,
                    cavity: 0.55 + exposure * 0.45,
                };
            }
            if tag >= 10.0 {
                // Ironstone gravel: dark red-brown to almost black, with the odd pale quartz.
                let id = tag - 10.0;
                let pebble = if id > 0.9 {
                    tone(0.5, 0.45, 0.38)
                } else {
                    mix(tone(0.075, 0.035, 0.022), tone(0.2, 0.085, 0.04), id / 0.9)
                };
                return Shade {
                    color: mix(pebble, tone(0.36, 0.17, 0.07), 0.35) * (0.8 + exposure * 0.2),
                    roughness: 0.78,
                    cavity: 0.6 + exposure * 0.4,
                };
            }
            let washed_here = tag >= 2.0;
            let cracked = tag.fract();
            // Orange-red clay, yellower in some stretches and redder in others, darker where
            // the damp lingers and paler on the dusty high points.
            let clay = mix(
                tone(0.33, 0.125, 0.045),
                tone(0.42, 0.2, 0.075),
                hue.at(u, v),
            );
            let stained = mix(
                clay,
                clay * tone(0.72, 0.66, 0.62),
                smoothstep(0.5, 0.85, stain.at(u, v)),
            );
            let dusty = mix(stained * 0.86, stained * tone(1.12, 1.1, 1.08), exposure);
            let mut color = dusty * (0.9 + mottle.at(u, v) * 0.2);
            if washed_here {
                // Settled fines: pale, even, very fine.
                color = mix(color, tone(0.47, 0.27, 0.13), 0.55);
            }
            color *= 1.0 - cracked * 0.35;
            Shade {
                color,
                roughness: if washed_here { 0.86 } else { 0.95 },
                cavity: (0.7 + exposure * 0.3) * (1.0 - cracked * 0.4),
            }
        },
    )
}

/// The track's clay wet and churned: dark, slick clods and smears, with water in the hollows.
pub fn bush_mud(seed: u32) -> Surface {
    // Clods thrown up by tyres, large and small, on a churned bed; smears where a tyre slid.
    let clods = |period: i32, radius: f32, coverage: f32, offset: u32| Stones {
        period,
        seed: seed + offset,
        radius,
        tile_metres: TILE_METRES,
        coverage,
    };
    let lumps = clods(26, 0.06, 0.38, 320);
    let crumbs = clods(70, 0.022, 0.5, 321);
    let lay = fbm(3, 4, seed + 322);
    let churn = billow(14, 4, seed + 323);
    let ripple = fbm(45, 3, seed + 324);
    let fine = fbm(220, 2, seed + 325);
    // The lowest of it holds water.
    let hue = fbm(4, 4, seed + 326).unit();
    let drying = fbm(6, 4, seed + 327).unit().smooth(0.55, 0.8);
    let stones = clods(34, 0.02, 0.06, 328);
    Surface::shaped(
        move |u, v| {
            let bed = lay.at(u, v) * 0.045 + churn.at(u, v) * 0.03 + ripple.at(u, v) * 0.006;
            let lump = lumps.sample(u, v);
            let crumb = crumbs.sample(u, v);
            let clod = (lump.height * 0.5).max(crumb.height * 0.55);
            let height = bed + clod + fine.at(u, v) * 0.0008;
            let stone = stones.sample(u, v);
            if stone.height > 0.01 {
                return (height + stone.height * 0.4, 10.0 + stone.id.min(0.99));
            }
            (height, if clod > 0.004 { 1.0 } else { 0.0 })
        },
        move |u, v, tag, relief| {
            // Water lies where the mud is lowest over a hand's breadth and more.
            let pooled =
                smoothstep(-0.012, -0.03, relief.broad) * smoothstep(0.004, -0.004, relief.medium);
            let high = smoothstep(-0.01, 0.014, relief.medium);
            if tag >= 10.0 {
                let id = tag - 10.0;
                let stone = mix(tone(0.07, 0.04, 0.03), tone(0.16, 0.1, 0.07), id);
                return Shade {
                    color: stone * (0.7 + high * 0.3),
                    roughness: 0.4,
                    cavity: 0.6 + high * 0.4,
                };
            }
            // Wet clay: a deep red-brown, a little paler and duller where its tops are drying.
            let wet = mix(
                tone(0.075, 0.032, 0.015),
                tone(0.13, 0.058, 0.026),
                hue.at(u, v),
            );
            let dried = drying.at(u, v) * high;
            let mud = mix(wet, tone(0.25, 0.12, 0.055), dried * 0.7);
            let shaded = mud * (0.62 + high * 0.38);
            // Standing water: brown, and as smooth as water is.
            let water = tone(0.085, 0.05, 0.028);
            Shade {
                color: mix(shaded, water, pooled),
                roughness: mix_f(mix_f(0.3, 0.62, dried), 0.05, pooled),
                cavity: mix_f(0.45 + high * 0.55, 1.0, pooled),
            }
        },
    )
}

/// The floor of a eucalypt forest: gum leaves, ribbons of bark, twigs and gumnuts on dry soil.
pub fn gum_litter(seed: u32) -> Surface {
    // Gum leaves: long, a hand's length, lying flat and dry.
    let leaves = Leaves {
        period: 44,
        seed: seed + 340,
        density: 3,
        length: 0.13,
        depth: 0.014,
        tile_metres: TILE_METRES,
    };
    // Ribbons of shed bark, as long as a forearm and a thumb wide, curling as they dry.
    let ribbons = Blades {
        period: 9,
        seed: seed + 341,
        density: 1,
        length: 0.55,
        width: 0.03,
        rise: 0.014,
        flow: fbm(2, 2, seed + 342),
        spread: 0.5,
        tile_metres: TILE_METRES,
    };
    let twigs = Blades {
        period: 12,
        seed: seed + 343,
        density: 1,
        length: 0.34,
        width: 0.006,
        rise: 0.012,
        flow: fbm(3, 2, seed + 344),
        spread: 0.5,
        tile_metres: TILE_METRES,
    };
    // Gumnuts and small stones.
    let nuts = Stones {
        period: 60,
        seed: seed + 345,
        radius: 0.009,
        tile_metres: TILE_METRES,
        coverage: 0.16,
    };
    let lay = fbm(3, 4, seed + 346);
    let crumbs = billow(52, 3, seed + 347);
    let grain = fbm(260, 2, seed + 348);
    // Where the litter lies thin, the soil shows: on rises, and where the wind has swept it.
    let thin = fbm(6, 5, seed + 349).unit().smooth(0.45, 0.7);
    let fade = fbm(5, 4, seed + 350).unit();
    let spots = fbm(140, 3, seed + 351);
    let soil_hue = fbm(4, 4, seed + 352).unit();
    Surface::shaped(
        move |u, v| {
            let ground = lay.at(u, v) * 0.03 + crumbs.at(u, v) * 0.006 + grain.at(u, v) * 0.0005;
            let twig = twigs.sample(u, v);
            if twig.id > 0.55 && twig.height > 0.0 {
                return (ground + 0.018 + twig.height * 0.5, 30.0 + twig.id * 0.99);
            }
            let ribbon = ribbons.sample(u, v);
            if ribbon.id > 0.45 && ribbon.height > 0.0 {
                // Curled across its width: its edges stand a little higher than its middle.
                return (
                    ground + 0.012 + ribbon.height * 0.7,
                    50.0 + (ribbon.id * 15.0).floor() + ribbon.along.min(0.99),
                );
            }
            let bare = thin.at(u, v);
            let leaf = leaves.sample(u, v);
            if leaf.height > 0.0 && leaf.age > bare * 0.9 {
                let tag = (leaf.id * 15.0).floor() * 16.0
                    + (leaf.age * 15.0).floor()
                    + leaf.vein.min(0.99);
                return (ground + leaf.height, tag + 1000.0);
            }
            let nut = nuts.sample(u, v);
            if nut.height > 0.003 {
                return (ground + nut.height * 0.8, 20.0 + nut.id.min(0.99));
            }
            (ground, -1.0)
        },
        move |u, v, tag, relief| {
            let shelter = smoothstep(-0.01, 0.006, relief.medium);
            if tag < 0.0 {
                // Dry soil: pale grey-brown, a little orange with the clay under it.
                let soil = mix(
                    tone(0.15, 0.105, 0.07),
                    tone(0.24, 0.15, 0.085),
                    soil_hue.at(u, v),
                ) * (0.85 + grain.at(u, v) * 0.3);
                return Shade {
                    color: soil * (0.6 + shelter * 0.4),
                    roughness: 0.96,
                    cavity: 0.4 + shelter * 0.45,
                };
            }
            if tag < 30.0 {
                // A gumnut, or a small stone.
                let id = tag - 20.0;
                let nut = mix(tone(0.07, 0.05, 0.035), tone(0.2, 0.15, 0.1), id);
                return Shade {
                    color: nut * (0.7 + shelter * 0.3),
                    roughness: 0.8,
                    cavity: 0.5 + shelter * 0.5,
                };
            }
            if tag < 50.0 {
                // A twig: grey where it has weathered, red-brown where it is newly fallen.
                let id = tag - 30.0;
                let twig = mix(tone(0.2, 0.17, 0.14), tone(0.12, 0.06, 0.035), id);
                return Shade {
                    color: twig * (0.8 + spots.at(u, v) * 0.4) * (0.6 + shelter * 0.4),
                    roughness: 0.85,
                    cavity: 0.5 + shelter * 0.5,
                };
            }
            if tag < 1000.0 {
                // A ribbon of bark: cream or pinkish inside when new, grey as it weathers,
                // with long streaks of rust.
                let packed = tag - 50.0;
                let (id, along) = (packed.floor() / 15.0, packed.fract());
                let new = mix(
                    tone(0.42, 0.3, 0.2),
                    tone(0.3, 0.16, 0.09),
                    smoothstep(0.3, 0.9, id),
                );
                let old = tone(0.27, 0.24, 0.2);
                let bark = mix(new, old, smoothstep(0.35, 0.75, fade.at(u + id, v)));
                let streak = 0.85 + 0.3 * spots.at(u * 0.3 + id, v * 3.0);
                return Shade {
                    color: bark * streak * (0.7 + shelter * 0.3) * (0.9 + along * 0.15),
                    roughness: 0.8,
                    cavity: 0.55 + shelter * 0.45,
                };
            }
            let packed = tag - 1000.0;
            let vein = packed.fract();
            let whole = packed.floor();
            let id = (whole / 16.0).floor() / 15.0;
            let age = (whole % 16.0) / 15.0;
            // Dry gum leaves: tan and straw, pinkish rust, bleached grey, and a few still olive.
            let fresh = if id < 0.4 {
                mix(tone(0.27, 0.18, 0.09), tone(0.36, 0.26, 0.13), id / 0.4)
            } else if id < 0.68 {
                mix(
                    tone(0.24, 0.1, 0.05),
                    tone(0.33, 0.15, 0.075),
                    (id - 0.4) / 0.28,
                )
            } else if id < 0.9 {
                mix(
                    tone(0.2, 0.18, 0.15),
                    tone(0.31, 0.28, 0.23),
                    (id - 0.68) / 0.22,
                )
            } else {
                mix(
                    tone(0.1, 0.12, 0.055),
                    tone(0.17, 0.18, 0.085),
                    (id - 0.9) / 0.1,
                )
            };
            // The oldest have gone to grey-brown and begun to break up.
            let weathered = mix(
                fresh,
                tone(0.17, 0.135, 0.1),
                smoothstep(0.55, 0.0, age) * 0.7,
            );
            let blotched = weathered * (0.86 + spots.at(u + id, v - id) * 0.32);
            Shade {
                color: mix(blotched, blotched * 0.78, vein) * (0.58 + shelter * 0.42),
                roughness: 0.74,
                cavity: 0.45 + shelter * 0.55,
            }
        },
    )
}

/// A grassy clearing's ground in summer: straw lying in swirls over dry soil.
pub fn tussock(seed: u32) -> Surface {
    // Long dead blades, lying over as wind and animals have left them.
    let straw = Blades {
        period: 64,
        seed: seed + 360,
        density: 4,
        length: 0.24,
        width: 0.0048,
        rise: 0.02,
        flow: fbm(2, 3, seed + 361),
        spread: 0.3,
        tile_metres: TILE_METRES,
    };
    // Shorter, finer thatch under them.
    let thatch = Blades {
        period: 150,
        seed: seed + 362,
        density: 2,
        length: 0.08,
        width: 0.003,
        rise: 0.006,
        flow: fbm(5, 2, seed + 363),
        spread: 0.5,
        tile_metres: TILE_METRES,
    };
    let lay = fbm(3, 4, seed + 364);
    let crumbs = billow(60, 3, seed + 365);
    // Bare patches, where the grass has died back to the soil.
    let bare = fbm(8, 5, seed + 366).unit().smooth(0.56, 0.74);
    let green = fbm(4, 4, seed + 367).unit().smooth(0.6, 0.82);
    let hue = fbm(6, 4, seed + 368).unit();
    let grain = fbm(240, 2, seed + 369);
    let pebbles = Stones {
        period: 48,
        seed: seed + 370,
        radius: 0.012,
        tile_metres: TILE_METRES,
        coverage: 0.12,
    };
    Surface::shaped(
        move |u, v| {
            let ground = lay.at(u, v) * 0.03 + crumbs.at(u, v) * 0.005;
            let open = bare.at(u, v);
            let blade = straw.sample(u, v);
            if blade.height > 0.0 && blade.id > open * 0.95 {
                let tag = (blade.id * 255.0).floor() + blade.along.min(0.99);
                return (ground + 0.01 + blade.height, 1000.0 + tag);
            }
            let under = thatch.sample(u, v);
            if under.height > 0.0 && under.id > open * 0.9 {
                return (ground + 0.003 + under.height, 500.0 + under.id.min(0.99));
            }
            let pebble = pebbles.sample(u, v);
            if pebble.height > 0.004 {
                return (ground + pebble.height * 0.6, 10.0 + pebble.id.min(0.99));
            }
            (ground + grain.at(u, v) * 0.0006, -1.0)
        },
        move |u, v, tag, relief| {
            let shade = smoothstep(-0.012, 0.012, relief.medium);
            if tag < 0.0 {
                let soil = mix(
                    tone(0.17, 0.11, 0.065),
                    tone(0.27, 0.15, 0.075),
                    hue.at(u, v),
                ) * (0.85 + grain.at(u, v) * 0.3);
                return Shade {
                    color: soil * (0.6 + shade * 0.4),
                    roughness: 0.96,
                    cavity: 0.4 + shade * 0.4,
                };
            }
            if tag < 500.0 {
                let id = tag - 10.0;
                let stone = mix(tone(0.11, 0.06, 0.04), tone(0.3, 0.24, 0.19), id);
                return Shade {
                    color: stone * (0.75 + shade * 0.25),
                    roughness: 0.82,
                    cavity: 0.6 + shade * 0.4,
                };
            }
            if tag < 1000.0 {
                // Thatch: old, grey straw.
                let id = tag - 500.0;
                let old = mix(tone(0.2, 0.165, 0.11), tone(0.3, 0.25, 0.16), id);
                return Shade {
                    color: old * (0.5 + shade * 0.5),
                    roughness: 0.9,
                    cavity: 0.4 + shade * 0.5,
                };
            }
            let packed = tag - 1000.0;
            let (id, along) = (packed.floor() / 255.0, packed.fract());
            // Straw: pale gold to bleached, a few blades still green at the root in the
            // damper patches.
            let dead = mix(tone(0.36, 0.28, 0.13), tone(0.52, 0.44, 0.25), id);
            let alive = mix(tone(0.1, 0.14, 0.04), tone(0.2, 0.22, 0.07), id);
            let living = green.at(u, v) * smoothstep(0.55, 0.1, along) * 0.8;
            let body = mix(dead, alive, living);
            let lit = mix(body * 0.7, body * 1.08, smoothstep(0.0, 0.6, along));
            Shade {
                color: lit * (0.55 + shade * 0.45),
                roughness: 0.72 + id * 0.15,
                cavity: 0.45 + shade * 0.55,
            }
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A surface's mean albedo, linear, from its smallest mip but one.
    fn mean(name: &str) -> Vec3 {
        let layer = by_name(name, 0).unwrap().bake(128);
        let mip = &layer.color_mips[layer.color_mips.len() - 1];
        let linear = |c: u8| (c as f32 / 255.0).powf(2.2);
        Vec3::new(linear(mip[0]), linear(mip[1]), linear(mip[2]))
    }

    #[test]
    fn every_name_is_a_surface() {
        for name in NAMES {
            assert!(by_name(name, 3).is_some(), "{name}");
        }
        assert!(by_name("bitumen", 0).is_none());
    }

    #[test]
    fn the_clay_is_red_and_the_mud_is_dark() {
        let clay = mean("red_clay");
        assert!(clay.x > clay.y * 1.6 && clay.y > clay.z * 1.5, "{clay}");
        assert!((0.2..0.45).contains(&clay.x), "{clay}");
        let mud = mean("bush_mud");
        assert!(mud.x < clay.x * 0.5, "mud {mud}, clay {clay}");
        assert!(mud.x > mud.z, "{mud}");
    }

    #[test]
    fn dry_litter_and_straw_are_pale_not_green() {
        for name in ["gum_litter", "tussock"] {
            let color = mean(name);
            assert!(color.x > color.y && color.y > color.z, "{name}: {color}");
            assert!((0.12..0.45).contains(&color.x), "{name}: {color}");
        }
        // Straw is paler than leaf litter.
        assert!(mean("tussock").y > mean("gum_litter").y);
    }
}
