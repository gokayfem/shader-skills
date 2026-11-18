//! The road itself, and what is built beside it: asphalt with the rubber cars have laid down
//! on it, its lines and joints and tyre marks, kerbs, gravel, concrete, and the steel barriers.

use super::line::{racing_line, turning};
use super::{lay, smoothstep, Land, Road};
use glam::{Vec2, Vec3};
use solstice_asset::{Material, SceneAsset};
use solstice_game::{Blocker, Track};
use solstice_procgen::foliage::Rng;
use solstice_procgen::noise::fbm;
use solstice_procgen::surface::TILE_METRES;

/// How many shades of rubber there are, from the faint edge of the racing line to its middle.
const SHADES: usize = 5;
/// How far above the road its paint is, and the other thin things laid on it.
const PAINT: f32 = 0.012;
const MARK: f32 = 0.012;
/// How high the wall that stops a car at a barrier stands: far above the barrier itself, as
/// a circuit's catch fence does, so that a car which has left the ground is stopped too.
const WALL: f32 = 10.0;
/// And how deep it is, behind the barrier. A thin wall stops a car that arrives fast, but with
/// the car's nose more than half-way through it; and what is more than half-way through a
/// wall is pushed out of its far side.
const WALL_DEPTH: f32 = 5.0;
/// How far apart along the road its surface is cut, in steps to a sample of the track.
const STEPS: usize = 3;

/// A cut across the road.
struct Section {
    centre: Vec3,
    left: Vec3,
    half_left: f32,
    half_right: f32,
    /// How far to the left of the middle the racing line is.
    line: f32,
    /// How hard cars are working here: none on a straight, all of it braking for and rounding
    /// a slow corner.
    work: f32,
    /// How far round the lap, in metres.
    along: f32,
}

impl Section {
    fn at(&self, offset: f32, lift: f32) -> Vec3 {
        self.centre + self.left * offset + Vec3::Y * lift
    }
}

/// The road cut across every metre or so, with the racing line marked on each cut.
fn sections(track: &Track) -> Vec<Section> {
    let samples = &track.samples;
    let count = samples.len();
    let line = racing_line(track);
    let driven: Vec<Vec2> = (0..count)
        .map(|i| {
            let p = samples[i].position + samples[i].left * line[i];
            Vec2::new(p.x, p.z)
        })
        .collect();
    let turn = turning(&driven);
    // Cars work hardest in the corners, and braking on the way into them.
    let cornering: Vec<f32> = turn
        .iter()
        .map(|t| smoothstep(0.002, 0.014, t.abs()))
        .collect();
    let mut work: Vec<f32> = (0..count)
        .map(|i| {
            let braking = (0..40)
                .map(|k| cornering[(i + k) % count] * (1.0 - k as f32 / 40.0))
                .fold(0.0, f32::max);
            let leaving = (0..14)
                .map(|k| cornering[(i + count - k) % count] * (1.0 - k as f32 / 14.0))
                .fold(0.0, f32::max);
            cornering[i].max(braking * 0.9).max(leaving * 0.7)
        })
        .collect();
    for _ in 0..4 {
        work = (0..count)
            .map(|i| (work[(i + count - 1) % count] + 2.0 * work[i] + work[(i + 1) % count]) * 0.25)
            .collect();
    }
    let mut out = Vec::with_capacity(count * STEPS);
    let mut along = 0.0;
    for i in 0..count {
        let (here, next) = (&samples[i], &samples[(i + 1) % count]);
        let span = here.position.distance(next.position);
        let j = (i + 1) % count;
        for step in 0..STEPS {
            let share = step as f32 / STEPS as f32;
            let between = |a: f32, b: f32| a + (b - a) * share;
            out.push(Section {
                centre: here.position.lerp(next.position, share),
                left: here.left.lerp(next.left, share).normalize_or(here.left),
                half_left: between(here.half_left, next.half_left),
                half_right: between(here.half_right, next.half_right),
                line: between(line[i], line[j]),
                work: between(work[i], work[j]),
                along: along + span * share,
            });
        }
        along += span;
    }
    out
}

/// A wandering number from -1 to 1 that changes every few metres along the road.
fn wander(along: f32, which: u32) -> f32 {
    fbm(
        Vec3::new(along * 0.31, which as f32 * 7.3, 0.5),
        2,
        500 + which,
    )
}

/// The paints and surfaces the road is made of.
struct Paints {
    asphalt: u32,
    rubber: [u32; SHADES],
    mark: u32,
    joint: u32,
    black: u32,
    apron: u32,
    gravel: u32,
    white: u32,
    red: u32,
    green: u32,
    steel: u32,
}

impl Paints {
    fn new(scene: &mut SceneAsset) -> Self {
        let textured = |scene: &mut SceneAsset, layer: i32, tint: [f32; 3], roughness: f32| {
            scene.materials.push(Material {
                base_color: [tint[0], tint[1], tint[2], 1.0],
                roughness,
                uv_scale: 1.0 / TILE_METRES,
                normal_strength: 1.0,
                texture_layer: layer,
                // A little, to hide the repeat: more would blotch it.
                macro_variation: 0.22,
                ..Default::default()
            });
            scene.materials.len() as u32 - 1
        };
        let painted =
            |scene: &mut SceneAsset, color: [f32; 3], roughness: f32, metallic: f32, flags: u32| {
                scene.materials.push(Material {
                    base_color: [color[0], color[1], color[2], 1.0],
                    roughness,
                    metallic,
                    flags,
                    ..Default::default()
                });
                scene.materials.len() as u32 - 1
            };
        let stone = scene.textures.len() as i32;
        scene
            .textures
            .push(crate::textures::surface("asphalt", scene.texture_size));
        let grit = scene.textures.len() as i32;
        scene
            .textures
            .push(crate::textures::surface("gravel", scene.texture_size));
        // Rubber darkens the road and fills its grain, shade by shade towards the middle of
        // the line.
        let rubber = std::array::from_fn(|shade| {
            let depth = (shade + 1) as f32 / SHADES as f32;
            let tint = 1.0 - 0.3 * depth;
            textured(scene, stone, [tint, tint, tint * 1.01], 1.0 - 0.14 * depth)
        });
        Self {
            asphalt: textured(scene, stone, [1.0; 3], 1.0),
            rubber,
            mark: textured(scene, stone, [0.56, 0.56, 0.57], 0.82),
            joint: textured(scene, stone, [0.6, 0.6, 0.61], 0.8),
            black: painted(scene, [0.012, 0.012, 0.013], 0.7, 0.0, 0),
            // Concrete, near enough: the same stone, pale.
            apron: textured(scene, stone, [3.3, 3.2, 3.0], 1.0),
            gravel: textured(scene, grit, [0.95, 0.93, 0.9], 1.0),
            white: painted(scene, [0.80, 0.80, 0.78], 0.6, 0.0, 0),
            red: painted(scene, [0.50, 0.035, 0.03], 0.55, 0.0, 0),
            green: painted(scene, [0.045, 0.16, 0.07], 0.8, 0.0, 0),
            steel: painted(scene, [0.60, 0.62, 0.64], 0.42, 0.9, Material::TWO_SIDED),
        }
    }
}

/// Lays the road and everything built beside it. Gives back the barriers, as walls.
/// `grass` is the land's own grass, which the verge at the road's edge is made of too.
pub(super) fn pave(scene: &mut SceneAsset, road: &Road, land: &Land, grass: u32) -> Vec<Blocker> {
    let track = road.track;
    let samples = &track.samples;
    let count = samples.len();
    let paints = Paints::new(scene);
    let cuts = sections(track);
    let cut = |index: usize| &cuts[index % cuts.len()];

    // ---- The asphalt, in bands along the road: clean at its edges, and darker shade by shade
    // towards the middle of the racing line, where the rubber is thickest. Each band's edge
    // wanders a little, as the cars do.
    let edges = |section: &Section| -> Vec<f32> {
        // How far out from the line each shade reaches. Cars spread out on the straights and
        // lay little rubber there; the darkest shades are only where they work hardest.
        let wide = 2.9 - 0.5 * section.work;
        let reach: Vec<f32> = (0..SHADES)
            .map(|shade| {
                let share = 1.0 - shade as f32 / SHADES as f32;
                let begins = (shade as f32 - 1.0) / (SHADES as f32 - 1.0) * 0.9;
                let there = smoothstep(begins - 0.15, begins + 0.2, section.work.max(0.02));
                wide * share * there
            })
            .collect();
        let mut edges = vec![-section.half_right];
        for (shade, &reach) in reach.iter().enumerate() {
            let ragged = wander(section.along, shade as u32) * 0.28 * reach.min(1.0);
            edges.push(section.line - reach + ragged);
        }
        for (shade, &reach) in reach.iter().enumerate().rev() {
            let ragged = wander(section.along, (SHADES + shade) as u32) * 0.28 * reach.min(1.0);
            edges.push(section.line + reach + ragged);
        }
        edges.push(section.half_left);
        // In order across the road, and on it.
        let (low, high) = (-section.half_right, section.half_left);
        let mut least = low;
        for edge in edges.iter_mut() {
            *edge = edge.clamp(least, high);
            least = *edge;
        }
        edges
    };
    let mut bands: Vec<Vec<[Vec3; 4]>> = vec![Vec::new(); SHADES + 1];
    for index in 0..cuts.len() {
        let (here, next) = (cut(index), cut(index + 1));
        let (a, b) = (edges(here), edges(next));
        for band in 0..a.len() - 1 {
            if a[band + 1] - a[band] < 1.0e-4 && b[band + 1] - b[band] < 1.0e-4 {
                continue;
            }
            // Which shade a band is: none at the road's edges, deepest in the middle.
            let shade = band.min(2 * SHADES - band);
            bands[shade].push([
                here.at(a[band], 0.0),
                here.at(a[band + 1], 0.0),
                next.at(b[band + 1], 0.0),
                next.at(b[band], 0.0),
            ]);
        }
    }
    lay(scene, &bands[0], paints.asphalt);
    for shade in 0..SHADES {
        lay(scene, &bands[shade + 1], paints.rubber[shade]);
    }

    // ---- The joints where the lanes it was laid in meet, sealed with bitumen.
    let mut joints = Vec::new();
    for index in 0..cuts.len() {
        let (here, next) = (cut(index), cut(index + 1));
        for lane in [-2.17_f32, 2.17] {
            joints.push([
                here.at(lane - 0.025, MARK),
                here.at(lane + 0.025, MARK),
                next.at(lane + 0.025, MARK),
                next.at(lane - 0.025, MARK),
            ]);
        }
    }
    lay(scene, &joints, paints.joint);

    // ---- Tyre marks: where cars have braked late into a corner, a locked wheel or a pair; and
    // where they have put the power down too soon out of one.
    let mut marks = Vec::new();
    let mut rng = Rng::new(23, 9);
    let total = cuts.len();
    let mut index = 0;
    while index < total {
        let work = cut(index).work;
        let (count_here, long) = if work > 0.55 { (3, 55.0) } else { (0, 0.0) };
        for _ in 0..count_here {
            if rng.unit() > 0.16 {
                continue;
            }
            let length = (rng.range(12.0, long) * STEPS as f32 / 3.0) as usize;
            let aside = rng.range(-1.6, 1.6);
            let drift = rng.range(-0.012, 0.012);
            let pair = rng.unit() < 0.6;
            let width = rng.range(0.2, 0.32);
            for step in 0..length {
                let (here, next) = (cut(index + step), cut(index + step + 1));
                // A mark fades in and out: it is narrower at its ends.
                let fade = |s: usize| {
                    let t = s as f32 / length as f32;
                    (smoothstep(0.0, 0.12, t) * smoothstep(1.0, 0.8, t)).max(0.45)
                };
                for wheel in [-0.8_f32, 0.8] {
                    if !pair && wheel > 0.0 {
                        continue;
                    }
                    let middle = |section: &Section, s: usize| {
                        let limit = section.half_left.min(section.half_right) - 0.6;
                        (section.line + aside + drift * s as f32 + wheel).clamp(-limit, limit)
                    };
                    let (m0, m1) = (middle(here, step), middle(next, step + 1));
                    let (w0, w1) = (width * 0.5 * fade(step), width * 0.5 * fade(step + 1));
                    marks.push([
                        here.at(m0 - w0, MARK),
                        here.at(m0 + w0, MARK),
                        next.at(m1 + w1, MARK),
                        next.at(m1 - w1, MARK),
                    ]);
                }
            }
        }
        index += 5;
    }
    lay(scene, &marks, paints.mark);

    // ---- The lines: one down each edge, the start line, and the grid.
    let mut lines = Vec::new();
    for index in 0..cuts.len() {
        let (here, next) = (cut(index), cut(index + 1));
        for (side, a, b) in [
            (1.0, here.half_left, next.half_left),
            (-1.0, here.half_right, next.half_right),
        ] {
            lines.push([
                here.at(side * (a - 0.36), PAINT),
                here.at(side * (a - 0.21), PAINT),
                next.at(side * (b - 0.21), PAINT),
                next.at(side * (b - 0.36), PAINT),
            ]);
        }
    }
    let stripe = |lines: &mut Vec<[Vec3; 4]>, along: f32, from: f32, to: f32, length: f32| {
        let (centre, forward, left) = track.place(along);
        let (a, b) = (
            centre + left * from + Vec3::Y * PAINT,
            centre + left * to + Vec3::Y * PAINT,
        );
        lines.push([a, b, b + forward * length, a + forward * length]);
    };
    // The start line is chequered: two rows of squares.
    let (from, to) = (-samples[0].half_right + 0.5, samples[0].half_left - 0.5);
    let squares = ((to - from) / 0.5).floor() as usize;
    let square = (to - from) / squares as f32;
    for row in 0..2 {
        for column in 0..squares {
            if (row + column) % 2 == 0 {
                let a = from + column as f32 * square;
                stripe(&mut lines, row as f32 * square, a, a + square, square);
            }
        }
    }
    for slot in 0..12 {
        // Two by two, staggered: each car's box is a line across its nose and one down each
        // side.
        let along = -(8.0 + slot as f32 * 8.0);
        let side = if slot % 2 == 0 { 1.0 } else { -1.0 };
        stripe(&mut lines, along, side * 1.45, side * 4.05, 0.12);
        for edge in [1.45_f32, 3.93] {
            stripe(
                &mut lines,
                along - 1.6,
                side * edge,
                side * (edge + 0.12),
                1.6,
            );
        }
    }
    lay(scene, &lines, paints.white);
    // The squares between are black.
    let mut blacks = Vec::new();
    for row in 0..2 {
        for column in 0..squares {
            if (row + column) % 2 == 1 {
                let a = from + column as f32 * square;
                stripe(&mut blacks, row as f32 * square, a, a + square, square);
            }
        }
    }
    lay(scene, &blacks, paints.black);

    // ---- What lies beside the road: kerbs in blocks of red and white, gravel, concrete; a
    // strip of green behind a kerb with nothing else behind it; and round the outside of it
    // all the verge, so that the grass meets its edge.
    let mut kerbs = [Vec::new(), Vec::new()];
    let mut traps = Vec::new();
    let mut aprons = Vec::new();
    let mut greens = Vec::new();
    let mut verges = Vec::new();
    for i in 0..count {
        let (here, next) = (&samples[i], &samples[(i + 1) % count]);
        for (side, half, half_next, bands, bands_next) in [
            (
                1.0,
                here.half_left,
                next.half_left,
                &here.left_bands,
                &next.left_bands,
            ),
            (
                -1.0,
                here.half_right,
                next.half_right,
                &here.right_bands,
                &next.right_bands,
            ),
        ] {
            let mut from = 0.0;
            let (mut edge, mut edge_next) = (half, half_next);
            for (what, width) in bands {
                let (a0, a1) = (half + from, half + from + width);
                // Where the same band is at the next sample: it may be wider or narrower there.
                // One that ends does so squarely.
                let (b0, b1) = match bands_next
                    .iter()
                    .position(|band| band.0.name() == what.name())
                {
                    Some(k) => {
                        let before: f32 = bands_next[..k].iter().map(|band| band.1).sum();
                        (half_next + before, half_next + before + bands_next[k].1)
                    }
                    None => (half_next + from, half_next + from + width),
                };
                match what.name() {
                    "kerb" => {
                        // Rising from the road's edge to a flat top, a hand's breadth up.
                        let colour = i % 2;
                        let (mid, mid_next) = (a0 + (a1 - a0) * 0.45, b0 + (b1 - b0) * 0.45);
                        kerbs[colour].push([
                            road.at(i, side * a0, PAINT),
                            road.at(i, side * mid, 0.04),
                            road.at(i + 1, side * mid_next, 0.04),
                            road.at(i + 1, side * b0, PAINT),
                        ]);
                        kerbs[colour].push([
                            road.at(i, side * mid, 0.04),
                            road.at(i, side * a1, 0.03),
                            road.at(i + 1, side * b1, 0.03),
                            road.at(i + 1, side * mid_next, 0.04),
                        ]);
                    }
                    other => {
                        let piece = [
                            road.at(i, side * a0, 0.01),
                            road.at(i, side * a1, 0.01),
                            road.at(i + 1, side * b1, 0.01),
                            road.at(i + 1, side * b0, 0.01),
                        ];
                        if other == "gravel" {
                            traps.push(piece);
                        } else {
                            aprons.push(piece);
                        }
                    }
                }
                from += width;
                (edge, edge_next) = (a1, b1);
            }
            let kerb_last = |bands: &[solstice_game::track::Band]| {
                bands.last().is_some_and(|band| band.0.name() == "kerb")
            };
            if kerb_last(bands) && kerb_last(bands_next) {
                greens.push([
                    road.at(i, side * edge, 0.03),
                    road.at(i, side * (edge + 0.9), 0.012),
                    road.at(i + 1, side * (edge_next + 0.9), 0.012),
                    road.at(i + 1, side * edge_next, 0.03),
                ]);
                (edge, edge_next) = (edge + 0.9, edge_next + 0.9);
            }
            // The verge: grass from the very edge, falling away under the land beside it,
            // which is drawn a little low there.
            verges.push([
                road.at(i, side * edge, 0.0),
                road.at(i, side * (edge + 1.7), -0.11),
                road.at(i + 1, side * (edge_next + 1.7), -0.11),
                road.at(i + 1, side * edge_next, 0.0),
            ]);
        }
    }
    lay(scene, &kerbs[0], paints.red);
    lay(scene, &kerbs[1], paints.white);
    lay(scene, &traps, paints.gravel);
    lay(scene, &aprons, paints.apron);
    lay(scene, &greens, paints.green);
    lay(scene, &verges, grass);

    // ---- The barriers: two steel rails, one above the other, on posts, a way back from each
    // edge; and a wall to stop a car where each stretch of them stands.
    let mut rails = Vec::new();
    let mut blockers = Vec::new();
    // A rail's shape, cut across: how far it stands out towards the road, and how high, from
    // its middle. Two waves, as a crash barrier has.
    const RAIL: [(f32, f32); 10] = [
        (0.000, -0.155),
        (0.030, -0.125),
        (0.083, -0.088),
        (0.083, -0.052),
        (0.030, -0.014),
        (0.030, 0.014),
        (0.083, 0.052),
        (0.083, 0.088),
        (0.030, 0.125),
        (0.000, 0.155),
    ];
    for (which, side) in [(0, 1.0_f32), (1, -1.0)] {
        let line: Vec<Vec3> = (0..count)
            .map(|i| {
                let sample = &samples[i];
                let flat = (sample.left * Vec3::new(1.0, 0.0, 1.0)).normalize_or_zero();
                let p = sample.position + flat * side * road.fence[which][i];
                Vec3::new(p.x, land.at(p.x, p.z), p.z)
            })
            .collect();
        // Towards the road, at each point of the line.
        let inward: Vec<Vec3> = (0..count)
            .map(|i| {
                let along = line[(i + 1) % count] - line[(i + count - 1) % count];
                Vec3::new(-along.z, 0.0, along.x).normalize_or_zero() * -side
            })
            .collect();
        for i in 0..count {
            let j = (i + 1) % count;
            let (a, b) = (line[i], line[j]);
            if a.distance(b) < 0.05 {
                continue;
            }
            for middle in [0.44_f32, 0.78] {
                for pair in RAIL.windows(2) {
                    let at = |p: Vec3, inward: Vec3, (out, high): (f32, f32)| {
                        p + inward * out + Vec3::Y * (middle + high)
                    };
                    rails.push([
                        at(a, inward[i], pair[0]),
                        at(a, inward[i], pair[1]),
                        at(b, inward[j], pair[1]),
                        at(b, inward[j], pair[0]),
                    ]);
                }
            }
            // A post behind the rails at each joint: a length of channel.
            let along = (b - a).normalize_or_zero();
            let (back, wide) = (-inward[i] * 0.11, along * 0.035);
            let corners = [a - wide, a + wide, a + wide + back, a - wide + back];
            for k in 0..4 {
                let (p, q) = (corners[k], corners[(k + 1) % 4]);
                rails.push([
                    p - Vec3::Y * 0.1,
                    p + Vec3::Y * 0.96,
                    q + Vec3::Y * 0.96,
                    q - Vec3::Y * 0.1,
                ]);
            }
            if i % 2 == 0 {
                let c = line[(i + 2) % count];
                let (dx, dz) = (c.x - a.x, c.z - a.z);
                // The wall's face is at the barrier, and the rest of it is behind: a car that
                // is part-way into it is then pushed back out the way it came, never on through.
                let behind = -(inward[i] + inward[(i + 2) % count]).normalize_or_zero();
                let middle = (a + c) * 0.5 + behind * (WALL_DEPTH * 0.5 - 0.15);
                blockers.push(Blocker {
                    centre: Vec3::new(middle.x, (a.y + c.y) * 0.5 + WALL * 0.5 - 1.5, middle.z),
                    size: Vec3::new(WALL_DEPTH, WALL, dx.hypot(dz) + 0.3),
                    yaw_degrees: dx.atan2(dz).to_degrees(),
                });
            }
        }
    }
    lay(scene, &rails, paints.steel);
    blockers
}
