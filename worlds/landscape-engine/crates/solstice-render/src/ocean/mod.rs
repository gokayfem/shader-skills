//! The ocean: what the water is like, as plain data with a text form, and the waves on it.
//!
//! An ocean is written one setting per line, like the weather:
//!
//! ```text
//! # Coastal: green-blue water over a sandy shelf.
//! absorption 0.40 0.09 0.07
//! scattering 0.010 0.018 0.016
//! turbidity 0.2
//! foam 1
//! choppiness 0.85
//! swell height=0.6 period=10 from=100
//! fetch 30
//! ```
//!
//! Every setting has a default, so a file names only what differs. The built-in oceans are files
//! of exactly this kind, in `ocean/` beside this crate. The wind that raises the waves is not part
//! of the ocean: it comes from the weather, so the sea answers to the sky.
//!
//! See `docs/ocean.md` for the design: [`spectrum`] says how much wave there is at each
//! wavelength, and [`waves`] turns that into moving waves on the GPU.

pub mod grid;
pub mod pond;
pub mod sea;
pub mod seabed;
pub mod spectrum;
pub mod surface;
pub mod waterline;
pub mod waves;

use crate::text::number;

pub use pond::Pond;
pub use sea::Sea;
pub use spectrum::SeaState;
pub use waves::OceanWaves;

/// A long, regular swell, raised by distant storms, which can come from a different direction
/// from the local wind.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Swell {
    /// Significant wave height, in metres: the mean height of the highest third of waves.
    pub height: f32,
    /// Seconds from one crest to the next.
    pub period: f32,
    /// Compass direction the swell comes from, in degrees.
    pub from: f32,
}

/// What the water is like: its colour and clarity, how readily it breaks into foam, and the
/// swell running through it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ocean {
    /// How strongly the water absorbs red, green and blue light, per metre. Red goes first, so
    /// deep water turns blue-green.
    pub absorption: [f32; 3],
    /// How strongly the water scatters red, green and blue light back out, per metre: its own
    /// glow, from particles and plankton.
    pub scattering: [f32; 3],
    /// How cloudy the water is, `0..1`, from clear to silty.
    pub turbidity: f32,
    /// How readily breaking waves turn to foam: 0 none, 1 as a real sea, more for a wilder one.
    pub foam: f32,
    /// How much crests sharpen and troughs widen, `0..1`. Zero gives smooth, rounded waves.
    pub choppiness: f32,
    pub swell: Swell,
    /// Kilometres of open water the wind blows across. Short reaches raise only small, steep
    /// waves; hundreds of kilometres let a fully developed sea grow.
    pub fetch: f32,
}

/// The built-in oceans, by name, as text.
const BUILT_IN: [(&str, &str); 4] = [
    ("coastal", include_str!("../../ocean/coastal.ocean")),
    ("clear", include_str!("../../ocean/clear.ocean")),
    ("ocean", include_str!("../../ocean/ocean.ocean")),
    ("murky", include_str!("../../ocean/murky.ocean")),
];

impl Default for Ocean {
    /// Coastal water.
    fn default() -> Self {
        Self::by_name("coastal").unwrap()
    }
}

impl Ocean {
    /// Names of the built-in oceans.
    pub fn names() -> impl Iterator<Item = &'static str> {
        BUILT_IN.iter().map(|(name, _)| *name)
    }

    /// A built-in ocean, or `None` if there is none of that name.
    pub fn by_name(name: &str) -> Option<Self> {
        let (_, text) = BUILT_IN.iter().find(|(n, _)| *n == name)?;
        Some(Self::parse(text).unwrap_or_else(|e| panic!("built-in ocean {name}: {e}")))
    }

    fn base_state() -> Self {
        Self {
            absorption: [0.40, 0.09, 0.07],
            scattering: [0.010, 0.018, 0.016],
            turbidity: 0.2,
            foam: 1.0,
            choppiness: 0.85,
            swell: Swell {
                height: 0.6,
                period: 10.0,
                from: 100.0,
            },
            fetch: 30.0,
        }
    }

    /// Reads an ocean from its text form. Errors name the line at fault.
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut ocean = Self::base_state();
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
            let colour = || match rest {
                [r, g, b] => Ok([
                    number(r).map_err(at)?.max(0.0),
                    number(g).map_err(at)?.max(0.0),
                    number(b).map_err(at)?.max(0.0),
                ]),
                _ => Err(at(format!(
                    "{keyword} takes three numbers: red, green and blue"
                ))),
            };
            match keyword {
                "absorption" => ocean.absorption = colour()?,
                "scattering" => ocean.scattering = colour()?,
                "turbidity" => ocean.turbidity = single()?.clamp(0.0, 1.0),
                "foam" => ocean.foam = single()?.max(0.0),
                "choppiness" => ocean.choppiness = single()?.clamp(0.0, 1.0),
                "fetch" => ocean.fetch = single()?.max(0.1),
                "swell" => {
                    for word in rest {
                        let (key, value) = word
                            .split_once('=')
                            .ok_or_else(|| at(format!("expected key=value, found {word:?}")))?;
                        let value = number(value).map_err(at)?;
                        match key {
                            "height" => ocean.swell.height = value.max(0.0),
                            "period" => ocean.swell.period = value.max(1.0),
                            "from" => ocean.swell.from = value,
                            _ => return Err(at(format!("unknown swell setting {key}"))),
                        }
                    }
                }
                _ => return Err(at(format!("unknown setting {keyword:?}"))),
            }
        }
        Ok(ocean)
    }

    /// Writes this ocean in the form [`Ocean::parse`] reads.
    pub fn to_text(&self) -> String {
        let [ar, ag, ab] = self.absorption;
        let [sr, sg, sb] = self.scattering;
        format!(
            "absorption {ar} {ag} {ab}\nscattering {sr} {sg} {sb}\nturbidity {}\nfoam {}\nchoppiness {}\nswell height={} period={} from={}\nfetch {}\n",
            self.turbidity,
            self.foam,
            self.choppiness,
            self.swell.height,
            self.swell.period,
            self.swell.from,
            self.fetch
        )
    }

    /// Part way from this ocean to `to`, by `t` in `0..1`. The swell turns the short way round.
    pub fn lerp(&self, to: &Self, t: f32) -> Self {
        let mix = |a: f32, b: f32| a + (b - a) * t;
        let mix3 = |a: [f32; 3], b: [f32; 3]| [mix(a[0], b[0]), mix(a[1], b[1]), mix(a[2], b[2])];
        let turn = (to.swell.from - self.swell.from + 540.0).rem_euclid(360.0) - 180.0;
        Self {
            absorption: mix3(self.absorption, to.absorption),
            scattering: mix3(self.scattering, to.scattering),
            turbidity: mix(self.turbidity, to.turbidity),
            foam: mix(self.foam, to.foam),
            choppiness: mix(self.choppiness, to.choppiness),
            swell: Swell {
                height: mix(self.swell.height, to.swell.height),
                period: mix(self.swell.period, to.swell.period),
                from: (self.swell.from + turn * t).rem_euclid(360.0),
            },
            fetch: mix(self.fetch, to.fetch),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_built_in_ocean_reads_and_writes_back_unchanged() {
        for name in Ocean::names() {
            let ocean = Ocean::by_name(name).unwrap();
            assert_eq!(Ocean::parse(&ocean.to_text()).unwrap(), ocean, "{name}");
        }
    }

    #[test]
    fn errors_name_the_line() {
        let error = Ocean::parse("foam 1\nfetch far").unwrap_err();
        assert!(error.starts_with("line 2"), "{error}");
        assert!(Ocean::parse("absorption 1 2")
            .unwrap_err()
            .contains("three numbers"));
        assert!(Ocean::parse("swell size=3")
            .unwrap_err()
            .contains("unknown swell setting"));
    }

    #[test]
    fn easing_turns_the_swell_the_short_way() {
        let mut a = Ocean::default();
        let mut b = Ocean::default();
        a.swell.from = 350.0;
        b.swell.from = 10.0;
        assert!((a.lerp(&b, 0.5).swell.from - 0.0).abs() < 1.0e-3);
    }
}
