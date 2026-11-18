//! How long a track takes: the time a good driver gets round it in, worked out from the track
//! itself, so that a track that is changed, or a new one, has times to beat without anybody
//! driving it first.
//!
//! It goes along the centre line a couple of metres at a time and asks how fast each place can
//! be taken, as [`crate::drive::autopilot`] asks of the road ahead of it: by how tight the bend
//! is, by what is underfoot, by whether the track drops away and comes back, by how steep it
//! is, and no faster than the truck goes. Then it works back from every slow place by how hard
//! the truck brakes, and forward by how hard it pulls away, and adds up the time.

use crate::drive::Pace;
use crate::track::Track;
use glam::Vec3;

/// How a truck of some kind gets round a track.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Going {
    /// How hard it corners and brakes, and how fast it will go.
    pub pace: Pace,
    /// How hard it pulls away, in metres a second squared.
    pub pull: f32,
    /// How much faster than a car that drives itself it crosses what is crossed at a crawl: a
    /// ford, a bog, a step.
    pub crawl: f32,
}

/// How far apart the places asked about are, in metres, or a little less: a whole number of
/// them go round the lap.
const STEP: f32 = 2.0;
/// How far behind the line a timed run starts from rest, in metres.
pub const RUN_UP: f32 = 6.0;
/// How much of its top speed a truck loses for each metre it climbs or drops in a metre, and
/// the least of it that is left however steep.
const STEEP: f32 = 2.5;
const STEEPEST: f32 = 0.2;

/// A track as one kind of truck drives it: how fast at each place, and how long to there.
#[derive(Clone, Debug, PartialEq)]
pub struct Profile {
    /// How far apart its places are, and how long the lap is, in metres.
    step: f32,
    length: f32,
    /// How fast each place is taken, in metres a second.
    speeds: Vec<f32>,
    /// How long it takes to get to each place from the line, in seconds, and, last, round to
    /// the line again.
    times: Vec<f32>,
}

impl Profile {
    /// Works a track out for a truck. `caution` says how fast a place may be crossed by a car
    /// that drives itself, as the autopilot is told: a ford or a bog at a crawl.
    ///
    /// The lap is from rest, [`RUN_UP`] behind the line, and is not slowed for the line at its
    /// end: a timed run.
    pub fn of(track: &Track, going: &Going, caution: Option<&dyn Fn(Vec3) -> f32>) -> Self {
        let length = track.length();
        let count = ((length / STEP).ceil() as usize).max(3);
        let step = length / count as f32;
        let pace = &going.pace;
        let flat = |v: Vec3| Vec3::new(v.x, 0.0, v.z);
        let mut speeds: Vec<f32> = (0..count)
            .map(|index| {
                let along = index as f32 * step;
                let at = |d: f32| track.place(along + d).0;
                let mut limit = pace.top_speed;
                // The bend: the circle through three points of the road, twelve metres of it.
                let (a, b, c) = (flat(at(-6.0)), flat(at(0.0)), flat(at(6.0)));
                let bend = 2.0 * (b - a).cross(c - b).length()
                    / ((b - a).length() * (c - b).length() * (c - a).length()).max(1.0e-6);
                limit = limit.min((pace.cornering / bend.max(1.0e-4)).sqrt());
                // A track that drops away and comes back, as at a washout or a step.
                let (before, here, after) = (at(-3.0), at(0.0), at(3.0));
                let dip = (before.y + after.y - 2.0 * here.y).abs();
                if dip > 0.22 {
                    limit = limit.min(2.2 * going.crawl);
                } else if dip > 0.1 {
                    limit = limit.min(4.0 * going.crawl);
                }
                // What is underfoot.
                if let Some(caution) = caution {
                    limit = limit.min(caution(here) * going.crawl);
                }
                // And how steep it is, up or down.
                let grade = (at(6.0).y - at(-6.0).y).abs() / 12.0;
                limit.min(pace.top_speed * (1.0 - STEEP * grade).max(STEEPEST))
            })
            .collect();
        // Back from every slow place, by how hard it brakes: twice round, so that what is
        // just past the line slows what comes up to it.
        for index in (0..2 * count).rev() {
            let next = speeds[(index + 1) % count];
            let here = &mut speeds[index % count];
            *here = here.min((next * next + 2.0 * pace.braking * step).sqrt());
        }
        // Forward from the start, by how hard it pulls away.
        speeds[0] = speeds[0].min((2.0 * going.pull * RUN_UP).sqrt());
        for index in 1..count {
            let before = speeds[index - 1];
            speeds[index] = speeds[index].min((before * before + 2.0 * going.pull * step).sqrt());
        }
        let mut times = Vec::with_capacity(count + 1);
        let mut seconds = 0.0;
        for index in 0..count {
            times.push(seconds);
            let next = speeds[(index + 1).min(count - 1)];
            seconds += step / ((speeds[index] + next) * 0.5).max(0.3);
        }
        times.push(seconds);
        Self {
            step,
            length,
            speeds,
            times,
        }
    }

    /// How long the lap takes, in seconds.
    pub fn seconds(&self) -> f32 {
        self.times[self.times.len() - 1]
    }

    /// How long the lap is, in metres.
    pub fn length(&self) -> f32 {
        self.length
    }

    /// How long it takes to get this far round from the line, in seconds.
    pub fn time_at(&self, along: f32) -> f32 {
        let place = (along.clamp(0.0, self.length) / self.step).min(self.speeds.len() as f32);
        let index = (place.floor() as usize).min(self.speeds.len() - 1);
        let share = place - index as f32;
        self.times[index] + (self.times[index + 1] - self.times[index]) * share
    }

    /// How far round from the line it has got after this long, in metres: no further than the
    /// line again.
    pub fn along_at(&self, seconds: f32) -> f32 {
        if seconds <= 0.0 {
            return 0.0;
        }
        let after = self.times.partition_point(|&time| time <= seconds);
        if after >= self.times.len() {
            return self.length;
        }
        let (from, to) = (self.times[after - 1], self.times[after]);
        let share = (seconds - from) / (to - from).max(1.0e-6);
        (((after - 1) as f32 + share) * self.step).min(self.length)
    }

    /// How fast this place is taken, in metres a second.
    pub fn speed_at(&self, along: f32) -> f32 {
        let index = (along.rem_euclid(self.length) / self.step) as usize;
        self.speeds[index.min(self.speeds.len() - 1)]
    }

    /// How much of the lap's length is taken slower than this, from 0 to 1.
    pub fn share_below(&self, speed: f32) -> f32 {
        let slow = self.speeds.iter().filter(|&&at| at < speed).count();
        slow as f32 / self.speeds.len() as f32
    }

    /// The slowest places on the track, where time is won and lost: the slowest of each of
    /// `count` equal stretches of the lap, so that they are from all round it, no two within
    /// `apart` metres of each other or of the line, in the order they come.
    pub fn slowest(&self, count: usize, apart: f32) -> Vec<f32> {
        let places = self.speeds.len();
        let mut found: Vec<f32> = Vec::new();
        for stretch in 0..count {
            let (from, to) = (stretch * places / count, (stretch + 1) * places / count);
            let slowest = (from..to)
                .filter(|&index| {
                    let along = index as f32 * self.step;
                    let clear = |other: f32| {
                        let gap = (along - other).abs();
                        gap.min(self.length - gap) >= apart
                    };
                    clear(0.0) && found.iter().all(|&other| clear(other))
                })
                .min_by(|&a, &b| self.speeds[a].total_cmp(&self.speeds[b]));
            if let Some(index) = slowest {
                found.push(index as f32 * self.step);
            }
        }
        found
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::track::tests::oval;

    fn going(scale: f32) -> Going {
        Going {
            pace: Pace {
                cornering: 3.0 * scale,
                braking: 3.5 * scale,
                top_speed: 14.0 * scale,
                looks: Pace::TOURING.looks,
            },
            pull: 1.5 * scale,
            crawl: 1.0,
        }
    }

    #[test]
    fn a_lap_takes_longer_than_flat_out_and_a_better_truck_is_quicker() {
        let track = oval();
        let slow = Profile::of(&track, &going(1.0), None);
        let quick = Profile::of(&track, &going(1.5), None);
        let flat_out = track.length() / 14.0;
        assert!(
            slow.seconds() > flat_out && slow.seconds() < flat_out * 2.0,
            "{} against {flat_out}",
            slow.seconds()
        );
        assert!(quick.seconds() < slow.seconds() * 0.85);
        // The oval's bends are fifty metres round: taken at what the truck corners at, and the
        // straights faster.
        let bend = (3.0_f32 * 50.0).sqrt();
        assert!(
            (slow.speed_at(190.0) - bend).abs() < 0.8,
            "{}",
            slow.speed_at(190.0)
        );
        assert!(slow.speed_at(420.0) > bend + 1.0);
        // It starts from rest behind the line, and is still getting going over it.
        assert!(slow.speed_at(0.0) < 5.0);
    }

    #[test]
    fn what_is_underfoot_slows_it_and_is_braked_for() {
        let track = oval();
        let dry = Profile::of(&track, &going(1.0), None);
        // A ford across the far straight, twenty metres of it.
        let ford = |at: Vec3| {
            if at.x > 90.0 && at.z.abs() < 10.0 {
                2.6
            } else {
                f32::INFINITY
            }
        };
        let wet = Profile::of(&track, &going(1.0), Some(&ford));
        assert!(wet.seconds() > dry.seconds() + 4.0);
        let slowest = wet.slowest(1, 60.0);
        assert_eq!(slowest.len(), 1);
        assert!(
            (wet.speed_at(slowest[0]) - 2.6).abs() < 0.01,
            "the ford is the slowest place: {slowest:?}"
        );
        // Ten metres before it the truck is already slowing for it.
        assert!(wet.speed_at(slowest[0] - 10.0) < dry.speed_at(slowest[0] - 10.0) - 1.0);
        // A truck that crawls faster loses less to it.
        let keen = Going {
            crawl: 1.5,
            ..going(1.0)
        };
        assert!(Profile::of(&track, &keen, Some(&ford)).seconds() < wet.seconds() - 1.0);
    }

    #[test]
    fn where_it_is_and_when_agree() {
        let track = oval();
        let profile = Profile::of(&track, &going(1.0), None);
        assert_eq!(profile.time_at(0.0), 0.0);
        assert!((profile.time_at(track.length()) - profile.seconds()).abs() < 1.0e-3);
        let mut last = -1.0;
        for tenth in 0..=10 {
            let along = track.length() * tenth as f32 / 10.0;
            let seconds = profile.time_at(along);
            assert!(seconds > last, "time goes on round the lap");
            last = seconds;
            assert!(
                (profile.along_at(seconds) - along).abs() < 0.05,
                "{along} m is reached at {seconds} s"
            );
        }
        assert_eq!(profile.along_at(-3.0), 0.0);
        assert_eq!(profile.along_at(profile.seconds() + 60.0), track.length());
    }

    #[test]
    fn the_slowest_places_are_apart_and_in_order() {
        let track = oval();
        let profile = Profile::of(&track, &going(1.0), None);
        let places = profile.slowest(3, 80.0);
        assert!(places.len() >= 2, "{places:?}");
        assert!(places.windows(2).all(|pair| pair[1] - pair[0] >= 80.0));
        assert!(places.iter().all(|&along| along >= 80.0));
    }
}
