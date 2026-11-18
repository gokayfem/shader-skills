//! State machines: which clips play, and when they cross-fade into one another.
//!
//! ```text
//! # scenes/harbour/walker.anim
//! source ../../target/characters/keeper_clips.glb     # the glTF the clips are in; several can be
//!                                                     # listed, and the first with a clip has it
//! roles ual.roles keeper.roles                        # its rig's, and the character's (optional)
//! clip idle Idle                                      # a name here, and in the file
//! clip walk Walk
//! clip run Run
//! notify walk 0.05 step_l                             # sent to scripts as `on_anim(name)`
//! notify walk 0.55 step_r
//! param speed 0
//! state idle clip idle                                # plays a clip
//! state moving blend1d speed 0 idle 1.4 walk 4.0 run  # blends by a parameter
//! state aiming blend2d dx dy 0 0 idle 1 0 walk 0 1 run  # blends by two
//! start idle
//! transition idle moving speed > 0.1 fade 0.25        # from, to, when, how long
//! transition moving idle speed < 0.1 fade 0.3
//! transition * idle dead == 1 fade 0.1                # from any state
//! ```
//!
//! Every clip of a blend shares one phase, so blended walk and run cycles keep their feet in step
//! (Unreal's sync groups).

use crate::clip::{duration, sample};
use crate::pose::{blend, Pose};
use solstice_asset::Clip;
use std::collections::HashMap;

/// What a state plays.
#[derive(Clone, Debug, PartialEq)]
pub enum Motion {
    Clip(String),
    /// A parameter, and each clip at its place along it.
    Blend1d(String, Vec<(f32, String)>),
    /// Two parameters, and each clip at its place in their plane.
    Blend2d(String, String, Vec<([f32; 2], String)>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Compare {
    Less,
    Greater,
    Equal,
    NotEqual,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Transition {
    pub from: Option<String>,
    pub to: String,
    param: String,
    compare: Compare,
    value: f32,
    pub fade: f32,
}

impl Transition {
    fn holds(&self, value: f32) -> bool {
        match self.compare {
            Compare::Less => value < self.value,
            Compare::Greater => value > self.value,
            Compare::Equal => (value - self.value).abs() < 1.0e-6,
            Compare::NotEqual => (value - self.value).abs() >= 1.0e-6,
        }
    }
}

/// A clip's name in the graph, its name in the source file, and whether it loops.
#[derive(Clone, Debug, PartialEq)]
pub struct ClipLine {
    pub name: String,
    pub source_name: String,
    pub looping: bool,
}

/// A parsed `.anim` file.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AnimGraph {
    /// The glTFs the clips are in, relative to this file. A clip comes from the first that has
    /// it.
    pub sources: Vec<String>,
    /// The role files of the source's rig and the character's, for retargeting. Without them,
    /// bones match by name.
    pub roles: Option<(String, String)>,
    pub clips: Vec<ClipLine>,
    pub notifies: Vec<(String, f32, String)>,
    pub params: Vec<(String, f32)>,
    pub states: Vec<(String, Motion)>,
    pub transitions: Vec<Transition>,
    pub start: String,
}

fn number(word: &str) -> Result<f32, String> {
    word.parse::<f32>()
        .ok()
        .filter(|value| value.is_finite())
        .ok_or_else(|| format!("`{word}` is not a number"))
}

impl AnimGraph {
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut graph = AnimGraph::default();
        for (index, line) in text.lines().enumerate() {
            let at = |m: String| format!("line {}: {m}", index + 1);
            let words: Vec<&str> = line
                .split('#')
                .next()
                .unwrap_or("")
                .split_whitespace()
                .collect();
            let Some((&keyword, rest)) = words.split_first() else {
                continue;
            };
            match (keyword, rest) {
                ("source", [path]) => graph.sources.push((*path).to_owned()),
                ("roles", [source, character]) => {
                    graph.roles = Some(((*source).to_owned(), (*character).to_owned()))
                }
                ("clip", [name, source] | [name, source, "loop"]) => graph.clips.push(ClipLine {
                    name: (*name).to_owned(),
                    source_name: (*source).to_owned(),
                    looping: true,
                }),
                ("clip", [name, source, "once"]) => graph.clips.push(ClipLine {
                    name: (*name).to_owned(),
                    source_name: (*source).to_owned(),
                    looping: false,
                }),
                ("notify", [clip, time, name]) => graph.notifies.push((
                    (*clip).to_owned(),
                    number(time).map_err(at)?,
                    (*name).to_owned(),
                )),
                ("param", [name, value]) => graph
                    .params
                    .push(((*name).to_owned(), number(value).map_err(at)?)),
                ("state", [name, "clip", clip]) => graph
                    .states
                    .push(((*name).to_owned(), Motion::Clip((*clip).to_owned()))),
                ("state", [name, "blend1d", param, points @ ..]) => {
                    if points.is_empty() || !points.len().is_multiple_of(2) {
                        return Err(at("a blend1d lists position and clip, in pairs".to_owned()));
                    }
                    let mut list = Vec::new();
                    for pair in points.chunks_exact(2) {
                        list.push((number(pair[0]).map_err(at)?, pair[1].to_owned()));
                    }
                    list.sort_by(|a, b| a.0.total_cmp(&b.0));
                    graph.states.push((
                        (*name).to_owned(),
                        Motion::Blend1d((*param).to_owned(), list),
                    ));
                }
                ("state", [name, "blend2d", x, y, points @ ..]) => {
                    if points.is_empty() || !points.len().is_multiple_of(3) {
                        return Err(at("a blend2d lists x, y and clip, in threes".to_owned()));
                    }
                    let mut list = Vec::new();
                    for triple in points.chunks_exact(3) {
                        list.push((
                            [
                                number(triple[0]).map_err(at)?,
                                number(triple[1]).map_err(at)?,
                            ],
                            triple[2].to_owned(),
                        ));
                    }
                    graph.states.push((
                        (*name).to_owned(),
                        Motion::Blend2d((*x).to_owned(), (*y).to_owned(), list),
                    ));
                }
                ("start", [name]) => graph.start = (*name).to_owned(),
                ("transition", [from, to, param, op, value, "fade", fade]) => {
                    let compare = match *op {
                        "<" => Compare::Less,
                        ">" => Compare::Greater,
                        "==" => Compare::Equal,
                        "!=" => Compare::NotEqual,
                        other => return Err(at(format!("`{other}` is not <, >, == or !="))),
                    };
                    graph.transitions.push(Transition {
                        from: (*from != "*").then(|| (*from).to_owned()),
                        to: (*to).to_owned(),
                        param: (*param).to_owned(),
                        compare,
                        value: number(value).map_err(at)?,
                        fade: number(fade).map_err(at)?.max(0.0),
                    });
                }
                _ => return Err(at(format!("didn't understand `{}`", words.join(" ")))),
            }
        }
        graph.check()?;
        Ok(graph)
    }

    fn check(&self) -> Result<(), String> {
        if self.states.is_empty() {
            return Err("a graph needs at least one state".to_owned());
        }
        let has_state = |name: &str| self.states.iter().any(|(n, _)| n == name);
        let has_clip = |name: &str| self.clips.iter().any(|c| c.name == name);
        let has_param = |name: &str| self.params.iter().any(|(n, _)| n == name);
        if !has_state(&self.start) {
            return Err(format!("`start` names no state: `{}`", self.start));
        }
        let mut clip_names = Vec::new();
        let looping = |name: &str| self.clips.iter().any(|c| c.name == name && c.looping);
        for (name, motion) in &self.states {
            let mut uses = Vec::new();
            match motion {
                Motion::Clip(c) => clip_names.push(c),
                Motion::Blend1d(param, points) => {
                    uses.push(param);
                    clip_names.extend(points.iter().map(|p| &p.1));
                }
                Motion::Blend2d(x, y, points) => {
                    uses.extend([x, y]);
                    clip_names.extend(points.iter().map(|p| &p.1));
                }
            }
            // A blend shares one phase, so its clips must all wrap around or all hold their ends.
            let points: Vec<&String> = match motion {
                Motion::Clip(_) => Vec::new(),
                Motion::Blend1d(_, points) => points.iter().map(|p| &p.1).collect(),
                Motion::Blend2d(_, _, points) => points.iter().map(|p| &p.1).collect(),
            };
            if !points.iter().all(|c| looping(c)) && points.iter().any(|c| looping(c)) {
                return Err(format!(
                    "state `{name}` blends clips that loop with clips that don't"
                ));
            }
            if let Some(param) = uses.iter().find(|p| !has_param(p)) {
                return Err(format!(
                    "state `{name}` blends by `{param}`: no `param` line names it"
                ));
            }
            if clip_names.iter().any(|c| !has_clip(c)) {
                return Err(format!("state `{name}` uses a clip no `clip` line names"));
            }
        }
        for transition in &self.transitions {
            if !has_state(&transition.to) || transition.from.as_ref().is_some_and(|f| !has_state(f))
            {
                return Err("a transition names a state that doesn't exist".to_owned());
            }
            if !has_param(&transition.param) {
                return Err(format!(
                    "a transition reads `{}`: no `param` line names it",
                    transition.param
                ));
            }
        }
        for (clip, _, name) in &self.notifies {
            if !has_clip(clip) {
                return Err(format!("notify `{name}` names no clip: `{clip}`"));
            }
        }
        Ok(())
    }
}

/// A graph's clips, resolved to a loaded clip by the graph's own names.
pub struct ClipSet {
    clips: HashMap<String, usize>,
    pub loaded: Vec<Clip>,
}

impl ClipSet {
    pub fn new(names: Vec<(String, Clip)>) -> Self {
        let mut clips = HashMap::new();
        let mut loaded = Vec::with_capacity(names.len());
        for (name, clip) in names {
            clips.insert(name, loaded.len());
            loaded.push(clip);
        }
        Self { clips, loaded }
    }

    fn get(&self, name: &str) -> &Clip {
        &self.loaded[self.clips[name]]
    }

    pub fn contains(&self, name: &str) -> bool {
        self.clips.contains_key(name)
    }
}

/// A state as it runs: which one, and how far through it is, from 0 to 1.
#[derive(Clone, Copy, Debug)]
struct Run {
    state: usize,
    phase: f32,
}

/// One clip of a state and its share of the blend, sorted out from the parameters.
fn shares_of(motion: &Motion, value: &dyn Fn(&str) -> f32) -> Vec<(String, f32)> {
    match motion {
        Motion::Clip(c) => vec![(c.clone(), 1.0)],
        Motion::Blend1d(param, points) => {
            let v = value(param);
            let first = points[0].0;
            let last = points[points.len() - 1].0;
            if v.is_nan() || v <= first {
                return vec![(points[0].1.clone(), 1.0)];
            }
            if v >= last {
                return vec![(points[points.len() - 1].1.clone(), 1.0)];
            }
            let at = points
                .windows(2)
                .position(|w| v >= w[0].0 && v <= w[1].0)
                .unwrap_or(0);
            let (lo, hi) = (&points[at], &points[at + 1]);
            let t = (v - lo.0) / (hi.0 - lo.0).max(1.0e-6);
            vec![(lo.1.clone(), 1.0 - t), (hi.1.clone(), t)]
        }
        Motion::Blend2d(x, y, points) => {
            let p = [value(x), value(y)];
            let mut weights: Vec<(String, f32)> = points
                .iter()
                .map(|(place, clip)| {
                    let d2 = (place[0] - p[0]).powi(2) + (place[1] - p[1]).powi(2);
                    (clip.clone(), if d2 < 1.0e-8 { 1.0e8 } else { 1.0 / d2 })
                })
                .collect();
            let total: f32 = weights.iter().map(|w| w.1).sum();
            for w in &mut weights {
                w.1 /= total;
            }
            weights
        }
    }
}

/// What a fade comes from.
#[derive(Clone, Copy, Debug)]
enum FadeFrom {
    /// A state still running as it fades out.
    Running(Run),
    /// The pose as it stood: a state changing in the middle of a fade carries on from there.
    Frozen,
}

/// A graph, running: parameters, the state playing, and any fade under way.
pub struct Animator {
    pub graph: AnimGraph,
    params: Vec<f32>,
    current: Run,
    fade: Option<(FadeFrom, f32, f32)>,
    scratch: Pose,
    blended: Pose,
    last: Pose,
    /// Whether `last` holds anything yet.
    evaluated: bool,
    /// What a [`FadeFrom::Frozen`] fade comes from.
    frozen: Pose,
}

impl Animator {
    pub fn new(graph: AnimGraph, bones: usize) -> Self {
        let params = graph.params.iter().map(|p| p.1).collect();
        let start = graph
            .states
            .iter()
            .position(|(n, _)| *n == graph.start)
            .unwrap_or(0);
        Self {
            graph,
            params,
            current: Run {
                state: start,
                phase: 0.0,
            },
            fade: None,
            scratch: Pose::with_bones(bones),
            blended: Pose::with_bones(bones),
            last: Pose::with_bones(bones),
            evaluated: false,
            frozen: Pose::with_bones(bones),
        }
    }

    pub fn state_name(&self) -> &str {
        &self.graph.states[self.current.state].0
    }

    /// Sets a parameter by name. Returns false if the graph has no such parameter, or the value
    /// isn't a number.
    pub fn set(&mut self, param: &str, value: f32) -> bool {
        if !value.is_finite() {
            return false;
        }
        match self.graph.params.iter().position(|(n, _)| n == param) {
            Some(index) => {
                self.params[index] = value;
                true
            }
            None => false,
        }
    }

    /// Fades to a state now, whatever the transitions say.
    pub fn go_to(&mut self, state: &str, fade: f32) -> bool {
        match self.graph.states.iter().position(|(n, _)| n == state) {
            Some(index) => {
                self.begin(index, fade);
                true
            }
            None => false,
        }
    }

    fn begin(&mut self, state: usize, fade: f32) {
        if state == self.current.state {
            return;
        }
        // A locomotion cycle stays in step across a fade; anything else starts from its start.
        let phase = if self.state_loops(self.current.state) && self.state_loops(state) {
            self.current.phase
        } else {
            0.0
        };
        let from = if self.fade.is_some() {
            // With nothing drawn yet, there is no pose to carry on from: no fade at all. Only
            // the fade: where the state starts mustn't depend on what has been drawn.
            if !self.evaluated {
                self.fade = None;
                self.current = Run { state, phase };
                return;
            }
            self.frozen.copy_from(&self.last);
            FadeFrom::Frozen
        } else {
            FadeFrom::Running(self.current)
        };
        self.fade = (fade > 0.0).then_some((from, 0.0, fade));
        self.current = Run { state, phase };
    }

    /// Whether every clip of a state loops, which is what makes it a cycle.
    fn state_loops(&self, state: usize) -> bool {
        let loops = |name: &str| {
            self.graph
                .clips
                .iter()
                .any(|clip| clip.name == name && clip.looping)
        };
        match &self.graph.states[state].1 {
            Motion::Clip(c) => loops(c),
            Motion::Blend1d(_, points) => points.iter().all(|p| loops(&p.1)),
            Motion::Blend2d(_, _, points) => points.iter().all(|p| loops(&p.1)),
        }
    }

    fn value(&self, name: &str) -> f32 {
        self.graph
            .params
            .iter()
            .position(|(n, _)| n == name)
            .map_or(0.0, |i| self.params[i])
    }

    /// How long a state's cycle lasts now: its clips' lengths, by their shares, so a blend's feet
    /// keep the blended pace.
    fn cycle_seconds(&self, clips: &ClipSet, state: usize) -> f32 {
        let value = |name: &str| self.value(name);
        let shares = shares_of(&self.graph.states[state].1, &value);
        shares
            .iter()
            .map(|(clip, share)| duration(clips.get(clip)) * share)
            .sum::<f32>()
            .max(1.0e-3)
    }

    /// Moves on by `seconds`: takes any transition that holds, advances the phases, and returns
    /// the notifies the dominant clip crossed.
    pub fn advance(&mut self, seconds: f32, clips: &ClipSet) -> Vec<String> {
        let here = self.graph.states[self.current.state].0.clone();
        for transition in &self.graph.transitions.clone() {
            let from_here = transition.from.as_ref().is_none_or(|f| *f == here);
            if from_here && transition.to != here && transition.holds(self.value(&transition.param))
            {
                let to = self
                    .graph
                    .states
                    .iter()
                    .position(|(n, _)| *n == transition.to)
                    .expect("checked when parsed");
                self.begin(to, transition.fade);
                break;
            }
        }

        if let Some((from, elapsed, length)) = self.fade {
            let from = match from {
                FadeFrom::Running(run) => {
                    let cycle = self.cycle_seconds(clips, run.state);
                    FadeFrom::Running(Run {
                        state: run.state,
                        phase: run.phase + seconds / cycle,
                    })
                }
                FadeFrom::Frozen => FadeFrom::Frozen,
            };
            let elapsed = elapsed + seconds;
            self.fade = (elapsed < length).then_some((from, elapsed, length));
        }

        let cycle = self.cycle_seconds(clips, self.current.state);
        let before = self.current.phase;
        let mut after = before + seconds / cycle;
        let dominant = self.dominant_clip();
        let clip = clips.get(&dominant);
        if !clip.loops {
            after = after.min(1.0);
        }
        let notifies = self.crossed(clip, &dominant, before, after);
        if clip.loops {
            after %= 1.0;
        } else {
            after = after.min(1.0);
        }
        self.current.phase = after;
        notifies
    }

    fn dominant_clip(&self) -> String {
        let value = |name: &str| self.value(name);
        shares_of(&self.graph.states[self.current.state].1, &value)
            .into_iter()
            .max_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(clip, _)| clip)
            .expect("states have at least one clip")
    }

    fn crossed(&self, clip: &Clip, name: &str, before: f32, after: f32) -> Vec<String> {
        let length = duration(clip).max(1.0e-3);
        let mut out = Vec::new();
        for (clip_name, time, notify) in &self.graph.notifies {
            if clip_name != name {
                continue;
            }
            let at = time / length;
            if clip.loops {
                // A long step can cross it several times; a time at the very end is the start.
                let at = at.rem_euclid(1.0);
                for cycle in 0..=after.floor() as u32 {
                    let at = at + cycle as f32;
                    if at >= before && at < after {
                        out.push(notify.clone());
                    }
                }
            } else {
                // A clip that holds its end reaches it once.
                let at = at.min(1.0);
                if (at >= before && at < after) || (at >= 1.0 && before < 1.0 && after >= 1.0) {
                    out.push(notify.clone());
                }
            }
        }
        out
    }

    /// The current pose, into `out`.
    pub fn evaluate(&mut self, clips: &ClipSet, rest: &Pose, out: &mut Pose) {
        self.evaluate_ahead(clips, rest, out, 0.0);
    }

    /// Where a run will have got to `seconds` on, without moving it.
    fn run_ahead(&self, run: Run, seconds: f32, clips: &ClipSet) -> Run {
        let phase = run.phase + seconds / self.cycle_seconds(clips, run.state);
        let phase = if self.state_loops(run.state) {
            phase % 1.0
        } else {
            phase.min(1.0)
        };
        Run { phase, ..run }
    }

    /// The pose `ahead` seconds after the last step, into `out`. Time moves on in fixed steps, so
    /// that a game replays exactly at any frame rate; this keeps what is drawn smooth between
    /// them. It doesn't move anything on, so a state change waits for the next step.
    pub fn evaluate_ahead(&mut self, clips: &ClipSet, rest: &Pose, out: &mut Pose, ahead: f32) {
        let current_run = self.run_ahead(self.current, ahead, clips);
        let fade = self.fade.map(|(from, elapsed, length)| {
            let from = match from {
                FadeFrom::Running(run) => FadeFrom::Running(self.run_ahead(run, ahead, clips)),
                FadeFrom::Frozen => FadeFrom::Frozen,
            };
            (from, elapsed + ahead, length)
        });
        let graph = std::mem::take(&mut self.graph);
        let params = std::mem::take(&mut self.params);
        let value = |name: &str| {
            graph
                .params
                .iter()
                .position(|(n, _)| n == name)
                .map_or(0.0, |i| params[i])
        };
        let current = shares_of(&graph.states[current_run.state].1, &value);
        Self::pose_of(
            &current,
            current_run.phase,
            clips,
            rest,
            out,
            &mut self.scratch,
        );
        if let Some((from, elapsed, length)) = fade {
            let t = (elapsed / length).clamp(0.0, 1.0);
            match from {
                FadeFrom::Running(run) => {
                    let older = shares_of(&graph.states[run.state].1, &value);
                    Self::pose_of(
                        &older,
                        run.phase,
                        clips,
                        rest,
                        &mut self.blended,
                        &mut self.scratch,
                    );
                }
                FadeFrom::Frozen => self.blended.copy_from(&self.frozen),
            }
            // From the old state towards the new one.
            let finished = out.clone();
            out.copy_from(&self.blended);
            blend(out, &finished, t, None);
        }
        self.params = params;
        self.graph = graph;
        self.last.copy_from(out);
        self.evaluated = true;
    }

    fn pose_of(
        shares: &[(String, f32)],
        phase: f32,
        clips: &ClipSet,
        rest: &Pose,
        out: &mut Pose,
        scratch: &mut Pose,
    ) {
        out.copy_from(rest);
        let mut filled = 0.0f32;
        for (clip, share) in shares {
            if *share <= 0.0 {
                continue;
            }
            let c = clips.get(clip);
            sample(c, phase * duration(c), scratch);
            let t = share / (filled + share);
            if filled == 0.0 {
                out.copy_from(scratch);
            } else {
                blend(out, scratch, t, None);
            }
            filled += share;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clip::{compress, Frames};
    use glam::{Quat, Vec3};

    const GRAPH: &str = "
        clip idle I
        clip walk W
        clip run R
        notify walk 0.5 step
        param speed 0
        state idle clip idle
        state moving blend1d speed 0 idle 2 walk 4 run
        start idle
        transition idle moving speed > 0.1 fade 0.2
        transition moving idle speed < 0.1 fade 0.2
    ";

    fn clip(name: &str, seconds: u32, height: f32) -> Clip {
        let frames = (seconds as usize) * 30;
        compress(
            name,
            0,
            &Frames {
                rotations: vec![vec![Quat::IDENTITY]; frames],
                translations: (0..frames)
                    .map(|i| vec![Vec3::Y * height * (i as f32 / frames as f32)])
                    .collect(),
            },
            true,
        )
    }

    fn set() -> ClipSet {
        ClipSet::new(vec![
            ("idle".to_owned(), clip("idle", 2, 0.0)),
            ("walk".to_owned(), clip("walk", 1, 2.0)),
            ("run".to_owned(), clip("run", 1, 4.0)),
        ])
    }

    #[test]
    fn sources_are_all_kept_in_order() {
        let graph = AnimGraph::parse(
            "source a.glb
source b.glb
clip idle I
state idle clip idle
start idle
",
        )
        .unwrap();
        assert_eq!(graph.sources, ["a.glb", "b.glb"]);
    }

    #[test]
    fn it_parses_and_refuses_mistakes() {
        let graph = AnimGraph::parse(GRAPH).unwrap();
        assert_eq!(graph.states.len(), 2);
        for (bad, says) in [
            ("clip a A\nstate a clip b\nstart a", "uses a clip"),
            ("clip a A\nstate a clip a\nstart b", "start"),
            ("state a blend1d x 1\nstart a", "pairs"),
            ("state", "understand"),
            (
                "clip a A\nparam x 0\nstate a clip a\nstart a\ntransition a b x > 1 fade 0",
                "doesn't exist",
            ),
            ("clip a A\nstate s blend1d x NaN a\nstart s", "not a number"),
            (
                "clip a A\nstate a clip a\nstart a\ntransition a a typo > 1 fade 0",
                "typo",
            ),
            ("clip a A\nstate s blend1d x 0 a\nstart s", "`x`"),
        ] {
            let error = AnimGraph::parse(bad).unwrap_err();
            assert!(error.contains(says), "{error}");
        }
    }

    #[test]
    fn a_blend_space_shares_by_where_the_parameter_lies() {
        let graph = AnimGraph::parse(GRAPH).unwrap();
        let (_, motion) = &graph.states[1];
        let value = |v: f32| shares_of(motion, &move |_: &str| v);
        assert_eq!(value(0.0), [("idle".to_owned(), 1.0)]);
        let half = value(3.0);
        assert_eq!(half.len(), 2);
        assert!((half[0].1 - 0.5).abs() < 1.0e-5 && half[0].0 == "walk");
        assert_eq!(value(99.0), [("run".to_owned(), 1.0)]);
    }

    #[test]
    fn a_2d_space_goes_wholly_to_the_clip_at_that_spot() {
        let graph = AnimGraph::parse(
            "clip a A\nclip b B\nparam x 0\nparam y 0\nstate s blend2d x y 0 0 a 1 0 b\nstart s",
        )
        .unwrap();
        let (_, motion) = &graph.states[0];
        let at =
            |p: [f32; 2]| shares_of(motion, &move |n: &str| if n == "x" { p[0] } else { p[1] });
        assert!(at([1.0, 0.0]).iter().any(|(c, w)| c == "b" && *w > 0.999));
        let middle = at([0.5, 0.0]);
        assert!((middle[0].1 - 0.5).abs() < 1.0e-3);
    }

    #[test]
    fn a_transition_fades_and_the_phases_stay_in_step() {
        let clips = set();
        let mut animator = Animator::new(AnimGraph::parse(GRAPH).unwrap(), 1);
        assert_eq!(animator.state_name(), "idle");
        animator.advance(0.1, &clips);
        animator.set("speed", 2.0);
        animator.advance(0.0, &clips);
        assert_eq!(animator.state_name(), "moving");
        assert!(animator.fade.is_some());
        let rest = Pose::with_bones(1);
        let mut pose = Pose::with_bones(1);
        // Just begun, it is still the state it left. The idle clip stands at 0, and the walk
        // rises to 2 over its cycle.
        animator.evaluate(&clips, &rest, &mut pose);
        assert!(
            pose.translations[0].y.abs() < 1.0e-3,
            "{}",
            pose.translations[0].y
        );
        // Half way through the fade, it is half way between them.
        animator.advance(0.1, &clips);
        animator.evaluate(&clips, &rest, &mut pose);
        let walk_height = 2.0 * animator.current.phase;
        let middle = pose.translations[0].y;
        assert!(
            (middle - walk_height * 0.5).abs() < 0.05,
            "{middle} isn't half way to {walk_height}"
        );
        animator.advance(0.5, &clips);
        assert!(animator.fade.is_none(), "the fade should be finished");
        animator.evaluate(&clips, &rest, &mut pose);
        assert!((pose.translations[0].y - 2.0 * animator.current.phase).abs() < 0.01);
    }

    #[test]
    fn cycles_stay_in_step_across_a_fade_and_one_shots_start_at_their_start() {
        let graph = "
            clip a A
            clip b B
            clip die D once
            state a clip a
            state b clip b
            state dead clip die
            start a
        ";
        let names = |name: &str, frames: usize| {
            (
                name.to_owned(),
                compress(
                    name,
                    0,
                    &Frames {
                        rotations: vec![vec![Quat::IDENTITY]; frames],
                        translations: vec![vec![Vec3::ZERO]; frames],
                    },
                    name != "die",
                ),
            )
        };
        let clips = ClipSet::new(vec![names("a", 60), names("b", 60), names("die", 60)]);
        let mut animator = Animator::new(AnimGraph::parse(graph).unwrap(), 1);
        animator.advance(0.5, &clips);
        let phase = animator.current.phase;
        assert!(phase > 0.2);
        assert!(animator.go_to("b", 0.2));
        assert_eq!(animator.current.phase, phase, "a cycle fell out of step");
        assert!(animator.go_to("dead", 0.2));
        assert_eq!(
            animator.current.phase, 0.0,
            "a one-shot started part way in"
        );
    }

    #[test]
    fn a_fade_from_a_frozen_pose_goes_at_the_same_pace_at_any_frame_rate() {
        let clips = set();
        let run = |step: f32| {
            let mut animator = Animator::new(AnimGraph::parse(GRAPH).unwrap(), 1);
            animator.set("speed", 4.0);
            animator.advance(0.1, &clips);
            let rest = Pose::with_bones(1);
            let mut pose = Pose::with_bones(1);
            animator.evaluate(&clips, &rest, &mut pose);
            assert!(animator.go_to("idle", 0.6));
            animator.advance(0.1, &clips);
            animator.evaluate(&clips, &rest, &mut pose);
            assert!(animator.go_to("moving", 0.6), "changing during the fade");
            let mut moved = 0.0;
            while moved < 0.2 {
                animator.advance(step, &clips);
                animator.evaluate(&clips, &rest, &mut pose);
                moved += step;
            }
            pose.translations[0].y
        };
        let (slow, fast) = (run(1.0 / 30.0), run(1.0 / 120.0));
        assert!(
            (slow - fast).abs() < 0.1,
            "{slow} at 30 Hz, {fast} at 120 Hz"
        );
    }

    #[test]
    fn a_change_of_state_before_anything_is_drawn_has_nothing_to_fade_from() {
        let clips = set();
        let mut animator = Animator::new(AnimGraph::parse(GRAPH).unwrap(), 1);
        assert!(animator.go_to("moving", 0.2));
        assert!(animator.go_to("idle", 0.2));
        let mut rest = Pose::with_bones(1);
        rest.translations[0] = Vec3::Y * 5.0;
        let mut pose = Pose::with_bones(1);
        animator.evaluate(&clips, &rest, &mut pose);
        // The idle clip's own pose, rather than a blend with a pose that was never drawn.
        assert!(
            pose.translations[0].y.abs() < 1.0e-3,
            "{}",
            pose.translations[0].y
        );
    }

    #[test]
    fn a_long_step_counts_every_cycle_and_a_notify_at_the_end_fires_each_cycle() {
        let clips = set();
        let graph = |time: &str| {
            format!(
                "clip idle I\nclip walk W\nclip run R\nnotify walk {time} step\nstate w clip walk\nstart w"
            )
        };
        let mut animator = Animator::new(AnimGraph::parse(&graph("0.5")).unwrap(), 1);
        animator.advance(0.2, &clips);
        assert_eq!(animator.advance(2.5, &clips).len(), 3, "a 2.5 s step");

        let mut animator = Animator::new(AnimGraph::parse(&graph("1.0")).unwrap(), 1);
        let count: usize = (0..40).map(|_| animator.advance(0.25, &clips).len()).sum();
        assert_eq!(count, 10, "a notify at the end of a 1 s loop, over 10 s");
    }

    #[test]
    fn looking_ahead_moves_the_pose_and_not_the_clock() {
        let clips = set();
        let graph = "clip idle I\nclip walk W\nclip run R\nstate w clip walk\nstart w";
        let mut animator = Animator::new(AnimGraph::parse(graph).unwrap(), 1);
        animator.advance(0.25, &clips);
        let rest = Pose::with_bones(1);
        let mut pose = Pose::with_bones(1);
        let phase = animator.current.phase;
        animator.evaluate_ahead(&clips, &rest, &mut pose, 0.25);
        assert!(
            (pose.translations[0].y - 1.0).abs() < 0.02,
            "{}",
            pose.translations[0].y
        );
        assert_eq!(animator.current.phase, phase);
    }

    #[test]
    fn a_blend_of_looping_and_one_shot_clips_is_refused() {
        let error = AnimGraph::parse(
            "clip a A\nclip b B once\nparam x 0\nstate s blend1d x 0 a 1 b\nstart s",
        )
        .unwrap_err();
        assert!(error.contains("loop"), "{error}");
    }

    #[test]
    fn a_change_of_state_during_a_fade_starts_from_where_it_had_got_to() {
        let clips = set();
        let graph = GRAPH.replace("fade 0.2", "fade 0");
        let mut animator = Animator::new(AnimGraph::parse(&graph).unwrap(), 1);
        animator.set("speed", 4.0);
        animator.advance(0.1, &clips);
        let rest = Pose::with_bones(1);
        let mut pose = Pose::with_bones(1);
        assert!(animator.go_to("idle", 0.5));
        animator.advance(0.25, &clips);
        animator.evaluate(&clips, &rest, &mut pose);
        let before = pose.translations[0].y;
        assert!(animator.go_to("moving", 0.5), "changing back in the middle");
        animator.evaluate(&clips, &rest, &mut pose);
        assert!(
            (pose.translations[0].y - before).abs() < 1.0e-3,
            "it jumped from {before} to {}",
            pose.translations[0].y
        );
    }

    #[test]
    fn an_end_of_clip_notify_fires_once_and_one_at_the_wrap_each_cycle() {
        let graph = "
            clip a A once
            notify a 1.0 end
            state a clip a
            start a
        ";
        let once = compress(
            "a",
            0,
            &Frames {
                rotations: vec![vec![Quat::IDENTITY]; 31],
                translations: vec![vec![Vec3::ZERO]; 31],
            },
            false,
        );
        let clips = ClipSet::new(vec![("a".to_owned(), once)]);
        let mut animator = Animator::new(AnimGraph::parse(graph).unwrap(), 1);
        let count: usize = (0..120)
            .map(|_| animator.advance(1.0 / 30.0, &clips).len())
            .sum();
        assert_eq!(count, 1);

        let clips = set();
        let graph = "
            clip idle I
            clip walk W
            clip run R
            notify walk 0.0 step
            state walking clip walk
            start walking
        ";
        let mut animator = Animator::new(AnimGraph::parse(graph).unwrap(), 1);
        let count: usize = (0..110)
            .map(|_| animator.advance(1.0 / 30.0, &clips).len())
            .sum();
        assert_eq!(count, 4, "at 0 s, and each wrap at 1, 2 and 3 s");
    }

    #[test]
    fn notifies_fire_once_as_their_time_is_crossed() {
        let clips = set();
        let mut animator = Animator::new(AnimGraph::parse(GRAPH).unwrap(), 1);
        animator.set("speed", 2.0);
        animator.advance(0.0, &clips);
        let mut count = 0;
        for _ in 0..120 {
            count += animator.advance(1.0 / 30.0, &clips).len();
        }
        assert_eq!(count, 4, "four cycles of a 1 s clip, one notify each");
    }

    #[test]
    fn a_blend_keeps_the_pace_of_its_shares() {
        let clips = set();
        let mut animator = Animator::new(AnimGraph::parse(GRAPH).unwrap(), 1);
        animator.set("speed", 1.0);
        animator.advance(0.0, &clips);
        // Half idle (2 s), half walk (1 s).
        assert!((animator.cycle_seconds(&clips, 1) - 1.5).abs() < 1.0e-3);
    }
}
