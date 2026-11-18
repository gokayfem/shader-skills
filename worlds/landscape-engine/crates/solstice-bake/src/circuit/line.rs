//! The racing line: the path round a track a fast car takes, which is where the rubber goes
//! down and the tyre marks are.
//!
//! It is the path within the road's edges that bends least: out wide before a corner, across
//! to the inside at its apex, and out wide again after. That is found by sliding each point
//! of the path across the road, a little at a time, towards where the path through it and its
//! neighbours would be straightest, first with few points for the line's broad sweep and then
//! with more for its detail.

use glam::Vec2;
use solstice_game::Track;

/// How far from the road's edge the middle of a car keeps.
const MARGIN: f32 = 1.45;

/// How far to the left of the road's middle the racing line is, at each of a track's samples.
pub fn racing_line(track: &Track) -> Vec<f32> {
    let samples = &track.samples;
    let count = samples.len();
    let flat = |v: glam::Vec3| Vec2::new(v.x, v.z);
    let mut offsets = vec![0.0_f32; count];
    // Coarse to fine: every 16th sample, every 8th, and so on down to every one.
    for stride in [16usize, 8, 4, 2, 1] {
        let points = (count / stride).max(8);
        let index = |j: usize| (j * count / points) % count;
        let centre: Vec<Vec2> = (0..points)
            .map(|j| flat(samples[index(j)].position))
            .collect();
        let left: Vec<Vec2> = (0..points)
            .map(|j| flat(samples[index(j)].left).normalize_or_zero())
            .collect();
        let limits: Vec<(f32, f32)> = (0..points)
            .map(|j| {
                let sample = &samples[index(j)];
                (
                    -(sample.half_right - MARGIN).max(0.0),
                    (sample.half_left - MARGIN).max(0.0),
                )
            })
            .collect();
        let mut across: Vec<f32> = (0..points).map(|j| offsets[index(j)]).collect();
        let at =
            |across: &[f32], j: usize| centre[j % points] + left[j % points] * across[j % points];
        for _ in 0..(1500 + 12_000 / stride) {
            for j in 0..points {
                // How far from straight the path is through each of the three points this one
                // is part of, and which way moving this one would mend that.
                let bend = |k: usize| {
                    at(&across, k + points - 1) - at(&across, k) * 2.0 + at(&across, k + 1)
                };
                let pull = bend(j + points - 1) - bend(j) * 2.0 + bend(j + 1);
                let moved = across[j] - pull.dot(left[j]) / 6.0;
                across[j] = moved.clamp(limits[j].0, limits[j].1);
            }
        }
        // Carry what was found to every sample, straight between the points it was found at.
        for j in 0..points {
            let (from, to) = (index(j), index(j + 1));
            let span = (to + count - from) % count;
            let span = if span == 0 { count } else { span };
            for step in 0..span {
                let share = step as f32 / span as f32;
                offsets[(from + step) % count] =
                    across[j] + (across[(j + 1) % points] - across[j]) * share;
            }
        }
    }
    offsets
}

/// How sharply a line turns at each of its points, per metre: above zero to the left.
pub fn turning(points: &[Vec2]) -> Vec<f32> {
    let count = points.len();
    (0..count)
        .map(|i| {
            let (a, b, c) = (
                points[(i + count - 1) % count],
                points[i],
                points[(i + 1) % count],
            );
            let (before, after) = (b - a, c - b);
            let step = (before.length() + after.length()) * 0.5;
            // With y up and z towards the viewer, a turn to the left is clockwise in (x, z).
            -before.perp_dot(after).atan2(before.dot(after)) / step.max(1.0e-4)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;
    use solstice_game::track::Sample;

    /// A track through points on the ground, 12 m wide, with nothing beside it.
    fn track(points: &[(f32, f32)]) -> Track {
        let count = points.len();
        let samples = (0..count)
            .map(|i| {
                let (before, after) = (points[(i + count - 1) % count], points[(i + 1) % count]);
                let forward = Vec3::new(after.0 - before.0, 0.0, after.1 - before.1).normalize();
                Sample {
                    position: Vec3::new(points[i].0, 0.0, points[i].1),
                    left: Vec3::Y.cross(forward),
                    half_left: 6.0,
                    half_right: 6.0,
                    left_bands: Vec::new(),
                    right_bands: Vec::new(),
                }
            })
            .collect();
        Track::new("Test", samples, vec![count / 3, 2 * count / 3]).unwrap()
    }

    /// A square with rounded corners, 300 m a side, driven clockwise seen from above: every
    /// corner is a right-hander.
    fn square() -> Vec<(f32, f32)> {
        let (side, radius) = (300.0_f32, 40.0_f32);
        let mut points = Vec::new();
        let straight = ((side - 2.0 * radius) / 3.0) as usize;
        let arc = (radius * std::f32::consts::FRAC_PI_2 / 3.0) as usize;
        // Each side in turn: north along the west side, east along the north, and so on.
        let corners = [
            (Vec2::new(0.0, side), Vec2::new(0.0, -1.0)),
            (Vec2::new(0.0, 0.0), Vec2::new(1.0, 0.0)),
            (Vec2::new(side, 0.0), Vec2::new(0.0, 1.0)),
            (Vec2::new(side, side), Vec2::new(-1.0, 0.0)),
        ];
        for (start, along) in corners {
            let right = Vec2::new(-along.y, along.x);
            for k in 0..straight {
                let p = start + along * (radius + k as f32 * 3.0);
                points.push((p.x, p.y));
            }
            let middle = start + along * (side - radius) + right * radius;
            for k in 0..arc {
                let a = k as f32 / arc as f32 * std::f32::consts::FRAC_PI_2;
                let p = middle - right * radius * a.cos() + along * radius * a.sin();
                points.push((p.x, p.y));
            }
        }
        points
    }

    #[test]
    fn on_a_ring_the_line_keeps_to_one_place_across_the_road() {
        let ring: Vec<(f32, f32)> = (0..300)
            .map(|k| {
                let a = k as f32 / 300.0 * std::f32::consts::TAU;
                (a.cos() * 150.0, a.sin() * 150.0)
            })
            .collect();
        let line = racing_line(&track(&ring));
        let (low, high) = line
            .iter()
            .fold((f32::MAX, f32::MIN), |(l, h), &o| (l.min(o), h.max(o)));
        assert!(high - low < 0.2, "from {low} to {high}");
    }

    #[test]
    fn the_line_runs_wide_into_a_corner_and_clips_its_inside() {
        let points = square();
        let track = track(&points);
        let line = racing_line(&track);
        let flat: Vec<Vec2> = points.iter().map(|p| Vec2::new(p.0, p.1)).collect();
        let turn = turning(&flat);
        // Every corner is to the right: at the middle of each the line is hard to the right,
        // and half-way along each straight it is well to the left.
        let apex = (0..points.len())
            .filter(|&i| turn[i] < -0.02)
            .map(|i| line[i])
            .fold(f32::MAX, f32::min);
        assert!(apex < -4.0, "at the apex: {apex}");
        let straights: Vec<f32> = (0..points.len())
            .filter(|&i| {
                (0..40).all(|k| turn[(i + k + points.len() - 20) % points.len()].abs() < 0.001)
            })
            .map(|i| line[i])
            .collect();
        let widest = straights.iter().copied().fold(f32::MIN, f32::max);
        assert!(widest > 3.0, "on the straights: {widest}");
        // And it never leaves the road.
        assert!(line.iter().all(|o| o.abs() <= 6.0 - MARGIN + 1.0e-3));
        // The line bends less than the road's middle does.
        let driven: Vec<Vec2> = (0..points.len())
            .map(|i| {
                let s = &track.samples[i];
                Vec2::new(s.position.x, s.position.z) + Vec2::new(s.left.x, s.left.z) * line[i]
            })
            .collect();
        // Round a corner it is a wider arc than the road's middle, and all told it bends
        // far less.
        let corner = |turn: &[f32]| {
            let mut sharp: Vec<f32> = turn.iter().map(|t| t.abs()).collect();
            sharp.sort_by(f32::total_cmp);
            sharp[sharp.len() * 9 / 10]
        };
        let bending = |turn: &[f32]| turn.iter().map(|t| t * t).sum::<f32>();
        let line_turn = turning(&driven);
        assert!(
            corner(&line_turn) < corner(&turn) * 0.85,
            "{} against {}",
            corner(&line_turn),
            corner(&turn)
        );
        assert!(bending(&line_turn) < bending(&turn) * 0.75);
    }
}
