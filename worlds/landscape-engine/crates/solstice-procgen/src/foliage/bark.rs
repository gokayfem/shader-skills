//! Bark, as surfaces of the procedural library.
//!
//! Each bark is listed in [`surfaces`](crate::surfaces) by name, so a scene can ask for
//! `birch_bark 2` like any other material. Bark wraps around stems, so every one repeats a whole
//! number of times across its tile: [`AROUND`] times, every [`BARK_REPEAT`] metres. Features
//! across a bark are built at that repeat, which keeps its seam invisible on every stem.
//!
//! Each is modelled on photographs of the real bark, by its structure first: what the ridges,
//! plates and markings are, how big, and which way they run. Colour then follows that shape.

use crate::field::{fbm, fbm_xy, ridged, smoothstep, Cell, Cells, Field};
use crate::surface::{Relief, Shade, Surface};
use glam::Vec3;

/// Metres one bark texture covers. Bark is seen close up and its detail is millimetres, so its
/// tile is a quarter of other surfaces': a 1024 texture holds a texel per millimetre.
pub const BARK_TILE_METRES: f32 = 1.0;
/// How many times bark repeats across its tile.
pub const AROUND: u32 = 4;
/// Bark repeats around a stem in whole multiples of this, in metres, so its seam never shows.
pub const BARK_REPEAT: f32 = BARK_TILE_METRES / AROUND as f32;

/// The kinds of bark.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bark {
    /// Pedunculate oak: long ridges that split and rejoin around deep vertical fissures, grey on
    /// the ridges and rusty brown in the fissures.
    Oak,
    /// Silver birch: chalky white paper with dark lenticels running around the stem, black
    /// fissured patches, and strips peeling away.
    Birch,
    /// Silver fir: smooth silver-grey, mottled, with resin blisters and faint small plates.
    Fir,
    /// Scots pine: thick red-brown plates split by dark furrows, flaking orange where the outer
    /// layers fall away.
    Pine,
    /// Beech: smooth grey with fine wavy wrinkles running around the stem, and lichen.
    Beech,
    /// Weeping willow: grey-brown and deeply furrowed like an oak's, its corky ridges paler and
    /// yellower, with tan inner bark showing in the furrows and moss where it stays damp.
    Willow,
    /// A smooth-barked gum, such as a manna gum: bark shed each year in long plates, leaving
    /// a patchwork of cream, grey, salmon and olive, streaked down the stem, with the dark
    /// edges of plates not yet let go.
    Gum,
    /// A stringybark, such as messmate: thick, soft and fibrous, in long strings that run up
    /// the stem between deep furrows, grey-brown outside and red-brown within.
    Stringybark,
    /// An ironbark: hard, deeply furrowed and nearly black, never shed, with dark red kino
    /// in its furrows. A tree that has been through a fire looks much the same.
    Ironbark,
}

impl Bark {
    pub const ALL: [Self; 9] = [
        Self::Oak,
        Self::Birch,
        Self::Fir,
        Self::Pine,
        Self::Beech,
        Self::Willow,
        Self::Gum,
        Self::Stringybark,
        Self::Ironbark,
    ];

    /// The name of this bark in the surface library.
    pub fn name(self) -> &'static str {
        match self {
            Self::Oak => "oak_bark",
            Self::Birch => "birch_bark",
            Self::Fir => "fir_bark",
            Self::Pine => "pine_bark",
            Self::Beech => "beech_bark",
            Self::Willow => "willow_bark",
            Self::Gum => "gum_bark",
            Self::Stringybark => "stringybark",
            Self::Ironbark => "ironbark",
        }
    }

    pub fn by_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|bark| bark.name() == name)
    }

    /// The bark's surface, covering [`BARK_TILE_METRES`].
    pub fn surface(self, seed: u32) -> Surface {
        match self {
            Self::Oak => oak(seed),
            Self::Birch => birch(seed),
            Self::Fir => fir(seed),
            Self::Pine => pine(seed),
            Self::Beech => beech(seed),
            Self::Willow => willow(seed),
            Self::Gum => gum(seed),
            Self::Stringybark => stringybark(seed),
            Self::Ironbark => ironbark(seed),
        }
        .with_tile_metres(BARK_TILE_METRES)
    }
}

/// Bark noise: `across` features in each repeat around the stem and `down` per metre along it,
/// in roughly `-1..1`. Unlike repeating a square of noise, this stretches features without
/// copying them, so nothing repeats along the stem.
fn noise(across: i32, down: i32, octaves: u32, seed: u32) -> impl Field + Copy {
    let field = fbm_xy(
        across,
        (down as f32 * BARK_TILE_METRES) as i32,
        octaves,
        seed,
    );
    move |u: f32, v: f32| field.at(u * AROUND as f32, v)
}

/// A field repeated [`AROUND`] times across the tile and `down` times along it.
fn wrapped(field: impl Field, down: u32) -> impl Field {
    field.repeated(AROUND, down)
}

/// A field that varies only along the stem. Anything that varies around it repeats every
/// [`BARK_REPEAT`], so broad features such as patches and mottling would line up in columns.
fn along(field: impl Field, down: u32) -> impl Field {
    move |_: f32, v: f32| field.at(0.37, v * down as f32)
}

/// Cells `across` times as many per repeat around the stem and `down` times along the tile,
/// each cell `period` to a side of its square. Wide `across` and small `down` give cells long in
/// the direction of the stem.
fn cells_at(cells: Cells, across: u32, down: u32, u: f32, v: f32) -> Cell {
    cells.sample(u * (AROUND * across) as f32, v * down as f32)
}

/// Oak at a point: (fissure 0..1, lichen crust 0..1, moss 0..1, relief of the plates in metres).
fn oak_shape(seed: u32) -> impl Fn(f32, f32) -> (f32, f32, f32, f32) + Send + Sync {
    // Matched to a scan of old oak bark (Poly Haven, jolcham_oak_bark_01). Fissures are the
    // zero lines of noise about three times longer than wide, 4 to 6 cm apart, broken and
    // jagged by its finer octaves, and fading out in places so they do not run on for ever.
    // Finer cracks run between them.
    let lines = noise(5, 6, 5, seed);
    let cracks = noise(10, 5, 4, seed + 11);
    let warp = noise(2, 4, 3, seed + 1);
    let presence = noise(2, 3, 3, seed + 2);
    // The plates between them crumble at three scales: 2 cm, 5 mm and 1.5 mm.
    let lumps = noise(12, 50, 3, seed + 3);
    let crumbs = noise(48, 200, 3, seed + 4);
    let grit = noise(160, 660, 2, seed + 5);
    // Crusts of pale lichen in ragged, grainy patches; moss in streaks along the fissures.
    let crust = noise(6, 24, 5, seed + 6);
    let moss = noise(3, 4, 4, seed + 7);
    move |u, v| {
        let w = warp.at(u, v) * 0.008;
        let (wu, wv) = (u + w, v + w * 2.0);
        // Fissures open out gradually, so their walls slope as a scan's do.
        let line = 1.0 - (lines.at(wu, wv).abs() / 0.16).min(1.0);
        let crack = 1.0 - (cracks.at(wu, wv).abs() / 0.07).min(1.0);
        let fissure =
            (line * line * smoothstep(-0.3, 0.05, presence.at(u, v))).max(crack * crack * 0.5);
        let crumb = crumbs.at(u, v);
        let relief = 0.0022 * lumps.at(u, v) + 0.0008 * crumb + 0.0003 * grit.at(u, v);
        // Lichen grows on the plates, not in the fissures, and breaks up into grains at its edge.
        let lichen = smoothstep(
            0.1,
            0.22,
            crust.at(u, v) + 0.5 * crumb + 0.25 * grit.at(u, v),
        ) * (1.0 - fissure);
        // Moss fills fissures first and spreads out in ragged, broken patches.
        let cushion = smoothstep(
            0.25,
            0.45,
            moss.at(u, v) + 0.5 * fissure + 0.6 * crumb + 0.4 * lumps.at(u, v),
        );
        (fissure, lichen, cushion, relief)
    }
}

/// The colours of a deeply furrowed bark, whose shape is an oak's.
#[derive(Clone, Copy)]
struct Furrowed {
    /// The ridges, streaked between these two along the stem.
    ridge: (Vec3, Vec3),
    /// Deep in a fissure.
    fissure: Vec3,
    /// How much of the lichen and of the moss the shape allows shows.
    lichen: f32,
    moss: f32,
}

fn oak(seed: u32) -> Surface {
    furrowed(
        seed,
        Furrowed {
            ridge: (Vec3::new(0.12, 0.095, 0.075), Vec3::new(0.17, 0.11, 0.075)),
            fissure: Vec3::new(0.016, 0.011, 0.004),
            lichen: 0.9,
            moss: 0.85,
        },
    )
}

fn willow(seed: u32) -> Surface {
    furrowed(
        seed + 400,
        Furrowed {
            // Greyer and paler than oak, the ridges weathered almost to ash in places.
            ridge: (Vec3::new(0.135, 0.12, 0.1), Vec3::new(0.2, 0.165, 0.115)),
            // The furrows show tan inner bark rather than black.
            fissure: Vec3::new(0.045, 0.028, 0.012),
            lichen: 0.45,
            moss: 1.0,
        },
    )
}

fn stringybark(seed: u32) -> Surface {
    furrowed(
        seed + 500,
        Furrowed {
            // Weathered grey-brown strings, warmer where they are newly split.
            ridge: (Vec3::new(0.15, 0.115, 0.085), Vec3::new(0.25, 0.165, 0.1)),
            // Red-brown inner bark deep in the furrows.
            fissure: Vec3::new(0.1, 0.04, 0.018),
            lichen: 0.12,
            moss: 0.0,
        },
    )
}

fn ironbark(seed: u32) -> Surface {
    furrowed(
        seed + 600,
        Furrowed {
            // Charcoal, a little brown where the light catches a ridge.
            ridge: (
                Vec3::new(0.028, 0.025, 0.024),
                Vec3::new(0.07, 0.056, 0.048),
            ),
            // Kino, the dark red gum that bleeds into the furrows and sets hard.
            fissure: Vec3::new(0.05, 0.012, 0.008),
            lichen: 0.06,
            moss: 0.0,
        },
    )
}

/// A smooth gum's bark: blotches of different ages, as the old bark comes away in sheets of
/// no particular shape and bares the new under it.
///
/// Nothing in it is regular. Where each blotch ends is where a broad, soft noise crosses a
/// level, so the patches are the shapes of a map's contours, long along the stem, each inside
/// or beside the next, as on a manna gum or a snow gum. A grid of plates, however jittered,
/// lines up into stripes round a thin stem.
fn gum(seed: u32) -> Surface {
    // How long ago the bark here was bared: broad shapes a hand wide and a forearm long, with
    // smaller ones on them, and the whole pushed about so that no edge runs straight.
    let broad = noise(2, 2, 3, seed + 701);
    let lesser = noise(4, 5, 3, seed + 702);
    let push = noise(3, 3, 2, seed + 703);
    let age = move |u: f32, v: f32| {
        let (du, dv) = (push.at(u, v) * 0.012, push.at(v + 0.31, u + 0.67) * 0.05);
        broad.at(u + du, v + dv) * 0.68 + lesser.at(u + du, v + dv) * 0.32
    };
    // The levels the blotches end at, youngest bark first.
    const LEVELS: [f32; 4] = [-0.3, -0.04, 0.2, 0.42];
    let fine = noise(60, 40, 3, seed + 704);
    let grain = noise(160, 500, 2, seed + 705);
    let weather = noise(1, 1, 3, seed + 706);
    // Faint long streaks where rain runs down the stem, and darker stains below a wound.
    let runs = noise(5, 1, 3, seed + 707);
    let stains = noise(3, 1, 4, seed + 708);
    let specks = noise(40, 60, 2, seed + 709);
    Surface::shaped(
        move |u, v| {
            let a = age(u, v);
            // Which blotch, and how near its edge: there the older bark still overlaps.
            let band = LEVELS.iter().filter(|&&level| a > level).count();
            let edge = LEVELS
                .iter()
                .map(|&level| (a - level).abs())
                .fold(f32::MAX, f32::min);
            let lip = 1.0 - smoothstep(0.0, 0.035, edge);
            // Each older layer stands a fraction of a millimetre proud of the one it covers.
            let height = band as f32 * 0.0005 - 0.0007 * lip
                + 0.0004 * fine.at(u, v)
                + 0.0001 * grain.at(u, v);
            (height, band as f32 + lip.min(0.99))
        },
        move |u, v, tag, relief: Relief| {
            let band = tag.floor() as usize;
            let lip = tag.fract();
            // Newly bared bark is cream, nearly white; it greys in the weather; the oldest
            // still on the tree is salmon and tan, and what is about to fall, grey-brown.
            let color = [
                Vec3::new(0.63, 0.58, 0.47),
                Vec3::new(0.52, 0.5, 0.43),
                Vec3::new(0.42, 0.41, 0.37),
                Vec3::new(0.47, 0.32, 0.22),
                Vec3::new(0.3, 0.27, 0.23),
            ][band.min(4)];
            let mut color = color
                * (1.0 + 0.1 * weather.at(u, v))
                * (1.0 + 0.06 * fine.at(u, v))
                * (1.0 + 0.05 * runs.at(u, v));
            let stain = smoothstep(0.45, 0.85, stains.at(u, v));
            color = color.lerp(color * Vec3::new(0.62, 0.54, 0.47), stain * 0.5);
            // Flecks of dark where insects and lichen have been.
            let fleck = smoothstep(0.55, 0.8, specks.at(u, v)) * 0.18;
            color = color.lerp(Vec3::new(0.14, 0.11, 0.09), fleck);
            // An edge is the shadowed underside of the sheet above it.
            color = color.lerp(color * 0.55, lip * 0.5);
            color *= 1.0 + (relief.fine * 150.0).clamp(-0.12, 0.12);
            Shade {
                color,
                // Smooth, with a faint waxy sheen where it is newest.
                roughness: 0.6 + 0.07 * band as f32,
                cavity: 1.0 - 0.35 * lip,
            }
        },
    )
}

fn furrowed(seed: u32, tones: Furrowed) -> Surface {
    let shape = oak_shape(seed);
    let shade_shape = oak_shape(seed);
    let grit = noise(160, 660, 2, seed + 12);
    let crumbs = noise(48, 200, 3, seed + 13);
    // Broad light and dark areas, a hand's width and more, as weather and shade leave them.
    let broad = noise(1, 2, 3, seed + 14);
    let streaks = noise(10, 4, 4, seed + 8);
    let blotches = noise(6, 20, 3, seed + 15);
    let lichen_tone = noise(8, 30, 3, seed + 10);
    let speckle = noise(120, 480, 2, seed + 9);
    Surface::shaped(
        move |u, v| {
            let (fissure, lichen, moss, relief) = shape(u, v);
            // Fissures about 1.5 cm deep, crusts and moss standing a little proud.
            // Moss is a cushion of tiny leaves, bumpy all over.
            let cushion = moss * (0.0015 + 0.0015 * grit.at(u, v) + 0.001 * crumbs.at(u, v));
            let height = -0.014 * fissure + relief + 0.0006 * lichen + cushion;
            (height, fissure)
        },
        move |u, v, fissure, relief: Relief| {
            let (_, lichen, moss, _) = shade_shape(u, v);
            // Warm brown bark, streaked along the stem from grey-brown to reddish, paler on
            // the crumbly high points.
            let streak = streaks.at(u, v);
            let bark = tones.ridge.0.lerp(tones.ridge.1, streak * 0.5 + 0.5)
                * (1.0 + 0.3 * broad.at(u, v))
                * (1.0 + 0.25 * blotches.at(u, v));
            let mut color = bark * (1.0 + (relief.fine * 200.0).clamp(-0.5, 0.55));
            color = color.lerp(tones.fissure, smoothstep(0.2, 0.75, fissure));
            // Lichen from pale grey-cream to grey-green, grainy within.
            let tone = lichen_tone.at(u, v) * 0.5 + 0.5;
            let crust = Vec3::new(0.34, 0.31, 0.23).lerp(Vec3::new(0.22, 0.24, 0.17), tone)
                * (1.0 + 0.25 * speckle.at(u, v));
            color = color.lerp(crust, lichen * tones.lichen);
            // Moss is dark olive, grainy with its tiny leaves and the shade between them.
            let grain = (speckle.at(u, v) * 0.5 + crumbs.at(u, v) * 0.6 + 0.5).clamp(0.0, 1.0);
            let cushion = Vec3::new(0.025, 0.03, 0.01).lerp(Vec3::new(0.09, 0.1, 0.04), grain)
                * (1.0 + (relief.fine * 250.0).clamp(-0.5, 0.6));
            color = color.lerp(cushion, (moss * tones.moss).min(0.92));
            color *= 1.0 + 0.15 * speckle.at(u, v);
            Shade {
                color,
                roughness: 0.92 - 0.1 * moss,
                cavity: 1.0 - 0.85 * smoothstep(0.1, 0.9, fissure),
            }
        },
    )
}

/// Birch markings at a point: (lenticel, dark patch, crack in a patch, peel).
fn birch_marks(seed: u32) -> impl Fn(f32, f32) -> (f32, f32, f32, f32) + Send + Sync {
    // Lenticels: dark dashes 1 to 4 cm long and a few millimetres tall, running around the stem.
    // Cells 6 cm wide and 8 mm tall, a third of which hold a dash in their middle.
    let lenticels = Cells {
        period: 4,
        seed: seed + 10,
    };
    // Each row of lenticels is shifted at random, so they do not stack into columns.
    let rows = along(fbm(60, 1, seed + 1), 1);
    // Black patches 10 to 30 cm long, some joined into rough bands.
    let patches = Cells {
        period: 2,
        seed: seed + 2,
    };
    let patch_edge = wrapped(fbm(3, 3, seed + 3), 6);
    let cracks = Cells {
        period: 3,
        seed: seed + 11,
    };
    // Papery strips peeling around the stem.
    let peel = wrapped(fbm(1, 3, seed + 5), 40);
    move |u, v| {
        let cell = cells_at(lenticels, 1, 120, u + rows.at(u, v) / AROUND as f32, v);
        let dash = if cell.id < 0.35 {
            1.0 - smoothstep(0.18, 0.32, cell.nearest)
        } else {
            0.0
        };
        let patch = cells_at(patches, 1, 6, u, v);
        let dark = if patch.id < 0.16 {
            1.0 - smoothstep(0.3, 0.42, patch.nearest + patch_edge.at(u, v) * 0.12)
        } else {
            0.0
        };
        let crack = (1.0 - smoothstep(0.0, 0.15, cells_at(cracks, 2, 10, u, v).border())) * dark;
        let peeling = smoothstep(0.25, 0.45, peel.at(u, v));
        (dash, dark, crack, peeling)
    }
}

fn birch(seed: u32) -> Surface {
    let marks = birch_marks(seed);
    let shade_marks = birch_marks(seed);
    let tint = fbm(4, 3, seed + 6).unit();
    Surface::shaped(
        move |u, v| {
            let (dash, dark, crack, peel) = marks(u, v);
            // Black patches are rough and raised; lenticels are slight grooves.
            let height = -0.0012 * dash + 0.002 * dark - 0.006 * crack + 0.0008 * peel;
            (height, 0.0)
        },
        move |u, v, _, relief: Relief| {
            let (dash, dark, crack, peel) = shade_marks(u, v);
            let chalk = Vec3::new(0.5, 0.49, 0.45).lerp(Vec3::new(0.44, 0.42, 0.37), tint.at(u, v));
            // Peeling strips show the warmer, pinkish layer beneath.
            let mut color = chalk.lerp(Vec3::new(0.42, 0.33, 0.27), peel * 0.5);
            color = color.lerp(Vec3::new(0.05, 0.045, 0.04), dash * 0.9);
            color = color.lerp(Vec3::new(0.04, 0.035, 0.03), dark * 0.9);
            color = color.lerp(Vec3::new(0.015, 0.012, 0.01), crack);
            color *= 1.0 + (relief.fine * 40.0).clamp(-0.1, 0.1);
            Shade {
                color,
                // The papery surface has a faint sheen; the black patches are rough.
                roughness: 0.55 + 0.35 * dark,
                cavity: 1.0 - 0.3 * dash - 0.5 * crack,
            }
        },
    )
}

/// Fir at a point: (crack between small plates, resin blister).
fn fir_shape(seed: u32) -> impl Fn(f32, f32) -> (f32, f32) + Send + Sync {
    // Faint plates about 5 cm across, only on parts of the stem.
    let plates = Cells { period: 3, seed };
    let aged = along(fbm(1, 3, seed + 1), 3);
    let blisters = Cells {
        period: 4,
        seed: seed + 2,
    };
    move |u, v| {
        let crack = (1.0 - smoothstep(0.0, 0.12, cells_at(plates, 2, 24, u, v).border()))
            * smoothstep(-0.25, 0.15, aged.at(u, v));
        let blister = cells_at(blisters, 1, 40, u, v);
        let bump = if blister.id < 0.12 {
            1.0 - smoothstep(0.0, 0.3, blister.nearest)
        } else {
            0.0
        };
        (crack, bump)
    }
}

fn fir(seed: u32) -> Surface {
    let shape = fir_shape(seed);
    let shade_shape = fir_shape(seed);
    let mottle = wrapped(fbm(1, 4, seed + 3), 24);
    let lichen = along(fbm(5, 5, seed + 4), 1).unit().smooth(0.62, 0.75);
    Surface::shaped(
        move |u, v| {
            let (crack, bump) = shape(u, v);
            (-0.003 * crack + 0.002 * bump * bump, crack)
        },
        move |u, v, crack, relief: Relief| {
            let (_, bump) = shade_shape(u, v);
            let grey = Vec3::new(0.13, 0.13, 0.125) * (1.0 + 0.3 * mottle.at(u, v));
            let mut color = grey.lerp(Vec3::new(0.07, 0.065, 0.06), crack);
            color = color.lerp(Vec3::new(0.2, 0.19, 0.16), bump * 0.4);
            color = color.lerp(Vec3::new(0.25, 0.28, 0.2), lichen.at(u, v) * 0.6);
            color *= 1.0 + (relief.fine * 60.0).clamp(-0.08, 0.1);
            Shade {
                color,
                roughness: 0.7,
                cavity: 1.0 - 0.4 * crack,
            }
        },
    )
}

/// Pine at a point: (furrow between plates, plate id, flaking).
fn pine_plates(seed: u32) -> impl Fn(f32, f32) -> (f32, f32, f32) + Send + Sync {
    // Plates about 8 cm across and 25 cm long.
    let plates = Cells { period: 3, seed };
    let warp = fbm(6, 2, seed + 1);
    let flakes = wrapped(fbm(12, 3, seed + 2), 6);
    move |u, v| {
        let w = warp.at((u * AROUND as f32).rem_euclid(1.0), v) * 0.003;
        let cell = cells_at(plates, 1, 5, u + w, v + w);
        let furrow = 1.0 - smoothstep(0.02, 0.2, cell.border());
        (furrow, cell.id, flakes.at(u, v))
    }
}

fn pine(seed: u32) -> Surface {
    let plates = pine_plates(seed);
    let shade_plates = pine_plates(seed);
    let grain = wrapped(fbm(40, 2, seed + 3), 4);
    let tint = fbm(3, 3, seed + 4).unit();
    Surface::shaped(
        move |u, v| {
            let (furrow, id, flake) = plates(u, v);
            let height =
                -0.015 * furrow + (id - 0.5) * 0.004 + 0.0015 * flake + 0.0004 * grain.at(u, v);
            (height, furrow)
        },
        move |u, v, furrow, relief: Relief| {
            let (_, _, flake) = shade_plates(u, v);
            let plate =
                Vec3::new(0.15, 0.085, 0.055).lerp(Vec3::new(0.11, 0.075, 0.06), tint.at(u, v));
            let mut color = plate.lerp(Vec3::new(0.035, 0.025, 0.02), smoothstep(0.2, 0.9, furrow));
            // Fresh flakes are orange where the outer layers have fallen away.
            let fresh = smoothstep(0.35, 0.7, flake) * (1.0 - furrow);
            color = color.lerp(Vec3::new(0.3, 0.13, 0.06), fresh * 0.6);
            color *= 1.0 + (relief.fine * 50.0).clamp(-0.12, 0.15);
            Shade {
                color,
                roughness: 0.85,
                cavity: 1.0 - furrow * 0.6,
            }
        },
    )
}

fn beech(seed: u32) -> Surface {
    // Fine wrinkles running around the stem, wavering as they go.
    let wrinkles = wrapped(ridged(1, 2, seed).warped(fbm(2, 2, seed + 1), 0.08), 90);
    let mottle = along(fbm(4, 4, seed + 2), 1);
    let lichen = wrapped(fbm(1, 4, seed + 3), 12);
    Surface::shaped(
        move |u, v| {
            let wrinkle = smoothstep(0.65, 0.9, wrinkles.at(u, v));
            let crust = smoothstep(0.2, 0.45, lichen.at(u, v));
            (-0.0008 * wrinkle + 0.0012 * crust, crust)
        },
        move |u, v, crust, relief: Relief| {
            let bark = Vec3::new(0.24, 0.24, 0.225)
                * (1.0 + 0.15 * mottle.at(u, v))
                * (1.0 + (relief.fine * 120.0).clamp(-0.15, 0.08));
            // Pale green-grey crusts of lichen.
            let color = bark.lerp(Vec3::new(0.26, 0.29, 0.2), crust * 0.7);
            Shade {
                color,
                roughness: 0.6 + 0.25 * crust,
                cavity: 1.0,
            }
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_bark_wraps_around_a_stem_without_a_seam() {
        // Features across bark repeat every sixteenth of the tile, so they match wherever a stem
        // closes on itself.
        for bark in Bark::ALL {
            let surface = bark.surface(0);
            for i in 0..40 {
                let v = (i as f32 + 0.5) / 40.0;
                let a = surface.formula_at(0.0, v);
                let b = surface.formula_at(1.0 / AROUND as f32, v);
                assert!(
                    (a.height - b.height).abs() < 2.0e-3,
                    "{}: {} against {}",
                    bark.name(),
                    a.height,
                    b.height
                );
            }
        }
    }

    #[test]
    fn barks_are_found_by_name() {
        for bark in Bark::ALL {
            assert_eq!(Bark::by_name(bark.name()), Some(bark));
        }
        assert_eq!(Bark::by_name("cork"), None);
    }

    /// Mean sRGB brightness of a bark, and the share of it that is dark markings.
    fn brightness(bark: Bark) -> (f32, f32) {
        let layer = bark.surface(0).bake(128);
        let pixels: Vec<f32> = layer.color_mips[0]
            .chunks_exact(4)
            .map(|c| (c[0] as f32 + c[1] as f32 + c[2] as f32) / 3.0)
            .collect();
        let mean = pixels.iter().sum::<f32>() / pixels.len() as f32;
        let dark = pixels.iter().filter(|&&p| p < mean * 0.5).count() as f32 / pixels.len() as f32;
        (mean, dark)
    }

    #[test]
    fn birch_is_pale_with_dark_markings_and_pine_is_dark() {
        let (birch, birch_marks) = brightness(Bark::Birch);
        let (oak, _) = brightness(Bark::Oak);
        let (pine, _) = brightness(Bark::Pine);
        assert!(birch > oak + 40.0, "birch {birch}, oak {oak}");
        assert!(pine < birch, "pine {pine}, birch {birch}");
        // Lenticels and black patches must actually appear.
        assert!(
            (0.04..0.4).contains(&birch_marks),
            "{birch_marks} of birch is dark"
        );
    }

    #[test]
    fn oak_has_deep_fissures() {
        let surface = Bark::Oak.surface(0);
        let heights: Vec<f32> = (0..400)
            .map(|i| {
                surface
                    .formula_at((i % 20) as f32 / 320.0, (i / 20) as f32 / 80.0)
                    .height
            })
            .collect();
        let (lo, hi) = heights
            .iter()
            .fold((f32::MAX, f32::MIN), |(a, b), &h| (a.min(h), b.max(h)));
        assert!(hi - lo > 0.015, "relief of {} m", hi - lo);
    }
}
