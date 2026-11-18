//! Effects as data: emitters built from small modules, with a text form (docs/particles.md).
//!
//! An effect is written one module per line, like a weather or an ocean:
//!
//! ```text
//! # A fountain: a jet of water that arcs up and falls back.
//! param strength 1
//!
//! emitter jet
//!   budget 4000
//!   spawn rate=600*strength
//!   shape sphere 0.05
//!   lifetime 1.6 2.0
//!   velocity 0 6 0 cone=8 spread=0.1
//!   size 0.02 0.035
//!   gravity
//!   drag 0.3
//!   color 0.7 0.8 0.9 0.6
//!   color life 0=1 1 1 0 0.1=1 1 1 1 1=1 1 1 0
//!   render sprite soft=0.2
//! ```
//!
//! Every number may instead be a parameter's name, or a number times one (`600*strength`), which
//! is how the weather or a script drives an effect while it runs. Two numbers where one would do
//! are a range, each particle taking its own value uniformly between them. Options are
//! `key=value`, and take the numbers that follow them too (`offset=0 1 0`).
//!
//! Every module has a default, so an emitter names only what differs. The built-in effects are
//! files of exactly this kind, in `particles/` beside this crate.

use crate::text::number;

/// The most emitters one effect may have.
pub const MAX_EMITTERS_PER_EFFECT: usize = 8;
/// The largest budget a sorted emitter may have: its whole list is sorted in one workgroup's
/// memory, of which WebGPU promises 16 KiB, four bytes a particle.
pub const MAX_SORTED: u32 = 4096;
/// Entries each curve is baked to on the GPU.
pub const CURVE_ENTRIES: usize = 32;

/// The built-in effects, by name, as text.
const BUILT_IN: [(&str, &str); 3] = [
    (
        "fountain",
        include_str!("../../particles/fountain.particles"),
    ),
    ("motes", include_str!("../../particles/motes.particles")),
    (
        "fireflies",
        include_str!("../../particles/fireflies.particles"),
    ),
];

/// A number, or a number times one of its effect's parameters.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Value {
    pub number: f32,
    /// Index into [`Effect::params`].
    pub param: Option<usize>,
}

impl Value {
    pub const fn constant(number: f32) -> Self {
        Self {
            number,
            param: None,
        }
    }

    /// Its value under the parameters' current values.
    pub fn eval(&self, params: &[f32]) -> f32 {
        match self.param {
            Some(p) => self.number * params.get(p).copied().unwrap_or(0.0),
            None => self.number,
        }
    }
}

/// Where new particles appear, in the effect's own space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Shape {
    Point,
    /// Within a sphere of this radius, or on its surface only.
    Sphere {
        radius: Value,
        surface: bool,
    },
    /// Within a box of these full sizes, centred on the origin.
    Box {
        size: [Value; 3],
    },
}

/// How a particle is blended over what is behind it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Blend {
    /// Covers what is behind by its opacity: smoke, dust, water.
    #[default]
    Alpha,
    /// Adds its light and hides nothing: sparks, glows.
    Add,
}

/// How a particle is lit.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Shading {
    /// By the sun, the sky, light bounced off the scene and local lights, through its phase
    /// function: smoke, dust, spray.
    #[default]
    Lit,
    /// Its colour times its brightness, as light of its own: glows and test patterns.
    Unlit,
}

/// What particles collide with: any of these, as bits of [`Collide::against`].
pub const COLLIDE_DEPTH: u32 = 1;
pub const COLLIDE_FIELD: u32 = 2;
pub const COLLIDE_WATER: u32 = 4;

/// What a particle does when it hits something.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Response {
    /// Rebounds, losing speed into the surface by its restitution and along it by friction.
    #[default]
    Bounce,
    /// Stays where it hit for the rest of its life: snow settling.
    Stick,
    /// Dies: a raindrop that has landed.
    Kill,
}

/// How an emitter's particles collide, if they do.
#[derive(Clone, Debug, PartialEq)]
pub struct Collide {
    /// What they hit: `COLLIDE_` bits.
    pub against: u32,
    pub response: Response,
    /// The share of speed into the surface kept on a bounce, `0..1`.
    pub restitution: Value,
    /// The share of speed along the surface lost on a bounce, `0..1`.
    pub friction: Value,
    /// An event each hit sends, which the emitter spawning on it hears.
    pub event: Option<String>,
}

/// Where a moving effect keeps its emitter.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Follow {
    /// Where the effect is placed.
    #[default]
    Effect,
    /// Around the camera, wherever it goes: rain and snow. Rotation is the effect's.
    Camera,
}

/// A value over a particle's life, as keys from 0 (born) to 1 (dies), linear between them.
#[derive(Clone, Debug, PartialEq)]
pub struct Curve<const N: usize> {
    pub keys: Vec<(f32, [f32; N])>,
}

impl<const N: usize> Curve<N> {
    /// A curve that is `value` throughout.
    pub fn constant(value: [f32; N]) -> Self {
        Self {
            keys: vec![(0.0, value)],
        }
    }

    /// Its value at `t` in `0..1`.
    pub fn at(&self, t: f32) -> [f32; N] {
        let Some(first) = self.keys.first() else {
            return [1.0; N];
        };
        if t <= first.0 {
            return first.1;
        }
        for pair in self.keys.windows(2) {
            let ((t0, a), (t1, b)) = (pair[0], pair[1]);
            if t <= t1 {
                let f = if t1 > t0 { (t - t0) / (t1 - t0) } else { 1.0 };
                return std::array::from_fn(|i| a[i] + (b[i] - a[i]) * f);
            }
        }
        self.keys.last().expect("not empty").1
    }

    /// Its values at [`CURVE_ENTRIES`] evenly spaced times from 0 to 1, as the GPU holds it.
    pub fn bake(&self) -> [[f32; N]; CURVE_ENTRIES] {
        std::array::from_fn(|i| self.at(i as f32 / (CURVE_ENTRIES - 1) as f32))
    }
}

/// One emitter of an effect: how its particles are born, move, and look.
#[derive(Clone, Debug, PartialEq)]
pub struct Emitter {
    pub name: String,
    /// The most particles it may have alive at once.
    pub budget: u32,
    pub follow: Follow,
    /// New particles per second.
    pub rate: Value,
    /// Particles at once when the effect starts, and again every `every` seconds if that is
    /// above zero.
    pub burst: Value,
    pub every: Value,
    pub shape: Shape,
    /// Where the shape is centred, in the effect's space.
    pub offset: [Value; 3],
    /// Seconds each particle lives, between the two.
    pub lifetime: [Value; 2],
    /// Initial direction and speed, in metres per second, in the effect's space.
    pub velocity: [Value; 3],
    /// Half-angle of the cone the initial direction is spread over, in degrees.
    pub cone: Value,
    /// Speed varies by up to this fraction either way.
    pub spread: Value,
    /// Radius of each particle in metres, between the two.
    pub size: [Value; 2],
    /// Linear colour and opacity. For lit particles the colour is how much light they scatter,
    /// as an albedo; for unlit ones it is the light they give.
    pub color: [Value; 4],
    /// Scales the colour. For unlit particles it is the radiance of a white one in the scene's
    /// units: 1 is a white surface lit by a sun of illuminance π.
    pub brightness: Value,
    pub shading: Shading,
    /// Drawn opaque where its shape is more than half covered, writing depth and motion like
    /// any surface, so that the temporal filter follows it exactly: sparks, debris.
    pub solid: bool,
    /// Its particles are drawn far to near, so that dark smoke in front hides what is behind.
    pub sort: bool,
    /// Henyey-Greenstein asymmetry of the light a lit particle scatters, `-1..1`: 0 scatters
    /// evenly, near 1 mostly onward, so that mist glows with the sun behind it.
    pub phase: Value,
    /// Temperature at birth in kelvin, between the two. Above about 800 K a particle glows as
    /// a black body; 0 is no glow.
    pub temperature: [Value; 2],
    /// How fast it cools towards the air's 300 K, as a rate per second.
    pub cooling: Value,
    /// How much of a black body's light it gives at its temperature, `0..1`: its emissivity.
    pub glow: Value,
    /// How strongly the wind and other moving air carry it, as a share of the weather's wind
    /// near the ground. The air pulls a particle by its drag, so without drag it does nothing.
    pub wind: Value,
    /// Swirling air, from the curl of a noise field, so it neither bunches particles up nor
    /// thins them out: its speed in m/s, its eddies' size in metres, and how fast they change.
    pub curl: [Value; 3],
    /// Hot particles rise: their lift as a share of gravity, times how much hotter than the air
    /// they are, as a fraction of their temperature.
    pub buoyancy: Value,
    /// Acceleration towards the effect's origin, in m/s².
    pub attract: Value,
    /// Air turning about the effect's up axis, in radians per second.
    pub vortex: Value,
    pub collide: Option<Collide>,
    /// An event this emitter spawns on, and how many particles each hit spawns.
    pub on_event: Option<(String, Value)>,
    /// Downward acceleration in m/s².
    pub gravity: Value,
    /// Linear drag: the share of its velocity relative to still air a particle loses, per
    /// second, as a rate (`dv/dt = -drag · v`).
    pub drag: Value,
    pub color_life: Curve<4>,
    pub size_life: Curve<1>,
    pub blend: Blend,
    /// Distance over which a particle fades into the surfaces behind it, in metres.
    pub soft: Value,
    pub opacity: Value,
}

impl Emitter {
    fn new(name: &str) -> Self {
        Self {
            name: name.to_owned(),
            budget: 1000,
            follow: Follow::Effect,
            rate: Value::constant(0.0),
            burst: Value::constant(0.0),
            every: Value::constant(0.0),
            shape: Shape::Point,
            offset: [Value::constant(0.0); 3],
            lifetime: [Value::constant(1.0); 2],
            velocity: [Value::constant(0.0); 3],
            cone: Value::constant(0.0),
            spread: Value::constant(0.0),
            size: [Value::constant(0.05); 2],
            color: [Value::constant(1.0); 4],
            brightness: Value::constant(1.0),
            shading: Shading::Lit,
            solid: false,
            sort: false,
            phase: Value::constant(0.2),
            temperature: [Value::constant(0.0); 2],
            cooling: Value::constant(0.0),
            glow: Value::constant(1.0),
            wind: Value::constant(0.0),
            curl: [
                Value::constant(0.0),
                Value::constant(4.0),
                Value::constant(0.2),
            ],
            buoyancy: Value::constant(0.0),
            attract: Value::constant(0.0),
            vortex: Value::constant(0.0),
            collide: None,
            on_event: None,
            gravity: Value::constant(0.0),
            drag: Value::constant(0.0),
            color_life: Curve::constant([1.0; 4]),
            size_life: Curve::constant([1.0]),
            blend: Blend::Alpha,
            soft: Value::constant(0.25),
            opacity: Value::constant(1.0),
        }
    }
}

/// An effect: named parameters and the emitters they drive.
#[derive(Clone, Debug, PartialEq)]
pub struct Effect {
    /// Each parameter's name and default value.
    pub params: Vec<(String, f32)>,
    pub emitters: Vec<Emitter>,
}

/// Standard gravity, for `gravity` with no value.
pub const STANDARD_GRAVITY: f32 = 9.81;

impl Effect {
    /// Names of the built-in effects.
    pub fn names() -> impl Iterator<Item = &'static str> {
        BUILT_IN.iter().map(|(name, _)| *name)
    }

    /// A built-in effect, or `None` if there is none of that name.
    pub fn by_name(name: &str) -> Option<Self> {
        let (_, text) = BUILT_IN.iter().find(|(n, _)| *n == name)?;
        Some(Self::parse(text).unwrap_or_else(|e| panic!("built-in effect {name}: {e}")))
    }

    /// The index of the parameter called `name`.
    pub fn param(&self, name: &str) -> Option<usize> {
        self.params.iter().position(|(n, _)| n == name)
    }

    /// Reads an effect from its text form. Errors name the line at fault.
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut effect = Effect {
            params: Vec::new(),
            emitters: Vec::new(),
        };
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
            effect.line(keyword, rest).map_err(at)?;
        }
        // Every event sent is heard by some emitter of the effect.
        for emitter in &effect.emitters {
            let Some(event) = emitter.collide.as_ref().and_then(|c| c.event.as_ref()) else {
                continue;
            };
            let heard = effect
                .emitters
                .iter()
                .any(|e| e.on_event.as_ref().is_some_and(|(name, _)| name == event));
            if !heard {
                return Err(format!(
                    "emitter {} sends event {event}, but no emitter spawns on it",
                    emitter.name
                ));
            }
        }
        // And by one emitter only: an event goes to a single emitter's slice.
        for (n, emitter) in effect.emitters.iter().enumerate() {
            let Some((event, _)) = &emitter.on_event else {
                continue;
            };
            let again = effect.emitters[n + 1..]
                .iter()
                .find(|e| e.on_event.as_ref().is_some_and(|(name, _)| name == event));
            if let Some(other) = again {
                return Err(format!(
                    "emitters {} and {} both spawn on event {event}; only one may",
                    emitter.name, other.name
                ));
            }
        }
        for emitter in &effect.emitters {
            if emitter.sort && emitter.budget > MAX_SORTED {
                return Err(format!(
                    "emitter {} sorts its particles, so its budget can be at most {MAX_SORTED}",
                    emitter.name
                ));
            }
        }
        if effect.emitters.is_empty() {
            return Err("an effect needs at least one emitter".into());
        }
        Ok(effect)
    }

    fn line(&mut self, keyword: &str, rest: &[&str]) -> Result<(), String> {
        match keyword {
            "param" => {
                if !self.emitters.is_empty() {
                    return Err("parameters come before the first emitter".into());
                }
                let [name, default] = rest else {
                    return Err("param takes a name and a default value".into());
                };
                if !is_name(name) {
                    return Err(format!("{name:?} is not a valid parameter name"));
                }
                if self.param(name).is_some() {
                    return Err(format!("parameter {name} is declared twice"));
                }
                self.params.push(((*name).to_owned(), number(default)?));
                Ok(())
            }
            "emitter" => {
                let [name] = rest else {
                    return Err("emitter takes a name".into());
                };
                if self.emitters.len() == MAX_EMITTERS_PER_EFFECT {
                    return Err(format!(
                        "an effect may have at most {MAX_EMITTERS_PER_EFFECT} emitters"
                    ));
                }
                if self.emitters.iter().any(|e| e.name == *name) {
                    return Err(format!("emitter {name} is declared twice"));
                }
                self.emitters.push(Emitter::new(name));
                Ok(())
            }
            _ => {
                let Some(emitter) = self.emitters.last() else {
                    return Err(format!("{keyword} must come after an emitter"));
                };
                let mut emitter = emitter.clone();
                let words = Words::split(rest)?;
                module(&mut emitter, keyword, &words, &self.params)?;
                *self.emitters.last_mut().expect("checked above") = emitter;
                Ok(())
            }
        }
    }
}

/// A line's words after its keyword: plain words, then `key=value` options, each with the
/// plain words that follow it.
struct Words<'a> {
    plain: Vec<&'a str>,
    options: Vec<(&'a str, Vec<&'a str>)>,
}

impl<'a> Words<'a> {
    fn split(rest: &[&'a str]) -> Result<Self, String> {
        let mut words = Words {
            plain: Vec::new(),
            options: Vec::new(),
        };
        for word in rest {
            match word.split_once('=') {
                Some((key, value)) => {
                    if key.is_empty() {
                        return Err(format!("{word:?} has no name before its ="));
                    }
                    let values = if value.is_empty() {
                        vec![]
                    } else {
                        vec![value]
                    };
                    words.options.push((key, values));
                }
                None => match words.options.last_mut() {
                    Some((_, values)) => values.push(word),
                    None => words.plain.push(word),
                },
            }
        }
        Ok(words)
    }

    /// Checks every option is one of `known`.
    fn only(&self, keyword: &str, known: &[&str]) -> Result<(), String> {
        for (key, _) in &self.options {
            if !known.contains(key) {
                return Err(format!("{keyword} has no option {key}"));
            }
        }
        Ok(())
    }

    fn option(&self, key: &str) -> Option<&[&'a str]> {
        self.options
            .iter()
            .rev()
            .find(|(k, _)| *k == key)
            .map(|(_, v)| v.as_slice())
    }
}

fn is_name(word: &str) -> bool {
    let mut chars = word.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// A number, a parameter, or a number times a parameter.
fn value(word: &str, params: &[(String, f32)]) -> Result<Value, String> {
    let param = |name: &str| {
        params
            .iter()
            .position(|(n, _)| n == name)
            .ok_or_else(|| format!("{name:?} is neither a number nor a parameter"))
    };
    if let Some((left, right)) = word.split_once('*') {
        let (scale, name) = match (number(left), number(right)) {
            (Ok(n), Err(_)) => (n, right),
            (Err(_), Ok(n)) => (n, left),
            _ => return Err(format!("{word:?} must be a number times a parameter")),
        };
        return Ok(Value {
            number: scale,
            param: Some(param(name)?),
        });
    }
    match number(word) {
        Ok(n) => Ok(Value::constant(n)),
        Err(_) if is_name(word) => Ok(Value {
            number: 1.0,
            param: Some(param(word)?),
        }),
        Err(e) => Err(e),
    }
}

fn values<const N: usize>(
    words: &[&str],
    params: &[(String, f32)],
    what: &str,
) -> Result<[Value; N], String> {
    if words.len() != N {
        return Err(format!("{what} takes {N} values, not {}", words.len()));
    }
    let mut out = [Value::constant(0.0); N];
    for (slot, word) in out.iter_mut().zip(words) {
        *slot = value(word, params)?;
    }
    Ok(out)
}

/// One value, or two for a range.
fn range(words: &[&str], params: &[(String, f32)], what: &str) -> Result<[Value; 2], String> {
    match words {
        [one] => {
            let v = value(one, params)?;
            Ok([v, v])
        }
        [low, high] => Ok([value(low, params)?, value(high, params)?]),
        _ => Err(format!("{what} takes one value or two for a range")),
    }
}

fn single(words: &[&str], params: &[(String, f32)], what: &str) -> Result<Value, String> {
    let [v] = values::<1>(words, params, what)?;
    Ok(v)
}

/// Keys of a curve: `T=A B C ...`, with N values each.
fn curve<const N: usize>(words: &Words, what: &str) -> Result<Curve<N>, String> {
    if !words.plain.is_empty() {
        return Err(format!(
            "{what} takes keys such as 0=…, not {:?}",
            words.plain[0]
        ));
    }
    let mut keys: Vec<(f32, [f32; N])> = Vec::new();
    for (time, numbers) in &words.options {
        let t = number(time)?;
        if !(0.0..=1.0).contains(&t) {
            return Err(format!("{what}: key {t} lies outside 0 to 1"));
        }
        if keys.last().is_some_and(|(last, _)| *last >= t) {
            return Err(format!("{what}: keys must be in order of time"));
        }
        if numbers.len() != N {
            return Err(format!("{what}: each key takes {N} values"));
        }
        let mut v = [0.0; N];
        for (slot, word) in v.iter_mut().zip(numbers) {
            *slot = number(word)?;
        }
        keys.push((t, v));
    }
    if keys.is_empty() {
        return Err(format!("{what} needs at least one key"));
    }
    Ok(Curve { keys })
}

fn module(
    emitter: &mut Emitter,
    keyword: &str,
    words: &Words,
    params: &[(String, f32)],
) -> Result<(), String> {
    let plain = words.plain.as_slice();
    let no_options = |w: &Words| w.only(keyword, &[]);
    match keyword {
        "budget" => {
            no_options(words)?;
            let [count] = plain else {
                return Err("budget takes a count".into());
            };
            emitter.budget = count
                .parse::<u32>()
                .ok()
                .filter(|&n| n > 0)
                .ok_or_else(|| format!("{count:?} is not a count above zero"))?;
        }
        "follow" => {
            no_options(words)?;
            emitter.follow = match plain {
                ["camera"] => Follow::Camera,
                ["effect"] => Follow::Effect,
                _ => return Err("follow takes camera or effect".into()),
            };
        }
        "spawn" => {
            words.only(keyword, &["rate", "burst", "every", "event", "count"])?;
            if !plain.is_empty() {
                return Err("spawn takes rate=, burst=, every=, and event= with count=".into());
            }
            if let Some(v) = words.option("event") {
                let [name] = v else {
                    return Err("event takes a name".into());
                };
                if !is_name(name) {
                    return Err(format!("{name:?} is not a valid event name"));
                }
                let count = match words.option("count") {
                    Some(c) => single(c, params, "count")?,
                    None => Value::constant(1.0),
                };
                emitter.on_event = Some(((*name).to_owned(), count));
            } else if words.option("count").is_some() {
                return Err("count= goes with event=".into());
            }
            if let Some(v) = words.option("rate") {
                emitter.rate = single(v, params, "rate")?;
            }
            if let Some(v) = words.option("burst") {
                emitter.burst = single(v, params, "burst")?;
            }
            if let Some(v) = words.option("every") {
                emitter.every = single(v, params, "every")?;
            }
        }
        "shape" => {
            no_options(words)?;
            emitter.shape = match plain {
                ["point"] => Shape::Point,
                ["sphere", radius] => Shape::Sphere {
                    radius: value(radius, params)?,
                    surface: false,
                },
                ["sphere", radius, "surface"] => Shape::Sphere {
                    radius: value(radius, params)?,
                    surface: true,
                },
                ["box", sizes @ ..] => Shape::Box {
                    size: values(sizes, params, "box")?,
                },
                _ => return Err("shape takes point, sphere RADIUS [surface] or box X Y Z".into()),
            };
        }
        "offset" => {
            no_options(words)?;
            emitter.offset = values(plain, params, "offset")?;
        }
        "lifetime" => {
            no_options(words)?;
            emitter.lifetime = range(plain, params, "lifetime")?;
        }
        "velocity" => {
            words.only(keyword, &["cone", "spread"])?;
            emitter.velocity = values(plain, params, "velocity")?;
            if let Some(v) = words.option("cone") {
                emitter.cone = single(v, params, "cone")?;
            }
            if let Some(v) = words.option("spread") {
                emitter.spread = single(v, params, "spread")?;
            }
        }
        "size" => match plain.first() {
            Some(&"life") => {
                let rest = Words {
                    plain: plain[1..].to_vec(),
                    options: words.options.clone(),
                };
                emitter.size_life = curve(&rest, "size life")?;
            }
            _ => {
                no_options(words)?;
                emitter.size = range(plain, params, "size")?;
            }
        },
        "color" => match plain.first() {
            Some(&"life") => {
                let rest = Words {
                    plain: plain[1..].to_vec(),
                    options: words.options.clone(),
                };
                emitter.color_life = curve(&rest, "color life")?;
            }
            _ => {
                no_options(words)?;
                emitter.color = match plain.len() {
                    3 => {
                        let [r, g, b] = values(plain, params, "color")?;
                        [r, g, b, Value::constant(1.0)]
                    }
                    _ => values(plain, params, "color")?,
                };
            }
        },
        "brightness" => {
            no_options(words)?;
            emitter.brightness = single(plain, params, "brightness")?;
        }
        "temperature" => {
            no_options(words)?;
            emitter.temperature = range(plain, params, "temperature")?;
        }
        "cooling" => {
            no_options(words)?;
            emitter.cooling = single(plain, params, "cooling")?;
        }
        "glow" => {
            no_options(words)?;
            emitter.glow = single(plain, params, "glow")?;
        }
        "gravity" => {
            no_options(words)?;
            emitter.gravity = match plain {
                [] => Value::constant(STANDARD_GRAVITY),
                _ => single(plain, params, "gravity")?,
            };
        }
        "wind" => {
            no_options(words)?;
            emitter.wind = single(plain, params, "wind")?;
        }
        "curl" => {
            words.only(keyword, &["scale", "change"])?;
            emitter.curl[0] = single(plain, params, "curl")?;
            if let Some(v) = words.option("scale") {
                emitter.curl[1] = single(v, params, "scale")?;
            }
            if let Some(v) = words.option("change") {
                emitter.curl[2] = single(v, params, "change")?;
            }
        }
        "buoyancy" => {
            no_options(words)?;
            emitter.buoyancy = single(plain, params, "buoyancy")?;
        }
        "attract" => {
            no_options(words)?;
            emitter.attract = single(plain, params, "attract")?;
        }
        "vortex" => {
            no_options(words)?;
            emitter.vortex = single(plain, params, "vortex")?;
        }
        "collide" => {
            words.only(keyword, &["restitution", "friction", "event"])?;
            let mut responses = 0;
            let mut collide = Collide {
                against: 0,
                response: Response::Bounce,
                restitution: Value::constant(0.4),
                friction: Value::constant(0.2),
                event: None,
            };
            for word in plain {
                match *word {
                    "depth" => collide.against |= COLLIDE_DEPTH,
                    "field" => collide.against |= COLLIDE_FIELD,
                    "water" => collide.against |= COLLIDE_WATER,
                    "bounce" | "stick" | "kill" => {
                        responses += 1;
                        collide.response = match *word {
                            "bounce" => Response::Bounce,
                            "stick" => Response::Stick,
                            _ => Response::Kill,
                        };
                    }
                    _ => {
                        return Err(format!(
                            "collide takes depth, field or water, then bounce, stick or kill, not {word:?}"
                        ))
                    }
                }
            }
            if collide.against == 0 {
                return Err("collide needs something to hit: depth, field or water".into());
            }
            if responses > 1 {
                return Err("collide takes one of bounce, stick or kill".into());
            }
            if let Some(v) = words.option("restitution") {
                collide.restitution = single(v, params, "restitution")?;
            }
            if let Some(v) = words.option("friction") {
                collide.friction = single(v, params, "friction")?;
            }
            if let Some(v) = words.option("event") {
                let [name] = v else {
                    return Err("event takes a name".into());
                };
                if !is_name(name) {
                    return Err(format!("{name:?} isn't a name for an event"));
                }
                collide.event = Some((*name).to_owned());
            }
            emitter.collide = Some(collide);
        }
        "drag" => {
            no_options(words)?;
            emitter.drag = single(plain, params, "drag")?;
        }
        "render" => {
            words.only(keyword, &["blend", "soft", "opacity", "shading", "phase"])?;
            match plain {
                ["sprite", flags @ ..] => {
                    for flag in flags {
                        match *flag {
                            "solid" => emitter.solid = true,
                            "sort" => emitter.sort = true,
                            _ => return Err(format!("render sprite has no {flag:?}")),
                        }
                    }
                }
                _ => return Err("render takes sprite, then solid or sort".into()),
            }
            if let Some(v) = words.option("blend") {
                emitter.blend = match v {
                    ["alpha"] => Blend::Alpha,
                    ["add"] => Blend::Add,
                    _ => return Err("blend takes alpha or add".into()),
                };
            }
            if let Some(v) = words.option("soft") {
                emitter.soft = single(v, params, "soft")?;
            }
            if let Some(v) = words.option("opacity") {
                emitter.opacity = single(v, params, "opacity")?;
            }
            if let Some(v) = words.option("shading") {
                emitter.shading = match v {
                    ["lit"] => Shading::Lit,
                    ["unlit"] => Shading::Unlit,
                    _ => return Err("shading takes lit or unlit".into()),
                };
            }
            if let Some(v) = words.option("phase") {
                emitter.phase = single(v, params, "phase")?;
            }
        }
        _ => return Err(format!("unknown module {keyword:?}")),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SMOKE: &str = "
        # A test.
        param heat 0.5
        param wind 2

        emitter puffs
          budget 500
          spawn rate=40*heat burst=10 every=2
          shape box 1 2 heat
          offset 0 0.5 0
          lifetime 3 4
          velocity 0 1 0 cone=20 spread=0.2
          size 0.2 0.4
          color 0.5 0.5 0.5 0.8
          gravity -0.5
          drag wind
          color life 0=1 1 1 0 0.2=1 1 1 1 1=0.4 0.4 0.4 0
          size life 0=0.5 1=2
          temperature 1200 1500
          cooling 0.5
          glow 0.3
          render sprite blend=add soft=0.5 opacity=0.7 shading=unlit phase=0.6
        emitter embers
          follow camera
          shape sphere 2 surface
    ";

    #[test]
    fn every_module_reads_back() {
        let effect = Effect::parse(SMOKE).unwrap();
        assert_eq!(
            effect.params,
            vec![("heat".into(), 0.5), ("wind".into(), 2.0)]
        );
        let e = &effect.emitters[0];
        let params = [0.5, 2.0];
        assert_eq!(e.budget, 500);
        assert_eq!(e.rate.eval(&params), 20.0);
        assert_eq!(e.burst.eval(&params), 10.0);
        assert_eq!(e.every.eval(&params), 2.0);
        let Shape::Box { size } = e.shape else {
            panic!("expected a box")
        };
        assert_eq!(size.map(|v| v.eval(&params)), [1.0, 2.0, 0.5]);
        assert_eq!(e.offset.map(|v| v.eval(&params)), [0.0, 0.5, 0.0]);
        assert_eq!(e.lifetime.map(|v| v.eval(&params)), [3.0, 4.0]);
        assert_eq!(e.velocity.map(|v| v.eval(&params)), [0.0, 1.0, 0.0]);
        assert_eq!(e.cone.eval(&params), 20.0);
        assert_eq!(e.spread.eval(&params), 0.2);
        assert_eq!(e.size.map(|v| v.eval(&params)), [0.2, 0.4]);
        assert_eq!(e.color.map(|v| v.eval(&params)), [0.5, 0.5, 0.5, 0.8]);
        assert_eq!(e.gravity.eval(&params), -0.5);
        assert_eq!(e.drag.eval(&params), 2.0);
        let mid = e.color_life.at(0.6);
        for (got, want) in mid.iter().zip([0.7, 0.7, 0.7, 0.5]) {
            assert!((got - want).abs() < 1.0e-6, "{mid:?}");
        }
        assert_eq!(e.size_life.at(0.5), [1.25]);
        assert_eq!(e.blend, Blend::Add);
        assert_eq!(e.soft.eval(&params), 0.5);
        assert_eq!(e.opacity.eval(&params), 0.7);
        assert_eq!(e.shading, Shading::Unlit);
        assert_eq!(e.phase.eval(&params), 0.6);
        assert_eq!(e.temperature.map(|v| v.eval(&params)), [1200.0, 1500.0]);
        assert_eq!(e.cooling.eval(&params), 0.5);
        assert_eq!(e.glow.eval(&params), 0.3);

        let embers = &effect.emitters[1];
        assert_eq!(embers.follow, Follow::Camera);
        assert!(matches!(embers.shape, Shape::Sphere { surface: true, .. }));
        // Defaults for all it does not name.
        assert_eq!(embers.budget, 1000);
        assert_eq!(embers.shading, Shading::Lit);
        assert_eq!(embers.temperature.map(|v| v.eval(&params)), [0.0, 0.0]);
        assert_eq!(embers.gravity.eval(&params), 0.0);
    }

    #[test]
    fn forces_and_collisions_read_back() {
        let effect = Effect::parse(
            "
            param gust 2
            emitter drops
              wind 0.5*gust
              curl 1.5 scale=3 change=0.4
              buoyancy 0.8
              attract 2
              vortex 0.5
              collide depth water kill event=splash
            emitter splash
              spawn event=splash count=4
              collide field stick restitution=0.2 friction=0.9
            ",
        )
        .unwrap();
        let params = [2.0];
        let drops = &effect.emitters[0];
        assert_eq!(drops.wind.eval(&params), 1.0);
        assert_eq!(drops.curl.map(|v| v.eval(&params)), [1.5, 3.0, 0.4]);
        assert_eq!(drops.buoyancy.eval(&params), 0.8);
        assert_eq!(drops.attract.eval(&params), 2.0);
        assert_eq!(drops.vortex.eval(&params), 0.5);
        let collide = drops.collide.as_ref().unwrap();
        assert_eq!(collide.against, COLLIDE_DEPTH | COLLIDE_WATER);
        assert_eq!(collide.response, Response::Kill);
        assert_eq!(collide.event.as_deref(), Some("splash"));
        let splash = &effect.emitters[1];
        let (name, count) = splash.on_event.as_ref().unwrap();
        assert_eq!((name.as_str(), count.eval(&params)), ("splash", 4.0));
        let collide = splash.collide.as_ref().unwrap();
        assert_eq!(
            (collide.against, collide.response),
            (COLLIDE_FIELD, Response::Stick)
        );
        assert_eq!(collide.restitution.eval(&params), 0.2);
        assert_eq!(collide.friction.eval(&params), 0.9);
    }

    #[test]
    fn parameters_drive_values_as_they_change() {
        let effect = Effect::parse(SMOKE).unwrap();
        let rate = effect.emitters[0].rate;
        assert_eq!(rate.eval(&[0.0, 2.0]), 0.0);
        assert_eq!(rate.eval(&[1.0, 2.0]), 40.0);
        // A number either side of the parameter.
        let effect = Effect::parse("param p 1\nemitter e\n spawn rate=p*3").unwrap();
        assert_eq!(effect.emitters[0].rate.eval(&[2.0]), 6.0);
    }

    #[test]
    fn errors_name_the_line() {
        let cases = [
            ("emitter e\n  speed 3", "line 2: unknown module"),
            ("emitter e\n  size 1 2 3", "line 2: size takes one value"),
            (
                "emitter e\n  spawn rate=heat",
                "line 2: \"heat\" is neither",
            ),
            ("emitter e\nparam p 1", "line 2: parameters come before"),
            (
                "emitter e\n collide depth bounce stick",
                "line 2: collide takes one of",
            ),
            (
                "emitter e\n collide depth event=2b",
                "line 2: \"2b\" isn't a name",
            ),
            (
                "emitter a\n collide depth event=x\nemitter b\n spawn event=x count=1\nemitter c\n spawn event=x count=1",
                "emitters b and c both spawn on event x",
            ),
            ("budget 3", "line 1: budget must come after"),
            (
                "emitter e\n color life 0.5=1 1 1 1 0.2=1 1 1 1",
                "line 2: color life: keys must",
            ),
            (
                "emitter e\n render sprite glow=1",
                "line 2: render has no option glow",
            ),
            ("emitter e\n budget 0", "line 2: \"0\" is not a count"),
            (
                "emitter e\nemitter e",
                "line 2: emitter e is declared twice",
            ),
            (
                "emitter e\n render sprite glossy",
                "line 2: render sprite has no \"glossy\"",
            ),
        ];
        for (text, expected) in cases {
            let error = Effect::parse(text).unwrap_err();
            assert!(error.starts_with(expected), "{text:?} gave {error:?}");
        }
        assert!(Effect::parse("# nothing\n").is_err());
        // An event someone sends must be heard.
        let error = Effect::parse("emitter rain\n collide water kill event=splash").unwrap_err();
        assert!(error.contains("no emitter spawns on it"), "{error}");
        let error = Effect::parse("emitter e\n collide bounce").unwrap_err();
        assert!(error.contains("needs something to hit"), "{error}");
        // Sorting is done in one workgroup's memory, which bounds the budget.
        let error = Effect::parse("emitter smoke\n budget 5000\n render sprite sort").unwrap_err();
        assert!(error.contains("at most 4096"), "{error}");
        let sorted =
            Effect::parse("emitter smoke\n budget 4096\n render sprite sort solid").unwrap();
        assert!(sorted.emitters[0].sort && sorted.emitters[0].solid);
    }

    #[test]
    fn curves_bake_their_ends_and_interpolate_between_keys() {
        let curve = Curve::<1> {
            keys: vec![(0.25, [2.0]), (0.75, [4.0])],
        };
        let baked = curve.bake();
        assert_eq!(baked[0], [2.0]);
        assert_eq!(baked[CURVE_ENTRIES - 1], [4.0]);
        assert!((curve.at(0.5)[0] - 3.0).abs() < 1.0e-6);
    }

    #[test]
    fn built_in_effects_parse() {
        for name in Effect::names() {
            assert!(Effect::by_name(name).is_some(), "{name}");
        }
        assert!(Effect::by_name("nothing").is_none());
    }
}
