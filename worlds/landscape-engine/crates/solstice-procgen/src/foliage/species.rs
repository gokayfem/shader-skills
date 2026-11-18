//! Species: everything that decides how a plant grows, as plain data with a text form.
//!
//! A species is written one setting per line, in the style of scene files, so a new tree or a
//! change to an old one needs no code:
//!
//! ```text
//! # A tall, slender silver birch.
//! height 14
//! trunk radius=0.15 flare=0.3 gnarl=0.06
//! crown round
//! bark birch_bark
//! level per_metre=3.4 span=0.35..0.98 angle=56..28 length=0.36 radius=0.5 upturn=0.22
//! level per_metre=3 angle=45 length=0.5 droop=0.45
//! leaves birch_leaves per_metre=34 length=0.045..0.07 width=0.7
//! ```
//!
//! Every `key=value` has a default, so a line names only what differs. A value written `a..b`
//! is a range; a single number stands for both ends. [`Species::parse`] reads this form and
//! [`Species::to_text`] writes it back, so a species can be loaded, adjusted in code and saved.
//!
//! The built-in species are files of exactly this kind, in `species/` beside this crate.

use super::bark::Bark;
use super::leaf::{LeafKind, LEAF_VARIANTS};
use super::text::{number, pairs, range, range_text, single};
use std::f32::consts::PI;
use std::fmt::Write as _;

/// The built-in species, by name, as text.
const BUILT_IN: [(&str, &str); 6] = [
    ("oak", include_str!("../../species/oak.species")),
    ("old_oak", include_str!("../../species/old_oak.species")),
    ("willow", include_str!("../../species/willow.species")),
    ("birch", include_str!("../../species/birch.species")),
    ("fir", include_str!("../../species/fir.species")),
    ("bush", include_str!("../../species/bush.species")),
];

/// The outline of a crown: how long the trunk's branches are at each height.
#[derive(Clone, Debug, PartialEq)]
pub enum Crown {
    /// Longest a third of the way up, as on an oak or a beech in the open.
    Round,
    /// Longest at the bottom, as on a conifer.
    Cone,
    /// Much the same length all the way up, as on a bush.
    Even,
    /// Lengths at even steps from the bottom of the crown to the top, joined by straight lines.
    Profile(Vec<f32>),
}

impl Crown {
    /// Branch length at `x`, from the bottom of the crown (0) to the top (1), as a fraction of
    /// the longest.
    pub fn envelope(&self, x: f32) -> f32 {
        let x = x.clamp(0.0, 1.0);
        match self {
            Self::Round => (PI * (x * 0.85 + 0.12)).sin().powf(0.7),
            Self::Cone => 1.0 - x * 0.85,
            Self::Even => 1.0 - x * 0.3,
            Self::Profile(points) => match points.len() {
                0 => 1.0,
                1 => points[0],
                n => {
                    let f = x * (n - 1) as f32;
                    let i = (f as usize).min(n - 2);
                    points[i] + (points[i + 1] - points[i]) * (f - i as f32)
                }
            },
        }
    }
}

/// One level of branching.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Level {
    /// Children per metre of the parent's length.
    pub per_metre: f32,
    /// Stretch of the parent they grow from, as fractions of its length.
    pub span: (f32, f32),
    /// Angle from the parent, in degrees, at the start and at the end of the span. Open-grown
    /// trees spread their lower limbs wide and reach up with the higher ones.
    pub angle: (f32, f32),
    /// Length as a fraction of the parent's.
    pub length: f32,
    /// How much shorter shoots are at the end of the span than at its start, which rounds off a
    /// branch's tip. The trunk's own branches take their lengths from the crown instead.
    pub taper: f32,
    /// Radius where it starts, as a fraction of the parent's there.
    pub radius: f32,
    /// How strongly it turns towards the light, per metre.
    pub upturn: f32,
    /// How strongly it sags under its weight, per metre, most towards its tip.
    pub droop: f32,
    /// How much it wanders, per metre.
    pub gnarl: f32,
    /// Children in each ring, for branches in whorls as on a conifer, or 0 for a spiral.
    pub whorl: u32,
    /// For shoots that hang, as a weeping willow's do: the height above the plant's ground at
    /// which they end, give or take, so that a curtain of them stops short of the grass however
    /// high each began. Their `length` is then the longest any grows. 0 for shoots that don't.
    pub reach: f32,
}

impl Default for Level {
    fn default() -> Self {
        Self {
            per_metre: 4.0,
            span: (0.0, 1.0),
            angle: (45.0, 45.0),
            length: 0.5,
            taper: 0.5,
            radius: 0.6,
            upturn: 0.0,
            droop: 0.0,
            gnarl: 0.3,
            whorl: 0,
            reach: 0.0,
        }
    }
}

/// How leaves grow.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Leaves {
    pub kind: LeafKind,
    /// Leaves per metre of the last level of stem.
    pub per_metre: f32,
    /// Range of leaf lengths, in metres.
    pub length: (f32, f32),
    /// Width as a fraction of length. For needles, the length of each needle as a fraction of
    /// its shoot's.
    pub width: f32,
    /// How likely each atlas variant is.
    pub variants: [f32; LEAF_VARIANTS as usize],
    /// How far leaf normals lean out from the centre of the crown, which lights a crown as one
    /// soft mass rather than thousands of separately facing leaves.
    pub crown_normals: f32,
    /// How far each leaf points along its shoot rather than out from it: 0 stands square to the
    /// shoot, 0.6 is most trees', and more lays leaves along it, as a willow's hang.
    pub lean: f32,
}

impl Default for Leaves {
    fn default() -> Self {
        Self {
            kind: LeafKind::Broadleaf,
            per_metre: 40.0,
            length: (0.08, 0.12),
            width: 0.55,
            variants: [0.4, 0.3, 0.2, 0.1],
            crown_normals: 0.45,
            lean: 0.6,
        }
    }
}

/// Everything that decides how a plant grows.
#[derive(Clone, Debug, PartialEq)]
pub struct Species {
    /// Length of the trunk, in metres.
    pub height: f32,
    pub trunk_radius: f32,
    /// How much wider the trunk spreads at the ground, as a fraction of its radius.
    pub root_flare: f32,
    pub trunk_gnarl: f32,
    pub crown: Crown,
    pub bark: Bark,
    /// Branching levels, starting with those on the trunk.
    pub levels: Vec<Level>,
    pub leaves: Leaves,
    /// The youngest shoots, if they are not yet barked: stems thinner than this radius where
    /// they start, in metres, are grown as a mesh of their own, `Plant::twigs`, to be given
    /// this colour: a willow's withes are olive-gold, a dogwood's red.
    pub twigs: Option<(f32, [f32; 3])>,
}

impl Default for Species {
    fn default() -> Self {
        Self {
            height: 10.0,
            trunk_radius: 0.2,
            root_flare: 0.4,
            trunk_gnarl: 0.05,
            crown: Crown::Round,
            bark: Bark::Oak,
            levels: Vec::new(),
            leaves: Leaves::default(),
            twigs: None,
        }
    }
}

impl Species {
    /// Names of the built-in species.
    pub fn names() -> impl Iterator<Item = &'static str> {
        BUILT_IN.iter().map(|(name, _)| *name)
    }

    /// A built-in species, or `None` if there is none of that name.
    pub fn by_name(name: &str) -> Option<Self> {
        let (_, text) = BUILT_IN.iter().find(|(n, _)| *n == name)?;
        Some(Self::parse(text).unwrap_or_else(|e| panic!("built-in species {name}: {e}")))
    }

    /// A young oak grown in the open.
    pub fn oak() -> Self {
        Self::by_name("oak").unwrap()
    }

    /// A tall, slender silver birch.
    pub fn birch() -> Self {
        Self::by_name("birch").unwrap()
    }

    /// A fir, in whorls of level branches.
    pub fn fir() -> Self {
        Self::by_name("fir").unwrap()
    }

    /// A dense bush of many stems from the ground.
    pub fn bush() -> Self {
        Self::by_name("bush").unwrap()
    }

    /// Reads a species from its text form. Errors name the line at fault.
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut species = Self::default();
        let mut have_leaves = false;
        for (index, raw) in text.lines().enumerate() {
            let line = raw.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            let at = |e: String| format!("line {}: {e}", index + 1);
            let mut words = line.split_whitespace();
            let keyword = words.next().unwrap_or("");
            let rest: Vec<&str> = words.collect();
            match keyword {
                "height" => species.height = single(&rest).map_err(at)?,
                "trunk" => {
                    for (key, value) in pairs(&rest).map_err(at)? {
                        match key {
                            "radius" => species.trunk_radius = number(value).map_err(at)?,
                            "flare" => species.root_flare = number(value).map_err(at)?,
                            "gnarl" => species.trunk_gnarl = number(value).map_err(at)?,
                            _ => return Err(at(format!("unknown trunk setting {key}"))),
                        }
                    }
                }
                "crown" => species.crown = crown(&rest).map_err(at)?,
                "bark" => {
                    let name = rest.first().copied().unwrap_or("");
                    species.bark =
                        Bark::by_name(name).ok_or_else(|| at(format!("unknown bark {name:?}")))?;
                }
                "level" => species.levels.push(level(&rest).map_err(at)?),
                "twigs" => {
                    let mut below = 0.01;
                    let mut colour = [0.2, 0.17, 0.08];
                    for (key, value) in pairs(&rest).map_err(at)? {
                        match key {
                            "below" => below = number(value).map_err(at)?,
                            "color" => {
                                colour = super::text::color(value).map_err(at)?.to_array();
                            }
                            _ => return Err(at(format!("unknown twigs setting {key}"))),
                        }
                    }
                    species.twigs = Some((below, colour));
                }
                "leaves" => {
                    species.leaves = leaves(&rest).map_err(at)?;
                    have_leaves = true;
                }
                _ => return Err(at(format!("unknown setting {keyword:?}"))),
            }
        }
        if species.levels.is_empty() {
            return Err("a species needs at least one level of branches".into());
        }
        if !have_leaves {
            return Err("a species needs a leaves line".into());
        }
        Ok(species)
    }

    /// Writes this species in the form [`Species::parse`] reads.
    pub fn to_text(&self) -> String {
        let mut text = String::new();
        let _ = writeln!(text, "height {}", self.height);
        let _ = writeln!(
            text,
            "trunk radius={} flare={} gnarl={}",
            self.trunk_radius, self.root_flare, self.trunk_gnarl
        );
        let crown = match &self.crown {
            Crown::Round => "round".to_string(),
            Crown::Cone => "cone".to_string(),
            Crown::Even => "even".to_string(),
            Crown::Profile(points) => {
                let points: Vec<String> = points.iter().map(f32::to_string).collect();
                format!("profile {}", points.join(" "))
            }
        };
        let _ = writeln!(text, "crown {crown}");
        let _ = writeln!(text, "bark {}", self.bark.name());
        for l in &self.levels {
            let _ = writeln!(
                text,
                "level per_metre={} span={} angle={} length={} taper={} radius={} upturn={} droop={} gnarl={} whorl={} reach={}",
                l.per_metre,
                range_text(l.span),
                range_text(l.angle),
                l.length,
                l.taper,
                l.radius,
                l.upturn,
                l.droop,
                l.gnarl,
                l.whorl,
                l.reach
            );
        }
        let v = self.leaves.variants;
        let _ = writeln!(
            text,
            "leaves {} per_metre={} length={} width={} variants={},{},{},{} normals={} lean={}",
            self.leaves.kind.name(),
            self.leaves.per_metre,
            range_text(self.leaves.length),
            self.leaves.width,
            v[0],
            v[1],
            v[2],
            v[3],
            self.leaves.crown_normals,
            self.leaves.lean
        );
        if let Some((below, [r, g, b])) = self.twigs {
            let _ = writeln!(text, "twigs below={below} color={r},{g},{b}");
        }
        text
    }
}

fn crown(words: &[&str]) -> Result<Crown, String> {
    match words {
        ["round"] => Ok(Crown::Round),
        ["cone"] => Ok(Crown::Cone),
        ["even"] => Ok(Crown::Even),
        ["profile", points @ ..] if !points.is_empty() => Ok(Crown::Profile(
            points.iter().map(|p| number(p)).collect::<Result<_, _>>()?,
        )),
        _ => Err("crown is round, cone, even, or profile followed by lengths".into()),
    }
}

fn level(words: &[&str]) -> Result<Level, String> {
    let mut level = Level::default();
    for (key, value) in pairs(words)? {
        match key {
            "per_metre" => level.per_metre = number(value)?,
            "span" => level.span = range(value)?,
            "angle" => level.angle = range(value)?,
            "length" => level.length = number(value)?,
            "taper" => level.taper = number(value)?,
            "radius" => level.radius = number(value)?,
            "upturn" => level.upturn = number(value)?,
            "droop" => level.droop = number(value)?,
            "gnarl" => level.gnarl = number(value)?,
            "reach" => level.reach = number(value)?,
            "whorl" => {
                level.whorl = value
                    .parse()
                    .map_err(|_| format!("whorl {value:?} is not a whole number"))?
            }
            _ => return Err(format!("unknown level setting {key}")),
        }
    }
    if level.span.0 > level.span.1 || level.span.0 < 0.0 || level.span.1 > 1.0 {
        return Err("span must lie within 0..1 and run forwards".into());
    }
    Ok(level)
}

fn leaves(words: &[&str]) -> Result<Leaves, String> {
    let (kind, settings) = words.split_first().ok_or("leaves needs a kind")?;
    let mut leaves = Leaves {
        kind: LeafKind::by_name(kind).ok_or_else(|| format!("unknown leaf kind {kind:?}"))?,
        ..Default::default()
    };
    for (key, value) in pairs(settings)? {
        match key {
            "per_metre" => leaves.per_metre = number(value)?,
            "length" => leaves.length = range(value)?,
            "width" => leaves.width = number(value)?,
            "normals" => leaves.crown_normals = number(value)?,
            "lean" => leaves.lean = number(value)?,
            "variants" => {
                let values: Vec<f32> = value.split(',').map(number).collect::<Result<_, _>>()?;
                leaves.variants = values.try_into().map_err(|_| {
                    format!("variants needs {LEAF_VARIANTS} weights, separated by commas")
                })?;
            }
            _ => return Err(format!("unknown leaves setting {key}")),
        }
    }
    Ok(leaves)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_built_in_species_reads_and_writes_back_unchanged() {
        for name in Species::names() {
            let species = Species::by_name(name).unwrap();
            assert_eq!(Species::parse(&species.to_text()), Ok(species), "{name}");
        }
        assert!(Species::by_name("baobab").is_none());
    }

    #[test]
    fn a_line_names_only_what_differs_from_the_defaults() {
        let species =
            Species::parse("height 6\nlevel angle=60..30\nleaves birch_leaves length=0.05\n")
                .unwrap();
        assert_eq!(species.height, 6.0);
        assert_eq!(species.levels[0].angle, (60.0, 30.0));
        assert_eq!(species.levels[0].length, Level::default().length);
        assert_eq!(species.leaves.kind, LeafKind::Birch);
        assert_eq!(species.leaves.length, (0.05, 0.05));
    }

    #[test]
    fn mistakes_are_reported_with_their_line() {
        let error = |text: &str| Species::parse(text).unwrap_err();
        assert!(error("height 6\nlevel lenght=2\n").starts_with("line 2:"));
        assert!(error("bark cork\n").contains("cork"));
        assert!(error("leaves fern\n").contains("fern"));
        assert!(error("level span=0.8..0.2\nleaves fir_needles\n").contains("span"));
        assert!(error("leaves fir_needles\n").contains("level"));
        assert!(error("level\n").contains("leaves"));
    }

    #[test]
    fn a_profile_crown_follows_its_points() {
        let crown = Crown::Profile(vec![1.0, 0.5, 0.0]);
        assert_eq!(crown.envelope(0.0), 1.0);
        assert!((crown.envelope(0.25) - 0.75).abs() < 1.0e-6);
        assert_eq!(crown.envelope(1.0), 0.0);
    }
}
