//! Weather: what the sky is doing, as plain data with a text form.
//!
//! A weather state is written one setting per line, like a tree species or a breed of grass:
//!
//! ```text
//! # Fair weather: small, separate cumulus on a clear day.
//! coverage 0.3
//! type 0.9
//! base 1200
//! thickness 1600
//! density 1
//! cirrus 0.15
//! wind speed=7 from=250 gusts=0.45 turbulence=0.45
//! evolution 1
//! rain 0
//! ```
//!
//! Every setting has a default, so a file names only what differs. The built-in states are
//! files of exactly this kind, in `weather/` beside this crate. [`Weather::observed`] turns a
//! live report of cloud cover and wind into a state, so the sky can follow real weather.

use crate::text::number;

/// What the sky is doing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Weather {
    /// Share of the sky the main layer of cloud covers, `0..1`.
    pub coverage: f32,
    /// What kind of cloud: 0 is flat sheets of stratus, 0.5 lumpy stratocumulus, 1 tall heaped
    /// cumulus.
    pub cloud_type: f32,
    /// Height of the cloud base above the ground, in metres.
    pub base: f32,
    /// Depth of the cloud layer, in metres. Towering storm clouds are many kilometres deep.
    pub thickness: f32,
    /// How thick the cloud is inside: 1 for ordinary cloud, more for rain-bearing cloud.
    pub density: f32,
    /// Share of the sky covered by high, thin cirrus, `0..1`.
    pub cirrus: f32,
    /// Wind at cloud height, in metres per second.
    pub wind_speed: f32,
    /// Compass direction the wind blows from, in degrees: 270 is a westerly.
    pub wind_from: f32,
    /// How gusty the wind is near the ground, `0..1`: 0 blows steadily, 1 in strong gusts and
    /// lulls (docs/weather-effects.md).
    pub gusts: f32,
    /// How much the wind's direction wanders from place to place, `0..1`.
    pub turbulence: f32,
    /// How fast clouds grow and fade as they drift, relative to ordinary weather.
    pub evolution: f32,
    /// How heavily it rains, `0..1`: darkens cloud bases and greys the air.
    pub rain: f32,
    /// How often lightning strikes, in strikes a minute: 0 for none (`lightning.rs`).
    pub thunder: f32,
}

/// The built-in states, by name, as text.
const BUILT_IN: [(&str, &str); 7] = [
    ("clear", include_str!("../../weather/clear.weather")),
    ("calm", include_str!("../../weather/calm.weather")),
    ("fair", include_str!("../../weather/fair.weather")),
    ("broken", include_str!("../../weather/broken.weather")),
    ("overcast", include_str!("../../weather/overcast.weather")),
    ("storm", include_str!("../../weather/storm.weather")),
    ("cirrus", include_str!("../../weather/cirrus.weather")),
];

impl Default for Weather {
    /// Fair weather.
    fn default() -> Self {
        Self::by_name("fair").unwrap()
    }
}

/// A live weather report, as weather services give it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Observation {
    /// Cloud cover below about 2 km, between about 2 and 6 km, and above, each in percent.
    pub cover_low: f32,
    pub cover_mid: f32,
    pub cover_high: f32,
    /// Wind near the ground, in kilometres per hour, and the direction it blows from in degrees.
    pub wind_kmh: f32,
    pub wind_from: f32,
    /// WMO weather code: 0 clear, 1 to 3 more and more cloud, 45 fog, 51 to 67 drizzle and
    /// rain, 71 to 86 snow and showers, 95 to 99 thunderstorm.
    pub code: u32,
    /// Precipitation in the last hour, in millimetres.
    pub precipitation: f32,
}

impl Weather {
    /// Names of the built-in states.
    pub fn names() -> impl Iterator<Item = &'static str> {
        BUILT_IN.iter().map(|(name, _)| *name)
    }

    /// A built-in state, or `None` if there is none of that name.
    pub fn by_name(name: &str) -> Option<Self> {
        let (_, text) = BUILT_IN.iter().find(|(n, _)| *n == name)?;
        Some(Self::parse(text).unwrap_or_else(|e| panic!("built-in weather {name}: {e}")))
    }

    fn base_state() -> Self {
        Self {
            coverage: 0.3,
            cloud_type: 0.9,
            base: 1200.0,
            thickness: 1600.0,
            density: 1.0,
            cirrus: 0.15,
            wind_speed: 7.0,
            wind_from: 250.0,
            gusts: 0.4,
            turbulence: 0.4,
            evolution: 1.0,
            rain: 0.0,
            thunder: 0.0,
        }
    }

    /// Reads a state from its text form. Errors name the line at fault.
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut weather = Self::base_state();
        for (index, raw) in text.lines().enumerate() {
            let words: Vec<&str> = raw
                .split('#')
                .next()
                .unwrap_or("")
                .split_whitespace()
                .collect();
            let Some((&keyword, rest)) = words.split_first() else {
                continue;
            };
            let at = |e: String| format!("line {}: {e}", index + 1);
            let single = || match rest {
                [value] => number(value).map_err(at),
                _ => Err(at(format!("{keyword} takes one number"))),
            };
            match keyword {
                "coverage" => weather.coverage = single()?.clamp(0.0, 1.0),
                "type" => weather.cloud_type = single()?.clamp(0.0, 1.0),
                "base" => weather.base = single()?.max(0.0),
                "thickness" => weather.thickness = single()?.max(100.0),
                "density" => weather.density = single()?.max(0.0),
                "cirrus" => weather.cirrus = single()?.clamp(0.0, 1.0),
                "evolution" => weather.evolution = single()?.max(0.0),
                "rain" => weather.rain = single()?.clamp(0.0, 1.0),
                "thunder" => weather.thunder = single()?.max(0.0),
                "wind" => {
                    for word in rest {
                        let (key, value) = word
                            .split_once('=')
                            .ok_or_else(|| at(format!("expected key=value, found {word:?}")))?;
                        match key {
                            "speed" => weather.wind_speed = number(value).map_err(at)?.max(0.0),
                            "from" => weather.wind_from = number(value).map_err(at)?,
                            "gusts" => weather.gusts = number(value).map_err(at)?.clamp(0.0, 1.0),
                            "turbulence" => {
                                weather.turbulence = number(value).map_err(at)?.clamp(0.0, 1.0)
                            }
                            _ => return Err(at(format!("unknown wind setting {key}"))),
                        }
                    }
                }
                _ => return Err(at(format!("unknown setting {keyword:?}"))),
            }
        }
        Ok(weather)
    }

    /// Writes this state in the form [`Weather::parse`] reads.
    pub fn to_text(&self) -> String {
        format!(
            "coverage {}\ntype {}\nbase {}\nthickness {}\ndensity {}\ncirrus {}\nwind speed={} from={} gusts={} turbulence={}\nevolution {}\nrain {}\nthunder {}\n",
            self.coverage,
            self.cloud_type,
            self.base,
            self.thickness,
            self.density,
            self.cirrus,
            self.wind_speed,
            self.wind_from,
            self.gusts,
            self.turbulence,
            self.evolution,
            self.rain,
            self.thunder,
        )
    }

    /// Part way from this state to `to`, by `t` in `0..1`. The wind turns the short way round.
    pub fn lerp(&self, to: &Self, t: f32) -> Self {
        let mix = |a: f32, b: f32| a + (b - a) * t;
        let turn = (to.wind_from - self.wind_from + 540.0).rem_euclid(360.0) - 180.0;
        Self {
            coverage: mix(self.coverage, to.coverage),
            cloud_type: mix(self.cloud_type, to.cloud_type),
            base: mix(self.base, to.base),
            thickness: mix(self.thickness, to.thickness),
            density: mix(self.density, to.density),
            cirrus: mix(self.cirrus, to.cirrus),
            wind_speed: mix(self.wind_speed, to.wind_speed),
            gusts: mix(self.gusts, to.gusts),
            turbulence: mix(self.turbulence, to.turbulence),
            wind_from: (self.wind_from + turn * t).rem_euclid(360.0),
            evolution: mix(self.evolution, to.evolution),
            rain: mix(self.rain, to.rain),
            thunder: mix(self.thunder, to.thunder),
        }
    }

    /// The wind's velocity over the ground, in metres per second, as (east, south): the
    /// scene's +X and +Z. Wind from the west (270) blows towards +X.
    pub fn wind_velocity(&self) -> [f32; 2] {
        let towards = (self.wind_from + 180.0).to_radians();
        // Compass bearings run clockwise from north (-Z): east is +X, south is +Z.
        [
            towards.sin() * self.wind_speed,
            -towards.cos() * self.wind_speed,
        ]
    }

    /// A state that matches a live report. Low and middle cloud make up the main layer, whose
    /// kind follows the report's weather code; high cloud becomes cirrus.
    pub fn observed(report: &Observation) -> Self {
        let low = (report.cover_low / 100.0).clamp(0.0, 1.0);
        let mid = (report.cover_mid / 100.0).clamp(0.0, 1.0);
        let high = (report.cover_high / 100.0).clamp(0.0, 1.0);
        let code = report.code;
        let storm = (95..=99).contains(&code);
        let showers = (80..=86).contains(&code);
        let rain_code = (51..=67).contains(&code) || (71..=77).contains(&code) || showers;
        // Meteorologists' cover counts sky that is only thinly clouded, and the eye counts
        // gaps between clouds, so the visible coverage is a little less than reported.
        let coverage = (low.max(mid * 0.85) * 0.9).clamp(0.0, 1.0);
        // Mostly low cloud covering the sky is a sheet; broken low cloud is heaped cumulus;
        // middle cloud alone is altocumulus, lumpy but flatter.
        let mut cloud_type = if low >= mid {
            0.95 - 0.75 * smooth(0.6, 0.95, low)
        } else {
            0.55 - 0.25 * smooth(0.6, 0.95, mid)
        };
        let mut base: f32 = if low >= mid { 1000.0 } else { 3000.0 };
        let mut thickness = if low >= mid { 1500.0 } else { 1200.0 };
        let mut density: f32 = 1.0;
        if showers {
            cloud_type = cloud_type.max(0.85);
            thickness = 3500.0;
            density = 1.4;
        }
        if storm {
            cloud_type = 1.0;
            base = 900.0;
            thickness = 7000.0;
            density = 2.0;
        }
        if rain_code {
            base = base.min(800.0);
            density = density.max(1.5);
        }
        let rain = if storm {
            1.0
        } else if rain_code {
            (0.3 + report.precipitation / 4.0).min(1.0)
        } else {
            0.0
        };
        Self {
            coverage: if code == 0 {
                coverage.min(0.05)
            } else {
                coverage
            },
            cloud_type,
            base,
            thickness,
            density,
            cirrus: high,
            // Wind at cloud height is about twice that near the ground.
            wind_speed: report.wind_kmh / 3.6 * 2.0,
            wind_from: report.wind_from,
            // Stronger winds and showers are gustier; storms most of all.
            gusts: if storm {
                0.85
            } else {
                (0.3 + report.wind_kmh / 80.0 + if showers { 0.2 } else { 0.0 }).min(1.0)
            },
            turbulence: if storm { 0.7 } else { 0.4 },
            evolution: 1.0 + rain,
            rain,
            // Thunderstorms: 95 is slight or moderate, 96 to 99 heavy, with hail.
            thunder: match code {
                95 => 2.0,
                96..=99 => 4.0,
                _ => 0.0,
            },
        }
    }
}

/// A sky that changes by itself. After a few minutes in one weather it moves on to one that
/// could follow it, as weather does: fair cumulus grows into broken cloud, which may close into
/// overcast and break into a storm, then clear again. It never leaps from clear to storm.
#[derive(Clone, Debug)]
pub struct WeatherCycle {
    current: &'static str,
    remaining: f32,
    state: u32,
}

/// Which weather can follow which.
const NEXT: [(&str, [&str; 2]); 6] = [
    ("clear", ["fair", "cirrus"]),
    ("cirrus", ["fair", "broken"]),
    ("fair", ["broken", "clear"]),
    ("broken", ["overcast", "fair"]),
    ("overcast", ["storm", "broken"]),
    ("storm", ["broken", "overcast"]),
];

impl WeatherCycle {
    /// Seconds spent in one weather before moving on, at least and at most.
    pub const DWELL: (f32, f32) = (120.0, 300.0);
    /// Seconds over which each change eases in.
    pub const TRANSITION: f32 = 90.0;

    /// A cycle starting in the named weather, or in fair weather if there is none of that name.
    pub fn new(start: &str, seed: u32) -> Self {
        let current = NEXT
            .iter()
            .map(|(name, _)| *name)
            .find(|name| *name == start)
            .unwrap_or("fair");
        let mut cycle = Self {
            current,
            remaining: 0.0,
            state: seed.wrapping_mul(747_796_405).wrapping_add(2_891_336_453),
        };
        cycle.remaining = cycle.dwell();
        cycle
    }

    /// The weather the cycle is in, or moving to.
    pub fn current(&self) -> &'static str {
        self.current
    }

    fn random(&mut self) -> f32 {
        self.state = self
            .state
            .wrapping_mul(747_796_405)
            .wrapping_add(2_891_336_453);
        let word =
            ((self.state >> ((self.state >> 28) + 4)) ^ self.state).wrapping_mul(277_803_737);
        ((word >> 22) ^ word) as f32 / u32::MAX as f32
    }

    fn dwell(&mut self) -> f32 {
        Self::DWELL.0 + (Self::DWELL.1 - Self::DWELL.0) * self.random()
    }

    /// Moves time on by `seconds`. Returns the next weather when it is time to change.
    pub fn advance(&mut self, seconds: f32) -> Option<&'static str> {
        self.remaining -= seconds;
        if self.remaining > 0.0 {
            return None;
        }
        let (_, options) = NEXT.iter().find(|(name, _)| *name == self.current)?;
        self.current = options[(self.random() * 1.999) as usize];
        self.remaining = self.dwell();
        Some(self.current)
    }
}

fn smooth(low: f32, high: f32, x: f32) -> f32 {
    let t = ((x - low) / (high - low)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_built_in_state_reads_and_writes_back_unchanged() {
        for name in Weather::names() {
            let weather = Weather::by_name(name).unwrap();
            assert_eq!(Weather::parse(&weather.to_text()), Ok(weather), "{name}");
        }
        assert!(Weather::by_name("hail").is_none());
    }

    #[test]
    fn states_differ_as_their_names_say() {
        let get = |name| Weather::by_name(name).unwrap();
        assert!(get("clear").coverage < 0.05);
        assert!(get("overcast").coverage > 0.9 && get("overcast").cloud_type < 0.3);
        assert!(get("storm").thickness > 5000.0 && get("storm").rain > 0.5);
        assert!(get("cirrus").cirrus > get("fair").cirrus && get("cirrus").coverage < 0.1);
        assert!(get("broken").coverage > get("fair").coverage);
    }

    #[test]
    fn mistakes_are_reported_with_their_line() {
        let error = |text: &str| Weather::parse(text).unwrap_err();
        assert!(error("coverage 0.5\ncovrage 0.2\n").starts_with("line 2:"));
        assert!(error("wind sped=3\n").contains("sped"));
        assert!(error("base high\n").contains("high"));
    }

    #[test]
    fn blending_eases_between_states_and_turns_the_short_way() {
        let a = Weather {
            wind_from: 350.0,
            ..Weather::by_name("clear").unwrap()
        };
        let b = Weather {
            wind_from: 10.0,
            ..Weather::by_name("overcast").unwrap()
        };
        assert_eq!(a.lerp(&b, 0.0), a);
        let half = a.lerp(&b, 0.5);
        assert!((half.coverage - (a.coverage + b.coverage) / 2.0).abs() < 1.0e-5);
        assert!(half.wind_from.abs() < 1.0e-3 || (half.wind_from - 360.0).abs() < 1.0e-3);
    }

    #[test]
    fn a_changing_sky_moves_through_neighbouring_weather() {
        let mut cycle = WeatherCycle::new("clear", 7);
        let mut visited = vec![cycle.current()];
        let mut since_change = 0.0;
        for _ in 0..(3 * 60 * 60) {
            since_change += 1.0;
            if let Some(next) = cycle.advance(1.0) {
                let previous = *visited.last().unwrap();
                let (_, options) = NEXT.iter().find(|(name, _)| *name == previous).unwrap();
                assert!(options.contains(&next), "{previous} jumped to {next}");
                assert!(
                    since_change >= WeatherCycle::DWELL.0,
                    "changed after {since_change} s"
                );
                assert!(Weather::by_name(next).is_some());
                visited.push(next);
                since_change = 0.0;
            }
        }
        // Three hours sees every kind of weather.
        for (name, _) in NEXT {
            assert!(visited.contains(&name), "never reached {name}: {visited:?}");
        }
    }

    #[test]
    fn westerly_wind_blows_towards_the_east() {
        let weather = Weather {
            wind_speed: 10.0,
            wind_from: 270.0,
            ..Weather::default()
        };
        let [east, south] = weather.wind_velocity();
        assert!((east - 10.0).abs() < 1.0e-4 && south.abs() < 1.0e-4);
    }

    #[test]
    fn live_reports_become_matching_weather() {
        let clear = Weather::observed(&Observation {
            code: 0,
            ..Default::default()
        });
        assert!(clear.coverage < 0.06 && clear.rain == 0.0);

        let overcast = Weather::observed(&Observation {
            cover_low: 100.0,
            code: 3,
            ..Default::default()
        });
        assert!(overcast.coverage > 0.85 && overcast.cloud_type < 0.3);

        let cumulus = Weather::observed(&Observation {
            cover_low: 40.0,
            code: 2,
            wind_kmh: 18.0,
            wind_from: 270.0,
            ..Default::default()
        });
        assert!(cumulus.cloud_type > 0.8 && cumulus.wind_speed > 9.0);

        let storm = Weather::observed(&Observation {
            cover_low: 80.0,
            cover_high: 60.0,
            code: 95,
            ..Default::default()
        });
        assert!(storm.thickness > 5000.0 && storm.rain == 1.0 && storm.cirrus > 0.5);
    }
}
