//! Events, and timers that send them.
//!
//! Systems talk through events rather than calling each other. An event is any value; each type
//! has its own queue, in the order events were sent. Every event is stamped with the step and
//! the tick group that sent it, and each reader sees it exactly once:
//!
//! * **A fixed-step group** sees what was sent after it ran in the step before (by itself or a
//!   later group), and what was sent before it in this step. So whichever group sends an event,
//!   every fixed-step group sees it once, within one step. A handler never runs halfway through
//!   another system, and replays give the same result.
//! * **The per-frame groups** see everything sent since they last ran, however many steps the
//!   frame held, zero or several.
//!
//! Events sent before a step starts (while setting a world up, or by timers firing as the step
//! starts) count as sent before the step's first group, so all of its groups see them.
//!
//! Timers send an event after a number of steps, once or repeatedly. They count steps, not
//! seconds, so they fire on the same step in every run.

use crate::clock::{STEP_TICKS, TICKS_PER_SECOND};
use std::any::{Any, TypeId};
use std::collections::{BTreeMap, HashMap};

/// When an event was sent: the step, and the place in it. Place 0 is before the step's first
/// group; the fixed-step groups are places 1, 2 and 3.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Stamp {
    pub step: u64,
    pub place: u8,
}

/// Events are kept this many steps at most: long enough for a frame of the most steps to see
/// them, and for the step after to.
const KEEP_STEPS: u64 = crate::clock::MAX_STEPS_PER_FRAME as u64 + 1;

/// Every event queue.
#[derive(Default)]
pub(crate) struct Events {
    queues: HashMap<TypeId, Box<dyn Queue>>,
}

trait Queue: Any {
    /// Drops events every reader has seen: those sent before `before` that the per-frame groups
    /// have seen too (sent by `seen`), and any older than [`KEEP_STEPS`].
    fn prune(&mut self, before: Stamp, seen: Stamp);
    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

struct TypedQueue<E> {
    events: Vec<(Stamp, E)>,
}

impl<E: 'static> Queue for TypedQueue<E> {
    fn prune(&mut self, before: Stamp, seen: Stamp) {
        let too_old = before.step.saturating_sub(KEEP_STEPS);
        self.events
            .retain(|(stamp, _)| stamp.step > too_old && (*stamp >= before || *stamp > seen));
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

/// Which events a reader sees.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Reader {
    /// A fixed-step group, at its place in a step.
    Step(Stamp),
    /// The per-frame groups: what was sent after `since`.
    Frame { since: Stamp },
}

impl Reader {
    fn sees(self, stamp: Stamp) -> bool {
        match self {
            // In the step before, from this place on; in this step, before this place.
            Self::Step(at) => {
                (stamp.step + 1 == at.step && stamp.place >= at.place)
                    || (stamp.step == at.step && stamp.place < at.place)
            }
            Self::Frame { since } => stamp > since,
        }
    }
}

impl Events {
    fn queue<E: 'static>(&mut self) -> &mut TypedQueue<E> {
        self.queues
            .entry(TypeId::of::<E>())
            .or_insert_with(|| Box::new(TypedQueue::<E> { events: Vec::new() }))
            .as_any_mut()
            .downcast_mut::<TypedQueue<E>>()
            .expect("each type has a queue of its own")
    }

    pub fn send<E: 'static>(&mut self, stamp: Stamp, event: E) {
        self.queue::<E>().events.push((stamp, event));
    }

    /// The events of one type a reader sees, in the order they were sent.
    pub fn read<E: 'static>(&self, reader: Reader) -> impl Iterator<Item = &E> {
        self.queues
            .get(&TypeId::of::<E>())
            .and_then(|queue| queue.as_any().downcast_ref::<TypedQueue<E>>())
            .map_or(&[][..], |queue| &queue.events[..])
            .iter()
            .filter(move |(stamp, _)| reader.sees(*stamp))
            .map(|(_, event)| event)
    }

    /// Drops events no reader can see any more, as step `step` starts, given what the
    /// per-frame groups have seen.
    pub fn prune(&mut self, step: u64, frame_seen: Stamp) {
        let before = Stamp {
            step: step.saturating_sub(1),
            place: 0,
        };
        for queue in self.queues.values_mut() {
            queue.prune(before, frame_seen);
        }
    }
}

/// Names a timer, to cancel it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TimerId(u64);

/// Timers waiting to send their events.
#[derive(Default)]
pub(crate) struct Timers {
    next_id: u64,
    /// By the step each fires on, then by when it was set, so ties fire in order.
    waiting: BTreeMap<(u64, TimerId), Timer>,
    /// Where each waiting timer is in `waiting`, to cancel it.
    due: HashMap<TimerId, u64>,
}

type SendFn = Box<dyn FnMut(&mut Events, Stamp)>;

struct Timer {
    /// Steps between firings, for one that repeats.
    every: Option<u64>,
    send: SendFn,
}

/// The steps that `seconds` covers, rounded up, and at least one. Seconds are first rounded to
/// the clock's ticks, so a whole number of steps is exactly that many, whatever rounding error
/// the seconds carry.
pub fn steps_for(seconds: f64) -> u64 {
    let ticks = (seconds * TICKS_PER_SECOND as f64).round();
    if ticks.is_finite() && ticks >= 1.0 {
        (ticks as u64).div_ceil(STEP_TICKS).max(1)
    } else {
        1
    }
}

impl Timers {
    /// Sends `event` `steps` steps after step `now`, at least one, so on a later step.
    pub fn after<E: Clone + 'static>(&mut self, now: u64, steps: u64, event: E) -> TimerId {
        self.add(now + steps.max(1), None, event)
    }

    /// Sends `event` every `steps` steps from step `now` on, until cancelled.
    pub fn every<E: Clone + 'static>(&mut self, now: u64, steps: u64, event: E) -> TimerId {
        let steps = steps.max(1);
        self.add(now + steps, Some(steps), event)
    }

    fn add<E: Clone + 'static>(&mut self, due: u64, every: Option<u64>, event: E) -> TimerId {
        let id = TimerId(self.next_id);
        self.next_id += 1;
        let timer = Timer {
            every,
            send: Box::new(move |events, stamp| events.send(stamp, event.clone())),
        };
        self.waiting.insert((due, id), timer);
        self.due.insert(id, due);
        id
    }

    /// Stops a timer. Returns whether it was still waiting.
    pub fn cancel(&mut self, id: TimerId) -> bool {
        match self.due.remove(&id) {
            Some(due) => self.waiting.remove(&(due, id)).is_some(),
            None => false,
        }
    }

    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.waiting.is_empty()
    }

    /// Sends the events of every timer due by step `now`, stamped as sent before the step's
    /// first group, in the order they fall due, and sets repeating ones again.
    pub fn fire(&mut self, now: u64, events: &mut Events) {
        let stamp = Stamp {
            step: now,
            place: 0,
        };
        while let Some(entry) = self.waiting.first_entry() {
            let (due, id) = *entry.key();
            if due > now {
                break;
            }
            let mut timer = entry.remove();
            (timer.send)(events, stamp);
            match timer.every {
                Some(every) => {
                    let next = due + every;
                    self.due.insert(id, next);
                    self.waiting.insert((next, id), timer);
                }
                None => {
                    self.due.remove(&id);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Debug, PartialEq)]
    struct Ping(u32);
    #[derive(Clone, Debug, PartialEq)]
    struct Pong(&'static str);

    fn at(step: u64, place: u8) -> Stamp {
        Stamp { step, place }
    }

    fn pings(events: &Events, reader: Reader) -> Vec<u32> {
        events.read::<Ping>(reader).map(|p| p.0).collect()
    }

    #[test]
    fn each_fixed_group_sees_each_event_once_within_a_step() {
        let mut events = Events::default();
        // Sent in step 5: before it starts, then by its first, second and third groups.
        events.send(at(5, 0), Ping(0));
        events.send(at(5, 1), Ping(1));
        events.send(at(5, 2), Ping(2));
        events.send(at(5, 3), Ping(3));
        events.send(at(5, 2), Pong("other type"));
        for group in 1..=3_u8 {
            let mut seen = pings(&events, Reader::Step(at(5, group)));
            seen.extend(pings(&events, Reader::Step(at(6, group))));
            seen.sort_unstable();
            assert_eq!(seen, [0, 1, 2, 3], "group {group}");
        }
        // In order, and only what each place should: the second group of step 5 sees what came
        // before it; the same group in step 6 sees the rest.
        assert_eq!(pings(&events, Reader::Step(at(5, 2))), [0, 1]);
        assert_eq!(pings(&events, Reader::Step(at(6, 2))), [2, 3]);
        assert!(pings(&events, Reader::Step(at(7, 1))).is_empty());
        assert_eq!(events.read::<Pong>(Reader::Step(at(5, 3))).count(), 1);
    }

    #[test]
    fn the_frame_sees_everything_since_it_last_looked_once() {
        let mut events = Events::default();
        events.send(at(1, 1), Ping(1));
        events.send(at(2, 3), Ping(2));
        events.send(at(3, 2), Ping(3));
        // A frame after three steps sees them all; the next, with none, sees nothing again.
        assert_eq!(pings(&events, Reader::Frame { since: at(0, 9) }), [1, 2, 3]);
        assert!(pings(&events, Reader::Frame { since: at(3, 9) }).is_empty());
    }

    #[test]
    fn pruning_keeps_what_a_reader_still_needs() {
        let mut events = Events::default();
        events.send(at(1, 2), Ping(1));
        events.send(at(2, 2), Ping(2));
        events.send(at(3, 3), Ping(3));
        // Step 4 starts, and the frame has seen everything up to step 2: step 3's event is
        // still wanted by step 4, the others by no one.
        events.prune(4, at(2, 9));
        assert_eq!(pings(&events, Reader::Frame { since: at(0, 0) }), [3]);
        // Unseen by the frame: kept for it, until it is too old.
        events.send(at(9, 1), Ping(9));
        events.prune(11, at(0, 0));
        assert_eq!(pings(&events, Reader::Frame { since: at(0, 0) }), [9]);
        events.prune(20, at(0, 0));
        assert!(pings(&events, Reader::Frame { since: at(0, 0) }).is_empty());
    }

    #[test]
    fn a_type_never_sent_reads_as_nothing() {
        assert_eq!(
            Events::default()
                .read::<Ping>(Reader::Step(at(1, 1)))
                .count(),
            0
        );
    }

    /// Runs steps from 1 to `last`, firing timers, and lists what each step's first group saw.
    fn run(timers: &mut Timers, last: u64) -> Vec<(u64, Vec<u32>)> {
        let mut events = Events::default();
        let mut log = Vec::new();
        for step in 1..=last {
            timers.fire(step, &mut events);
            let seen = pings(&events, Reader::Step(at(step, 1)));
            if !seen.is_empty() {
                log.push((step, seen));
            }
        }
        log
    }

    #[test]
    fn timers_fire_on_their_step_in_the_order_they_were_set() {
        let mut timers = Timers::default();
        timers.after(0, 3, Ping(1));
        timers.after(0, 2, Ping(2));
        timers.after(0, 3, Ping(3));
        // Zero steps still means a later step, not this one.
        timers.after(0, 0, Ping(4));
        assert_eq!(
            run(&mut timers, 5),
            vec![(1, vec![4]), (2, vec![2]), (3, vec![1, 3])]
        );
        assert!(timers.is_empty());
    }

    #[test]
    fn repeating_timers_keep_firing_until_cancelled() {
        let mut timers = Timers::default();
        let every = timers.every(0, 2, Ping(7));
        let once = timers.after(0, 5, Ping(8));
        assert_eq!(
            run(&mut timers, 6),
            vec![(2, vec![7]), (4, vec![7]), (5, vec![8]), (6, vec![7])]
        );
        assert!(!timers.cancel(once), "a timer that fired is gone");
        assert!(timers.cancel(every));
        assert!(!timers.cancel(every));
        assert!(timers.is_empty());
    }

    #[test]
    fn a_late_firing_catches_up_on_everything_due() {
        let mut timers = Timers::default();
        timers.after(0, 1, Ping(1));
        timers.after(0, 2, Ping(2));
        timers.every(0, 2, Ping(3));
        let mut events = Events::default();
        timers.fire(5, &mut events);
        // The repeating one fell due at 2 and 4, so twice, each in its turn.
        assert_eq!(pings(&events, Reader::Step(at(5, 1))), [1, 2, 3, 3]);
    }

    #[test]
    fn seconds_become_whole_steps_rounded_up() {
        assert_eq!(steps_for(1.0), 60);
        assert_eq!(steps_for(0.01), 1);
        assert_eq!(steps_for(0.0), 1);
        assert_eq!(steps_for(-3.0), 1);
        assert_eq!(steps_for(f64::NAN), 1);
        assert_eq!(steps_for(2.5), 150);
        // Seconds that are a whole number of steps, give or take rounding, are exactly that.
        assert_eq!(steps_for(crate::clock::STEP_SECONDS * 125.0), 125);
        assert_eq!(steps_for(4.15), 249);
        assert_eq!(steps_for(8.05), 483);
    }
}
