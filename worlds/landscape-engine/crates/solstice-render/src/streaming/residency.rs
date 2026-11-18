//! Decides which pages should be on the GPU. No GPU code, so all of it can be tested alone.
//!
//! The rule that keeps a partly loaded scene watertight: a page only installs after every page
//! it depends on, and only leaves after every page depending on it. A page's dependencies hold
//! the parents of its clusters, so a parent is always there to draw until its children are.

use std::cmp::Reverse;
use std::collections::{BTreeSet, BinaryHeap};
use std::ops::Index;

/// A page graph stored in two contiguous arrays instead of a heap allocation per page.
/// Its topology is fixed when the baked scene opens; frames only read slices of it.
struct PageLinks {
    offsets: Vec<usize>,
    entries: Vec<u32>,
}

impl PageLinks {
    fn new(lists: Vec<Vec<u32>>) -> Self {
        let mut offsets = Vec::with_capacity(lists.len() + 1);
        let mut entries = Vec::with_capacity(lists.iter().map(Vec::len).sum());
        offsets.push(0);
        for list in lists {
            entries.extend(list);
            offsets.push(entries.len());
        }
        Self { offsets, entries }
    }

    fn reversed(&self) -> Self {
        let mut offsets = vec![0; self.offsets.len()];
        for &page in &self.entries {
            offsets[page as usize + 1] += 1;
        }
        for i in 1..offsets.len() {
            offsets[i] += offsets[i - 1];
        }
        let mut next = offsets.clone();
        let mut entries = vec![0; self.entries.len()];
        for page in 0..self.offsets.len() - 1 {
            for &dependency in &self[page] {
                let at = &mut next[dependency as usize];
                entries[*at] = page as u32;
                *at += 1;
            }
        }
        Self { offsets, entries }
    }
}

impl Index<usize> for PageLinks {
    type Output = [u32];

    fn index(&self, page: usize) -> &Self::Output {
        &self.entries[self.offsets[page]..self.offsets[page + 1]]
    }
}

/// In the request array, a page that was seen but occluded: worth keeping, not worth loading.
pub const REQUEST_KEEP: u32 = 1;

/// In the request array, the most a page is asked for with by a view that doesn't see it yet:
/// one fetching ahead of need. Twin of `REQUEST_LOWEST_LOAD` in `vg/cull.wgsl`.
pub const REQUEST_AHEAD: u32 = 2;

/// How many times as wanted as an installed page a page without a slot must be to take its
/// slot. Well over one, so that two pages wanted about as much don't keep changing places.
pub const OUTBID: u32 = 2;

/// Frames a page stays safe from eviction after it was last wanted. Long enough that looking
/// around doesn't throw away what's about to be seen again.
pub const PROTECTED_FRAMES: u32 = 30;

/// The same for a scene with pools by level of detail, whose pools are full nearly all the time:
/// what has left the view makes way sooner, so a new view fills in faster.
pub const PROTECTED_FRAMES_LARGE: u32 = 10;

/// Frames an unpacked page may wait for a slot, unwanted, before it is thrown away.
const ARRIVED_PATIENCE_FRAMES: u32 = 120;

/// Frames a page stays safe after it came for a view kept from frame to frame, and frames of
/// such a view's redrawing for which a page stays safe after the view asked for it. Such a view
/// asks only when it is redrawn, and a page's coming makes it redraw: were the page let go in
/// between, the redraw would ask for it again, and never stop. So only redraws that leave a
/// page unasked count against it. While nothing redraws the view it stands on the page still,
/// however long that is.
pub const HOLD_FRAMES: u32 = 240;

/// Frames in each of the two spans over which a page counts as wanted as much as it was asked
/// for at most. Views ask by turns: one kept from frame to frame only when it is redrawn, and
/// between its redraws a page it needs is asked for with far less by the others, or as one only
/// hidden. Taken by the last asking alone, such a page would lose its slot at each turn and
/// take another's at the next.
pub const ASKED_FRAMES: u32 = 120;

/// Frames of such views' redrawing for which a page counts as wanted as much as a view kept
/// from frame to frame asked for it with at most. Such a view asks again only when something
/// makes it redraw, and a page's coming does: so a page that lost its slot for being left
/// unasked would be asked for again as soon as the page that took it came, and take it back.
/// There are several such views of a kind, each redrawn in its own time and wanting the page as
/// much as it sees of it. Long enough that this hardly happens, and not for ever, as a view may
/// move on and not ask again. Also the frames after which a kind of such view that has asked
/// for nothing counts as gone.
pub const KEPT_ASKED_FRAMES: u32 = 3600;

/// Frames for which a page's coming is news to the kept views that asked for it.
const AWAIT_FRAMES: u32 = 3600;

/// What is known of one kind of view kept from frame to frame.
#[derive(Clone, Copy, Default)]
struct Kept {
    /// For how many frames its views were being redrawn: the frames up to each plan for which
    /// one of them asked for a page.
    redrawn: u32,
    /// The frame of the last such plan.
    asked_at: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Absent,
    Fetching,
    /// Data is on the CPU, waiting for dependencies or a slot.
    Arrived,
    Installed(u32),
}

/// One change to what is on the GPU.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// A page left its slot.
    Evict(u32),
    /// A page went into a slot.
    Install { page: u32, slot: u32 },
}

/// A page moved onto the GPU.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Install {
    pub page: u32,
    pub slot: u32,
    /// The page that had to leave that slot.
    pub evicted: Option<u32>,
}

/// Tracks every page's state, and owns the allocation of pool slots.
pub struct Residency {
    dependencies: PageLinks,
    dependants: PageLinks,
    pinned: u32,
    states: Vec<State>,
    /// The installed pages, so that a look over them costs what's installed, not the whole scene.
    installed: BTreeSet<u32>,
    /// Scratch for [`Self::reclaimable`], all false between uses.
    needed: Vec<bool>,
    /// See [`PROTECTED_FRAMES`].
    protected_frames: u32,
    /// How many pages depending on this one are installed.
    installed_dependants: Vec<u32>,
    last_wanted: Vec<u32>,
    /// Which pool each page belongs in, by its level of detail.
    home_bands: Vec<u8>,
    /// Which pool holds each page, or will: its home unless that had no room when it was
    /// fetched and another did. Each pool evicts only the pages it holds.
    bands: Vec<u8>,
    free_slots: Vec<Vec<u32>>,
    fetching_by_band: Vec<u32>,
    arrived: BTreeSet<u32>,
    fetching: u32,
    /// Pages that wanted loading but found no room in the pool at the last plan.
    starved: u32,
    /// What the last plan wanted and could not start yet: highest priority, then lowest id.
    /// Heap construction is linear; a frame needn't sort a million pages to fetch thirty-two.
    queue: BinaryHeap<(u32, Reverse<u32>)>,
    /// Slots the last plan found reclaimable in each band, which is slow to work out. Each
    /// eviction spends one.
    reclaimable_at_plan: Vec<u32>,
    /// Each band's pages that may be evicted, likeliest last, found once a frame as that takes
    /// a whole scan.
    victims: Vec<Vec<u32>>,
    victims_frame: Option<u32>,
    /// Pages let go by another band, whose records the caller must update.
    released: Vec<u32>,
    /// The frame, plus one, at which a view kept from frame to frame last asked for each page
    /// while it wasn't installed. 0 for never.
    awaited: Vec<u32>,
    /// Which kinds of kept view asked then, a bit for each.
    awaited_by: Vec<u8>,
    /// The kinds of kept view for which such a page has installed since
    /// [`Self::take_awaited_arrival`].
    awaited_arrival: u8,
    /// The most each page was asked for with in the span of [`ASKED_FRAMES`] it was last asked
    /// in, and in the span before that one.
    asked: Vec<[u32; 2]>,
    /// The span each page was last asked in.
    asked_span: Vec<u32>,
    /// The most each page was asked for with as a view kept from frame to frame asked for it,
    /// in the [`KEPT_ASKED_FRAMES`] before it was last asked for with that much, and what
    /// `kept_redrawn` was then.
    kept_asked: Vec<(u32, u32)>,
    /// For how many frames views kept from frame to frame were being redrawn, of any kind: the
    /// frames up to each plan for which one asked for a page.
    kept_redrawn: u32,
    /// Each kind of view kept from frame to frame that has asked for a page.
    kept: Vec<Kept>,
    /// For each such kind, and each page: [`Kept::redrawn`], plus one, when a view of the kind
    /// last asked for the page. 0 for never.
    held_at: Vec<u32>,
    /// The frame of the last plan.
    planned_at: u32,
    /// The plan, as numbered by `plans`, that found each page outbid. Until the next plan such
    /// a page may be evicted though it is wanted.
    outbid: Vec<u32>,
    /// How many plans have been made, from 1.
    plans: u32,
}

impl Residency {
    /// `dependencies` must only point at lower page ids, which is how scenes are baked. The
    /// first `pinned` pages never leave. There must be slots for them.
    pub fn new(dependencies: Vec<Vec<u32>>, pinned: u32, slots: u32) -> Self {
        let bands = vec![0; dependencies.len()];
        Self::with_bands(dependencies, pinned, bands, &[slots])
    }

    /// As [`Self::new`], with the pool divided into bands, each page in the band `bands` gives.
    /// Slots are numbered through the bands in order.
    pub fn with_bands(
        dependencies: Vec<Vec<u32>>,
        pinned: u32,
        bands: Vec<u8>,
        band_slots: &[u32],
    ) -> Self {
        let count = dependencies.len();
        let mut pinned_by_band = vec![0u32; band_slots.len()];
        for &band in &bands[..pinned as usize] {
            pinned_by_band[band as usize] += 1;
        }
        assert!(
            pinned_by_band.iter().zip(band_slots).all(|(p, s)| p <= s),
            "the pool must hold the pinned pages"
        );
        for (page, list) in dependencies.iter().enumerate() {
            for &dependency in list {
                assert!((dependency as usize) < page, "dependencies point coarser");
            }
        }
        let dependencies = PageLinks::new(dependencies);
        let dependants = dependencies.reversed();
        let mut first = 0;
        let free_slots = band_slots
            .iter()
            .map(|&slots| {
                let band: Vec<u32> = (first..first + slots).rev().collect();
                first += slots;
                band
            })
            .collect();
        Self {
            dependencies,
            dependants,
            pinned,
            states: vec![State::Absent; count],
            installed: BTreeSet::new(),
            needed: vec![false; count],
            protected_frames: if band_slots.len() > 1 {
                PROTECTED_FRAMES_LARGE
            } else {
                PROTECTED_FRAMES
            },
            installed_dependants: vec![0; count],
            last_wanted: vec![0; count],
            home_bands: bands.clone(),
            bands,
            free_slots,
            fetching_by_band: vec![0; band_slots.len()],
            arrived: BTreeSet::new(),
            fetching: 0,
            starved: 0,
            queue: BinaryHeap::new(),
            reclaimable_at_plan: vec![0; band_slots.len()],
            victims: vec![Vec::new(); band_slots.len()],
            victims_frame: None,
            released: Vec::new(),
            awaited: vec![0; count],
            awaited_by: vec![0; count],
            awaited_arrival: 0,
            asked: vec![[0; 2]; count],
            asked_span: vec![0; count],
            kept_asked: vec![(0, 0); count],
            kept_redrawn: 0,
            kept: Vec::new(),
            held_at: Vec::new(),
            planned_at: 0,
            outbid: vec![0; count],
            plans: 1,
        }
    }

    pub fn is_installed(&self, page: u32) -> bool {
        matches!(self.states[page as usize], State::Installed(_))
    }

    /// How many pages are installed in each band.
    pub fn installed_by_band(&self) -> Vec<u32> {
        let mut count = vec![0; self.free_slots.len()];
        for &page in &self.installed {
            count[usize::from(self.bands[page as usize])] += 1;
        }
        count
    }

    pub fn installed_count(&self) -> u32 {
        self.installed.len() as u32
    }

    /// Pages being fetched, or fetched and waiting to install.
    pub fn in_transit(&self) -> u32 {
        self.fetching + self.arrived.len() as u32
    }

    /// Pages that wanted to load at the last plan, but the pool had no room for.
    pub fn starved(&self) -> u32 {
        self.starved
    }

    /// Takes in one frame's request array, and returns the pages to start fetching now, most
    /// wanted first. At most `max_fetching` pages are ever being fetched at once, and it only
    /// fetches what the pool could make room for.
    ///
    /// `requests` holds, per page, 0 if unseen, [`REQUEST_KEEP`] if occluded, and otherwise a
    /// priority, higher for more visible error.
    pub fn plan(&mut self, frame: u32, requests: &[u32], max_fetching: u32) -> Vec<u32> {
        self.plan_holding(frame, requests, &[], max_fetching)
    }

    /// As [`Self::plan`], with the pages that views kept from frame to frame asked for: for
    /// each kind of such view, up to eight, a bit per page in `held`. Such a view asks only when
    /// it is redrawn, so its pages are kept until views of its kind have been redrawn for
    /// [`HOLD_FRAMES`] without asking again, and their coming is reported, by kind, by
    /// [`Self::take_awaited_arrival`].
    pub fn plan_holding(
        &mut self,
        frame: u32,
        requests: &[u32],
        held: &[&[u32]],
        max_fetching: u32,
    ) -> Vec<u32> {
        let count = self.states.len();
        let mut priority = vec![0u32; count];
        priority[..count.min(requests.len())]
            .copy_from_slice(&requests[..count.min(requests.len())]);
        let mut hold = vec![0u8; if held.is_empty() { 0 } else { count }];
        for (kind, words) in held.iter().enumerate() {
            for (index, &word) in words.iter().enumerate() {
                let mut bits = word;
                while bits != 0 {
                    if let Some(page) = hold.get_mut(index * 32 + bits.trailing_zeros() as usize) {
                        *page |= 1 << kind;
                    }
                    bits &= bits - 1;
                }
            }
        }
        if self.kept.len() < held.len() {
            self.kept.resize(held.len(), Kept::default());
            self.held_at.resize(held.len() * count, 0);
        }
        let elapsed = frame.saturating_sub(self.planned_at);
        self.planned_at = frame;
        let mut redrawn = false;
        for (kind, kept) in self.kept.iter_mut().enumerate() {
            let asked = held
                .get(kind)
                .is_some_and(|words| words.iter().any(|&word| word != 0));
            if asked {
                kept.asked_at = frame;
            }
            // A kind left alone for long may be drawn no more, and must not keep its pages for
            // ever: its frames count again.
            if asked || frame.saturating_sub(kept.asked_at) > KEPT_ASKED_FRAMES {
                kept.redrawn = kept.redrawn.saturating_add(elapsed);
                redrawn = true;
            }
        }
        if redrawn {
            self.kept_redrawn = self.kept_redrawn.saturating_add(elapsed);
        }

        // Parents have lower ids, so one pass downwards carries wants all the way up. A parent
        // is at least as wanted as any child, as it must arrive first.
        for page in (0..count).rev() {
            let wanted = priority[page];
            let held = hold.get(page).copied().unwrap_or(0);
            if wanted == 0 && held == 0 {
                continue;
            }
            // Not simply this frame: a page that came for a kept view is safe for longer.
            self.last_wanted[page] = self.last_wanted[page].max(frame);
            if wanted != 0 {
                self.ask(page, frame, wanted);
            }
            if held != 0 {
                let (kept, at) = self.kept_asked[page];
                if wanted >= kept || self.kept_redrawn - at > KEPT_ASKED_FRAMES {
                    self.kept_asked[page] = (wanted, self.kept_redrawn);
                }
                let mut kinds = held;
                while kinds != 0 {
                    let kind = kinds.trailing_zeros() as usize;
                    self.held_at[kind * count + page] = self.kept[kind].redrawn.saturating_add(1);
                    kinds &= kinds - 1;
                }
                if !self.is_installed(page as u32) {
                    let fresh = frame.saturating_sub(self.awaited[page].saturating_sub(1))
                        <= AWAIT_FRAMES
                        && self.awaited[page] != 0;
                    self.awaited_by[page] = if fresh {
                        self.awaited_by[page] | held
                    } else {
                        held
                    };
                    self.awaited[page] = frame.saturating_add(1);
                }
            }
            for index in 0..self.dependencies[page].len() {
                let dependency = self.dependencies[page][index] as usize;
                priority[dependency] = priority[dependency].max(wanted);
                if held != 0 {
                    hold[dependency] |= held;
                }
            }
        }

        // Unpacked pages nobody wants any more shouldn't hold a place in the queue forever.
        let stale: Vec<u32> = self
            .arrived
            .iter()
            .copied()
            .filter(|&page| {
                frame.saturating_sub(self.last_wanted[page as usize]) > ARRIVED_PATIENCE_FRAMES
                    && !self.is_held(page as usize)
            })
            .collect();
        for page in stale {
            self.arrived.remove(&page);
            self.states[page as usize] = State::Absent;
            self.bands[page as usize] = self.home_bands[page as usize];
        }

        let mut wanted = Vec::new();
        // How wanted the pages already on their way are: each has a slot to come.
        let mut coming = Vec::new();
        for (page, &worth) in priority.iter().enumerate() {
            match self.states[page] {
                State::Absent if worth > REQUEST_KEEP => wanted.push(page as u32),
                State::Fetching | State::Arrived => coming.push(worth),
                _ => {}
            }
        }
        self.plans = self.plans.wrapping_add(1).max(1);
        self.reclaimable_at_plan = self.reclaimable(frame);
        coming.extend(wanted.iter().map(|&page| priority[page as usize]));
        self.find_outbid(frame, coming);
        // Listed before this plan, the pages that may leave lack those it found outbid.
        self.victims_frame = None;
        let room: u32 = self.room().1.iter().sum();
        self.starved = (wanted.len() as u32).saturating_sub(room);
        self.queue = BinaryHeap::from(
            wanted
                .into_iter()
                .map(|page| (priority[page as usize], Reverse(page)))
                .collect::<Vec<_>>(),
        );
        self.refill(max_fetching)
    }

    /// Starts fetching more of what the last plan wanted, as earlier fetches finish. Requests
    /// come back from the GPU only every few frames, and between them the pipeline would
    /// otherwise run dry.
    pub fn refill(&mut self, max_fetching: u32) -> Vec<u32> {
        let (mut free, mut room) = self.room();
        // The host holds the data of every page that has come and waits to go in, and a pool
        // slow to make room would let them pile up without end: once as many wait on the pool
        // as may be fetched at once, no more are fetched until some have gone in.
        let can_fetch = if self.waiting_on_pool() >= max_fetching {
            0
        } else {
            room.iter()
                .sum::<u32>()
                .min(max_fetching.saturating_sub(self.fetching))
        };
        let mut started = Vec::new();
        while (started.len() as u32) < can_fetch {
            let Some((_, Reverse(page))) = self.queue.pop() else {
                break;
            };
            if self.states[page as usize] != State::Absent {
                continue;
            }
            // Its own pool first, and otherwise whichever has the most to spare: a pool lends
            // only what it has free, isn't using, or wants far less than this page is wanted.
            // An empty slot anywhere comes before one that a page must leave: what isn't wanted
            // now is often wanted again soon, by a view that is only drawn now and then.
            let home = usize::from(self.home_bands[page as usize]);
            let most = |of: &[u32]| (0..of.len()).max_by_key(|&band| of[band]).unwrap_or(home);
            let band = if free[home] > 0 {
                home
            } else if free[most(&free)] > 0 {
                most(&free)
            } else if room[home] > 0 {
                home
            } else {
                most(&room)
            };
            if room[band] > 0 {
                room[band] -= 1;
                free[band] = free[band].saturating_sub(1);
                self.bands[page as usize] = band as u8;
                self.states[page as usize] = State::Fetching;
                self.fetching_by_band[band] += 1;
                started.push(page);
            }
        }
        self.fetching += started.len() as u32;
        started
    }

    /// How many pages have come and wait only for the pool to take them: everything each
    /// depends on is in, or has come or is coming in its turn. One waiting for a page that is
    /// absent, because its fetch failed or it has left since, waits for a fetch, not for the
    /// pool: were it counted, enough of them would stop the very fetch they wait for.
    fn waiting_on_pool(&self) -> u32 {
        // What a page depends on has a lower id, so is decided before it, and these stay in
        // order.
        let mut orphaned: Vec<u32> = Vec::new();
        let mut waiting = 0;
        for &page in &self.arrived {
            let orphan = self.dependencies[page as usize].iter().any(|&d| {
                self.states[d as usize] == State::Absent || orphaned.binary_search(&d).is_ok()
            });
            if orphan {
                orphaned.push(page);
            } else {
                waiting += 1;
            }
        }
        waiting
    }

    /// When more pages need slots than the pool can give, finds the installed pages that are
    /// wanted far less than those left without: each page left out, most wanted first, takes
    /// the place of the least wanted page still in, while it is wanted [`OUTBID`] times as
    /// much. So a full pool comes to hold what is wanted most, whatever came first, and what a
    /// view only fetched ahead, or kept though hidden, makes way for what is seen.
    ///
    /// `bids` is how wanted each page needing a slot is: those on their way, and those absent.
    /// The pages found count as reclaimable, and may be evicted, until the next plan.
    fn find_outbid(&mut self, frame: u32, mut bids: Vec<u32>) {
        let slots = self.free_slots.iter().map(|band| band.len()).sum::<usize>()
            + self.reclaimable_at_plan.iter().sum::<u32>() as usize;
        if bids.len() <= slots {
            return;
        }
        // Left out: the least wanted, past what the empty and unwanted slots hold.
        bids.select_nth_unstable_by(slots, |a, b| b.cmp(a));
        let bids = &mut bids[slots..];
        bids.sort_unstable_by(|a, b| b.cmp(a));
        let best = bids[0];
        // What no view sees yet outbids nothing: it would only be in the way in its turn.
        if best <= REQUEST_AHEAD {
            return;
        }
        let mut standing: Vec<(u32, std::cmp::Reverse<u32>)> = self
            .installed
            .range(self.pinned..)
            .filter(|&&page| {
                self.installed_dependants[page as usize] == 0
                    && self.is_wanted(page as usize, frame)
            })
            .map(|&page| {
                (
                    self.asked(page as usize, frame).max(1),
                    std::cmp::Reverse(page),
                )
            })
            .filter(|&(asked, _)| asked.saturating_mul(OUTBID) <= best)
            .collect();
        // The least wanted first, and of those the finest, which change the image least.
        if standing.len() > bids.len() {
            standing.select_nth_unstable(bids.len());
            standing.truncate(bids.len());
        }
        standing.sort_unstable();
        for (&bid, &(asked, std::cmp::Reverse(page))) in bids.iter().zip(&standing) {
            if bid <= REQUEST_AHEAD || bid < asked.saturating_mul(OUTBID) {
                break;
            }
            self.outbid[page as usize] = self.plans;
            self.reclaimable_at_plan[usize::from(self.bands[page as usize])] += 1;
        }
    }

    /// Records that a page was asked for with `wanted` at `frame`.
    fn ask(&mut self, page: usize, frame: u32, wanted: u32) {
        let span = frame / ASKED_FRAMES;
        let asked = &mut self.asked[page];
        match span.wrapping_sub(self.asked_span[page]) {
            0 => asked[0] = asked[0].max(wanted),
            1 => *asked = [wanted, asked[0]],
            _ => *asked = [wanted, 0],
        }
        self.asked_span[page] = span;
    }

    /// How wanted a page is at `frame`: the most it was asked for with in this span of
    /// [`ASKED_FRAMES`] and the one before, so over at least one span's length and at most two,
    /// and no less than a view kept from frame to frame asked for it with in the last
    /// [`KEPT_ASKED_FRAMES`] of such views' redrawing.
    fn asked(&self, page: usize, frame: u32) -> u32 {
        let asked = self.asked[page];
        let lately = match (frame / ASKED_FRAMES).wrapping_sub(self.asked_span[page]) {
            0 => asked[0].max(asked[1]),
            1 => asked[0],
            _ => 0,
        };
        let (kept, at) = self.kept_asked[page];
        if self.kept_redrawn - at <= KEPT_ASKED_FRAMES {
            lately.max(kept)
        } else {
            lately
        }
    }

    /// Whether a view kept from frame to frame asked for a page, and views of its kind have not
    /// been redrawn for [`HOLD_FRAMES`] since.
    fn is_held(&self, page: usize) -> bool {
        let count = self.states.len();
        self.kept.iter().enumerate().any(|(kind, kept)| {
            let at = self.held_at[kind * count + page];
            at != 0 && kept.redrawn - (at - 1) <= HOLD_FRAMES
        })
    }

    /// Whether a page is still wanted: it keeps its slot unless outbid.
    fn is_wanted(&self, page: usize, frame: u32) -> bool {
        frame.saturating_sub(self.last_wanted[page]) <= self.protected_frames || self.is_held(page)
    }

    /// How many more pages could be brought into each band: into slots that are empty, and in
    /// all, counting the slots of pages that could leave.
    fn room(&self) -> (Vec<u32>, Vec<u32>) {
        let mut coming = self.fetching_by_band.clone();
        for &page in &self.arrived {
            coming[usize::from(self.bands[page as usize])] += 1;
        }
        let bands = 0..self.free_slots.len();
        let free = bands
            .clone()
            .map(|band| (self.free_slots[band].len() as u32).saturating_sub(coming[band]))
            .collect();
        let all = bands
            .map(|band| {
                (self.free_slots[band].len() as u32 + self.reclaimable_at_plan[band])
                    .saturating_sub(coming[band])
            })
            .collect();
        (free, all)
    }

    /// Installed pages of each band that could be evicted, now or once what depends on them has
    /// gone.
    fn reclaimable(&mut self, frame: u32) -> Vec<u32> {
        let mut needed = std::mem::take(&mut self.needed);
        let mut count = vec![0; self.free_slots.len()];
        // Highest first: what a page depends on has a lower id, so its dependants are decided.
        for &page in self.installed.range(self.pinned..).rev() {
            let page = page as usize;
            let recent = self.is_wanted(page, frame);
            let held = self.dependants[page]
                .iter()
                .any(|&d| needed[d as usize] && self.is_installed(d));
            needed[page] = recent || held;
            if !needed[page] {
                count[usize::from(self.bands[page])] += 1;
            }
        }
        for &page in self.installed.range(self.pinned..) {
            needed[page as usize] = false;
        }
        self.needed = needed;
        count
    }

    #[cfg(test)]
    fn mark_fetching(&mut self, page: u32) {
        if self.states[page as usize] == State::Absent {
            self.states[page as usize] = State::Fetching;
            self.fetching += 1;
            self.fetching_by_band[usize::from(self.bands[page as usize])] += 1;
        }
    }

    /// The data for a page has come. Returns false if the page's data isn't needed any more.
    pub fn arrived(&mut self, page: u32) -> bool {
        if self.states[page as usize] != State::Fetching {
            return false;
        }
        self.fetching -= 1;
        self.fetching_by_band[usize::from(self.bands[page as usize])] -= 1;
        self.states[page as usize] = State::Arrived;
        self.arrived.insert(page);
        true
    }

    /// A fetch failed. The page can be requested again.
    pub fn failed(&mut self, page: u32) {
        if self.states[page as usize] == State::Fetching {
            self.fetching -= 1;
            self.fetching_by_band[usize::from(self.bands[page as usize])] -= 1;
            self.states[page as usize] = State::Absent;
            self.bands[page as usize] = self.home_bands[page as usize];
        }
    }

    /// Whether a page's data is still being held for it.
    pub fn is_waiting(&self, page: u32) -> bool {
        self.states[page as usize] == State::Arrived
    }

    /// Finds the next page ready to install, and gives it a slot, evicting if it must. Returns
    /// `None` when nothing can install this frame. Coarsest pages go first. A band with no slot
    /// to give holds back only its own pages.
    pub fn next_install(&mut self, frame: u32) -> Option<Install> {
        let mut stuck = 0u32;
        loop {
            let page = self
                .arrived
                .iter()
                .copied()
                .find(|&page| self.can_install(page, stuck))?;
            let band = usize::from(self.bands[page as usize]);
            let Some((slot, evicted)) = self.take_slot(frame, band) else {
                stuck |= 1 << band;
                continue;
            };
            self.settle(page, slot, frame);
            return Some(Install {
                page,
                slot,
                evicted,
            });
        }
    }

    /// Installs the arrived pages that can go in, coarsest first, at most `limit` of them, in
    /// one look over them. Returns what changed on the GPU, in the order to make the changes:
    /// a slot's old page leaves before its new one arrives. The pages [`Self::take_released`]
    /// would give are among them.
    pub fn install_ready(&mut self, frame: u32, limit: u32) -> Vec<Step> {
        let mut steps = Vec::new();
        let mut installed = 0;
        let mut stuck = 0u32;
        // In order, so a page's parents, which have lower ids, are in before its turn comes.
        let waiting: Vec<u32> = self.arrived.iter().copied().collect();
        for page in waiting {
            if installed == limit {
                break;
            }
            if !self.can_install(page, stuck) {
                continue;
            }
            let band = usize::from(self.bands[page as usize]);
            let slot = self.take_slot(frame, band);
            steps.extend(self.released.drain(..).map(Step::Evict));
            let Some((slot, evicted)) = slot else {
                stuck |= 1 << band;
                continue;
            };
            steps.extend(evicted.map(Step::Evict));
            self.settle(page, slot, frame);
            steps.push(Step::Install { page, slot });
            installed += 1;
        }
        steps
    }

    /// Whether an arrived page's parents are in, and its band isn't among the `stuck`.
    fn can_install(&self, page: u32, stuck: u32) -> bool {
        stuck & (1 << self.bands[page as usize]) == 0
            && self.dependencies[page as usize]
                .iter()
                .all(|&d| self.is_installed(d))
    }

    /// A slot of `band` for a page to go into, and the page that left it, if one had to.
    fn take_slot(&mut self, frame: u32, band: usize) -> Option<(u32, Option<u32>)> {
        if let Some(slot) = self.free_slots[band].pop() {
            return Some((slot, None));
        }
        let victim = self.victim_making_room(frame, band)?;
        let State::Installed(slot) = self.states[victim as usize] else {
            unreachable!("victims are installed");
        };
        self.remove(victim);
        self.reclaimable_at_plan[band] = self.reclaimable_at_plan[band].saturating_sub(1);
        Some((slot, Some(victim)))
    }

    /// Records an arrived page as installed in `slot`.
    fn settle(&mut self, page: u32, slot: u32, frame: u32) {
        self.arrived.remove(&page);
        self.place(page, slot);
        let asked = std::mem::take(&mut self.awaited[page as usize]);
        if asked != 0 && frame.saturating_sub(asked - 1) <= AWAIT_FRAMES {
            // Kept until the views that asked have been redrawn with it, and asked again.
            self.awaited_arrival |= self.awaited_by[page as usize];
            let until = frame.saturating_add(HOLD_FRAMES);
            self.last_wanted[page as usize] = self.last_wanted[page as usize].max(until);
        }
    }

    /// Installs a page at once, whatever its state. Its dependencies must be installed.
    pub fn install_now(&mut self, page: u32) -> u32 {
        assert!(
            self.dependencies[page as usize]
                .iter()
                .all(|&d| self.is_installed(d)),
            "dependencies install first"
        );
        let band = usize::from(self.bands[page as usize]);
        let slot = self.free_slots[band].pop().expect("a free slot");
        self.arrived.remove(&page);
        if self.states[page as usize] == State::Fetching {
            self.fetching -= 1;
            self.fetching_by_band[band] -= 1;
        }
        self.place(page, slot);
        slot
    }

    fn place(&mut self, page: u32, slot: u32) {
        self.states[page as usize] = State::Installed(slot);
        self.installed.insert(page);
        for index in 0..self.dependencies[page as usize].len() {
            let dependency = self.dependencies[page as usize][index];
            self.installed_dependants[dependency as usize] += 1;
        }
    }

    fn remove(&mut self, page: u32) {
        self.states[page as usize] = State::Absent;
        self.outbid[page as usize] = 0;
        self.bands[page as usize] = self.home_bands[page as usize];
        self.installed.remove(&page);
        for index in 0..self.dependencies[page as usize].len() {
            let dependency = self.dependencies[page as usize][index];
            self.installed_dependants[dependency as usize] -= 1;
            // Its last dependant has gone, so it may go next, with no need to look again.
            if self.installed_dependants[dependency as usize] == 0 && dependency >= self.pinned {
                self.victims[usize::from(self.bands[dependency as usize])].push(dependency);
            }
        }
    }

    /// The least recently wanted page that nothing depends on, unprotected. Ties go to the
    /// finest page, which has the least effect on the image.
    /// Pages let go since the last call to make room in another band.
    pub fn take_released(&mut self) -> Vec<u32> {
        std::mem::take(&mut self.released)
    }

    /// The kinds of view kept from frame to frame, a bit for each, that asked for a page that
    /// has installed since the last call: those views should be redrawn.
    pub fn take_awaited_arrival(&mut self) -> u8 {
        std::mem::take(&mut self.awaited_arrival)
    }

    /// A page to evict from `band`. A page held only by children in other bands can't go, and
    /// those bands may have no reason to let the children go, so when a band is stuck, unwanted
    /// pages of other bands are released until it isn't.
    fn victim_making_room(&mut self, frame: u32, band: usize) -> Option<u32> {
        /// Enough for a chain of children, without clearing other bands for one page.
        const MAX_RELEASES: u32 = 16;
        for _ in 0..MAX_RELEASES {
            if let Some(victim) = self.choose_victim(frame, band) {
                return Some(victim);
            }
            let other = (0..self.free_slots.len())
                .filter(|&other| other != band)
                .find_map(|other| Some((other, self.choose_victim(frame, other)?)))?;
            let (other, page) = other;
            let State::Installed(slot) = self.states[page as usize] else {
                unreachable!("victims are installed");
            };
            self.remove(page);
            self.free_slots[other].push(slot);
            self.reclaimable_at_plan[other] = self.reclaimable_at_plan[other].saturating_sub(1);
            self.released.push(page);
        }
        self.choose_victim(frame, band)
    }

    fn choose_victim(&mut self, frame: u32, band: usize) -> Option<u32> {
        if self.victims_frame != Some(frame) {
            self.find_victims(frame);
        }
        // Evicting a page adds any parent that frees to its list, so once a frame is enough.
        self.victim_from_list(frame, band)
    }

    /// The likeliest page in a band's list that may still be evicted.
    fn victim_from_list(&mut self, frame: u32, band: usize) -> Option<u32> {
        while let Some(page) = self.victims[band].pop() {
            if self.may_be_evicted(page as usize, frame) {
                return Some(page);
            }
        }
        None
    }

    fn may_be_evicted(&self, page: usize, frame: u32) -> bool {
        self.is_installed(page as u32)
            && self.installed_dependants[page] == 0
            && (!self.is_wanted(page, frame) || self.is_outbid(page))
    }

    /// Whether the last plan found a page wanted far less than one without a slot.
    fn is_outbid(&self, page: usize) -> bool {
        self.outbid[page] == self.plans
    }

    /// Lists every page that may be evicted, by band, likeliest last so it pops first: those
    /// nobody wants, least recently wanted first, and then those outbid, least wanted first.
    fn find_victims(&mut self, frame: u32) {
        let mut lists = vec![Vec::new(); self.victims.len()];
        for &page in self.installed.range(self.pinned..) {
            if self.may_be_evicted(page as usize, frame) {
                lists[usize::from(self.bands[page as usize])].push(page);
            }
        }
        for list in &mut lists {
            list.sort_unstable_by_key(|&page| {
                let outbid = self.is_outbid(page as usize);
                let order = if outbid {
                    self.asked(page as usize, frame)
                } else {
                    self.last_wanted[page as usize]
                };
                (!outbid, std::cmp::Reverse((order, std::cmp::Reverse(page))))
            });
        }
        self.victims = lists;
        self.victims_frame = Some(frame);
    }
}

#[cfg(test)]
mod tests {
    /// A pool with no room borrows what another has free, and gets its page back home later.
    /// Evicting a page frees its parent to go as well, without another look over the pages.
    #[test]
    fn a_parent_can_follow_its_last_child_out_in_one_frame() {
        // A chain, 0 <- 1 <- 2, in one slot-starved band, with two more pages wanted and no
        // slot free anywhere.
        let deps = vec![vec![], vec![0], vec![1], vec![], vec![]];
        let mut residency = Residency::with_bands(deps, 0, vec![0; 5], &[3, 0]);
        residency.plan(1, &[10, 10, 10, 0, 0], 8);
        for page in 0..3 {
            residency.arrived(page);
        }
        while residency.next_install(1).is_some() {}
        assert_eq!(residency.installed_count(), 3);

        let frame = PROTECTED_FRAMES + 10;
        residency.plan(frame, &[0, 0, 0, 10, 10], 8);
        residency.arrived(3);
        residency.arrived(4);
        let mut evicted = Vec::new();
        while let Some(install) = residency.next_install(frame) {
            evicted.extend(install.evicted);
        }
        assert_eq!(
            evicted,
            [2, 1],
            "the child and then its parent, in the same frame"
        );
    }

    /// A band with nothing to let go holds back its own pages, and no other band's.
    #[test]
    fn a_band_with_no_room_does_not_hold_up_the_others() {
        // Pages 0 and 1 belong in the first band, which has one slot; page 2 in the second.
        let mut residency = Residency::with_bands(vec![vec![]; 3], 0, vec![0, 0, 1], &[1, 1]);
        residency.plan(1, &[10, 0, 0], 8);
        residency.arrived(0);
        assert!(residency.next_install(1).is_some());

        // Page 0 is still wanted, so page 1 has nowhere to go; page 2 has its own band's slot.
        for page in [1, 2] {
            residency.mark_fetching(page);
            residency.arrived(page);
        }
        residency.plan(2, &[10, 10, 10], 8);
        assert_eq!(
            residency.install_ready(2, 8),
            [Step::Install { page: 2, slot: 1 }],
            "the second band's page goes in past the first band's, which waits"
        );
        assert!(residency.is_waiting(1));
        assert_eq!(residency.next_install(2), None);
    }

    /// Once as many pages wait to go in as may be fetched at once, no more are fetched.
    #[test]
    fn pages_waiting_to_go_in_hold_back_fetching() {
        let mut residency = Residency::new(vec![vec![]; 6], 0, 6);
        assert_eq!(residency.plan(1, &[10; 6], 2), [0, 1]);
        residency.arrived(0);
        residency.arrived(1);
        assert_eq!(
            residency.refill(2),
            [],
            "two wait to go in: no more are fetched"
        );
        assert!(residency.next_install(1).is_some());
        assert_eq!(residency.refill(2), [2, 3], "one went in: fetching goes on");
    }

    /// Pages that wait for one whose fetch failed don't stop it being fetched again, however
    /// many they are, nor do those that wait for them.
    #[test]
    fn pages_waiting_for_a_failed_fetch_do_not_hold_back_fetching_it_again() {
        // Page 0, its two children, and a child of the first of those.
        let mut residency = Residency::new(vec![vec![], vec![0], vec![0], vec![1]], 0, 4);
        assert_eq!(residency.plan(1, &[10; 4], 2), [0, 1]);
        residency.arrived(1);
        assert_eq!(residency.refill(2), [2]);
        residency.arrived(2);
        residency.failed(0);
        assert_eq!(residency.next_install(1), None, "their parent isn't in");

        assert_eq!(
            residency.plan(2, &[10; 4], 2),
            [0, 3],
            "two wait, as many as may be fetched at once, but for a fetch and not the pool"
        );
        residency.arrived(3);
        residency.arrived(0);
        assert_eq!(residency.refill(2), [], "now they wait on the pool");
        let mut order = Vec::new();
        while let Some(install) = residency.next_install(2) {
            order.push(install.page);
        }
        assert_eq!(order, [0, 1, 2, 3]);
    }

    /// A page takes an empty slot of another band before it makes a page of its own leave.
    #[test]
    fn an_empty_slot_elsewhere_comes_before_evicting() {
        let mut residency = Residency::with_bands(vec![vec![]; 3], 0, vec![0; 3], &[2, 1]);
        residency.plan(1, &[10, 10, 0], 8);
        residency.arrived(0);
        residency.arrived(1);
        while residency.next_install(1).is_some() {}

        // Long after, with nothing asking for the first two.
        let frame = PROTECTED_FRAMES + 10;
        assert_eq!(residency.plan(frame, &[0, 0, 10], 8), [2]);
        residency.arrived(2);
        let install = residency.next_install(frame).expect("there is a slot");
        assert_eq!(install.evicted, None, "the other band had a slot");
        assert_eq!(residency.installed_count(), 3);
    }

    /// What a view kept from frame to frame asks for is held past the usual protection, and
    /// its coming is reported, once.
    #[test]
    fn a_held_page_is_kept_and_its_coming_reported() {
        let mut residency = Residency::new(vec![vec![], vec![0], vec![]], 0, 2);
        // Page 1 is held, by the second kind of view; its parent, page 0, is held with it.
        assert_eq!(
            residency.plan_holding(1, &[0, 10, 0], &[&[], &[0b10]], 8),
            [0, 1]
        );
        residency.arrived(0);
        residency.arrived(1);
        assert_eq!(residency.take_awaited_arrival(), 0);
        while residency.next_install(2).is_some() {}
        assert_eq!(
            residency.take_awaited_arrival(),
            0b10,
            "for the kind that asked"
        );
        assert_eq!(residency.take_awaited_arrival(), 0, "reported once");

        // Something else wants in, after the usual protection but within the hold: no room.
        let frame = 2 + PROTECTED_FRAMES + 5;
        residency.plan(frame, &[0, 0, 10], 8);
        assert!(!residency.is_installed(2));
        assert_eq!(residency.installed_count(), 2);

        // Nor is there long after, while the view that asked is not redrawn: it stands on what
        // it asked for.
        let frame = 3 * (HOLD_FRAMES + PROTECTED_FRAMES);
        assert!(residency.plan(frame, &[0, 0, 10], 8).is_empty());

        // Once views of its kind have been redrawn for the length of the hold without asking
        // again, the held pages make way.
        for redrawn in 1..=HOLD_FRAMES {
            let started = residency.plan_holding(frame + redrawn, &[0, 0, 10], &[&[], &[0b100]], 8);
            assert!(started.is_empty(), "after {redrawn} frames of redrawing");
        }
        let frame = frame + HOLD_FRAMES + 1;
        assert_eq!(
            residency.plan_holding(frame, &[0, 0, 10], &[&[], &[0b100]], 8),
            [2]
        );
        residency.arrived(2);
        assert_eq!(
            residency.next_install(frame).and_then(|i| i.evicted),
            Some(1)
        );
        assert_eq!(residency.take_awaited_arrival(), 0b10);
    }

    /// A kind of kept view that is drawn no more does not keep its pages for ever.
    #[test]
    fn what_a_kept_view_asked_for_makes_way_once_the_view_is_long_gone() {
        let mut r = Residency::new(vec![Vec::new(); 2], 0, 1);
        r.install_now(0);
        r.plan_holding(1, &[4, 0], &[&[1]], 16);
        // Only fetched ahead of need, the other page outbids nothing.
        let requests = [0, REQUEST_AHEAD];
        for frame in 2..=KEPT_ASKED_FRAMES + 1 {
            assert!(r.plan(frame, &requests, 16).is_empty(), "at frame {frame}");
        }
        let started: Vec<u32> = (0..HOLD_FRAMES + 2)
            .flat_map(|turn| r.plan(KEPT_ASKED_FRAMES + 2 + turn, &requests, 16))
            .collect();
        assert_eq!(started, [1]);
    }

    #[test]
    fn counting_installed_pages_follows_installs_and_evictions() {
        let mut residency = Residency::new(vec![Vec::new(); 3], 1, 2);
        residency.install_now(0);
        assert_eq!(residency.installed_count(), 1);
        residency.plan(1, &[0, 10, 0], 8);
        residency.arrived(1);
        residency.next_install(1).expect("room");
        assert_eq!(residency.installed_count(), 2);
        assert_eq!(residency.installed_by_band(), [2]);
        residency.remove(1);
        assert_eq!(residency.installed_count(), 1);
    }

    #[test]
    fn a_full_band_borrows_from_one_with_room() {
        // Three pages at home in band 1, which has one slot. Band 0 has two, and no pages.
        let mut residency = Residency::with_bands(vec![Vec::new(); 3], 0, vec![1, 1, 1], &[2, 1]);
        let fetched = residency.plan(1, &[10; 3], 8);
        assert_eq!(fetched.len(), 3, "the other band's slots are used");
        for &page in &fetched {
            residency.arrived(page);
        }
        let mut slots = Vec::new();
        while let Some(install) = residency.next_install(1) {
            assert_eq!(install.evicted, None);
            slots.push(install.slot);
        }
        slots.sort_unstable();
        assert_eq!(slots, [0, 1, 2]);
        assert_eq!(residency.installed_by_band(), [2, 1]);

        // Unwanted later, a borrowed page goes home: band 1 gets its first claim on a new one.
        let later = PROTECTED_FRAMES + 10;
        let wanted = vec![0u32; 3];
        residency.plan(later, &wanted, 8);
        let page = fetched
            .iter()
            .copied()
            .find(|&p| residency.bands[p as usize] == 0)
            .expect("one was lent");
        residency.remove(page);
        assert_eq!(residency.bands[page as usize], 1);
    }

    /// Lending must not take what a band's own wanted pages need.
    #[test]
    fn a_band_keeps_the_slots_its_wanted_pages_use() {
        // Page 0 lives in band 0's only slot and stays wanted. Pages 1 and 2 are at home in band
        // 1, which has one slot, so one of them must wait.
        let mut residency = Residency::with_bands(vec![Vec::new(); 3], 0, vec![0, 1, 1], &[1, 1]);
        residency.plan(1, &[10, 0, 0], 8);
        residency.arrived(0);
        assert!(residency.next_install(1).is_some());
        let fetched = residency.plan(2, &[10, 10, 10], 8);
        assert_eq!(fetched.len(), 1, "band 0's slot is in use: {fetched:?}");
    }

    #[test]
    fn fetches_keep_coming_between_read_backs() {
        // Six independent pages, of which only two can be fetched at once.
        let mut residency = Residency::new(vec![Vec::new(); 6], 0, 6);
        let mut requests = vec![10u32; 6];
        requests[5] = 90;
        let first = residency.plan(1, &requests, 2);
        assert_eq!(first.len(), 2);
        assert_eq!(first[0], 5, "most wanted first");

        assert!(residency.refill(2).is_empty(), "the limit still holds");
        assert!(residency.arrived(first[0]));
        let more = residency.refill(2);
        assert_eq!(more.len(), 1, "a finished fetch makes room for the next");
        assert!(!first.contains(&more[0]));

        // A new plan replaces what was queued, and nothing is fetched twice.
        let next = residency.plan(2, &requests, 8);
        let everything: Vec<u32> = first.iter().chain(&more).chain(&next).copied().collect();
        let mut unique = everything.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), everything.len());
        assert_eq!(unique.len(), 6);
    }

    #[test]
    fn a_band_uses_and_evicts_only_its_own_slots() {
        // One slot each. Page 0 is in band 0; pages 1 and 2 share band 1.
        let mut residency = Residency::with_bands(vec![Vec::new(); 3], 0, vec![0, 1, 1], &[1, 1]);
        let requests = vec![10u32; 3];
        let fetched = residency.plan(1, &requests, 8);
        assert_eq!(fetched.len(), 2, "band 1 has room for only one of its two");
        for page in fetched {
            residency.arrived(page);
        }
        let mut slots = Vec::new();
        while let Some(install) = residency.next_install(1) {
            slots.push(install.slot);
            assert_eq!(install.evicted, None);
        }
        slots.sort_unstable();
        assert_eq!(slots, [0, 1], "one slot from each band's own range");

        // Band 0 now wants nothing, and band 1's other page takes its neighbour's place.
        let frame = PROTECTED_FRAMES + 10;
        let installed_band_one = if residency.is_installed(1) { 1 } else { 2 };
        let other = 3 - installed_band_one;
        let mut requests = vec![0u32; 3];
        requests[other as usize] = 10;
        assert_eq!(residency.plan(frame, &requests, 8), [other]);
        residency.arrived(other);
        let install = residency.next_install(frame).unwrap();
        assert_eq!(install.evicted, Some(installed_band_one));
        assert!(
            residency.is_installed(0),
            "the other band's page is untouched"
        );
    }

    /// A far page held by a near child mustn't block the far band while the near band sits idle.
    #[test]
    fn a_stuck_band_gets_its_room_from_the_others() {
        // Band 1's page 1 depends on band 0's page 0, and there's one slot in each.
        let deps = vec![vec![], vec![0], vec![], vec![2]];
        let mut residency = Residency::with_bands(deps, 0, vec![0, 1, 0, 1], &[1, 1]);
        residency.plan(1, &[10, 10, 0, 0], 8);
        residency.arrived(0);
        residency.arrived(1);
        while residency.next_install(1).is_some() {}
        assert!(residency.is_installed(1));

        // Page 2, in band 0, is wanted later: page 0 is its band's only page and is held by 1.
        let frame = PROTECTED_FRAMES + 10;
        residency.plan(frame, &[0, 0, 10, 0], 8);
        residency.arrived(2);
        let install = residency.next_install(frame).expect("room was made");
        assert_eq!(install.page, 2);
        assert_eq!(install.evicted, Some(0));
        assert_eq!(residency.take_released(), [1], "its child went first");
        assert!(residency.take_released().is_empty());
    }

    #[test]
    fn a_new_plan_replaces_what_was_queued() {
        let mut residency = Residency::new(vec![Vec::new(); 6], 0, 6);
        let first = residency.plan(1, &[10; 6], 1);
        assert_eq!(first.len(), 1);
        residency.arrived(first[0]);
        assert!(residency.next_install(1).is_some());

        // Now only one other page is wanted, so the rest of the old queue must not be fetched.
        let wanted = (first[0] + 1) % 6;
        let mut requests = vec![0; 6];
        requests[wanted as usize] = 10;
        assert_eq!(residency.plan(2, &requests, 1), [wanted]);
        residency.arrived(wanted);
        assert!(residency.refill(4).is_empty());
    }

    #[test]
    fn refill_does_not_overcommit_the_pool() {
        // Three slots, all full of pages nobody wants any more.
        let mut residency = Residency::new(vec![Vec::new(); 7], 0, 3);
        for page in 0..3 {
            residency.install_now(page);
        }
        let frame = PROTECTED_FRAMES + 10;
        let mut requests = vec![0; 7];
        requests[3..7].fill(10);
        // Room for three, one fetched at a time: the fourth must wait for a later plan.
        let mut fetched = residency.plan(frame, &requests, 1);
        for _ in 0..8 {
            for page in std::mem::take(&mut fetched) {
                assert!(residency.arrived(page));
            }
            while residency.next_install(frame).is_some() {}
            fetched = residency.refill(1);
        }
        assert_eq!(residency.installed_count(), 3, "the pool is full");
        assert_eq!(residency.in_transit(), 0, "and nothing is left stranded");
    }

    use super::*;

    /// A small DAG: page 0 is pinned, 1 and 2 hang off it, 3 needs both, 4 needs 2.
    fn dag(slots: u32) -> Residency {
        let mut r = Residency::new(
            vec![vec![], vec![0], vec![0], vec![1, 2], vec![2]],
            1,
            slots,
        );
        r.install_now(0);
        r
    }

    fn want(pages: &[(u32, u32)], count: usize) -> Vec<u32> {
        let mut requests = vec![0; count];
        for &(page, priority) in pages {
            requests[page as usize] = priority;
        }
        requests
    }

    fn install_all(r: &mut Residency, frame: u32) -> Vec<Install> {
        let mut out = Vec::new();
        while let Some(install) = r.next_install(frame) {
            out.push(install);
        }
        out
    }

    #[test]
    fn plans_parents_of_what_is_wanted_most_wanted_first() {
        let mut r = dag(5);
        let fetch = r.plan(1, &want(&[(3, 10), (4, 50)], 5), 16);
        // Page 4 is most wanted, and page 2 must come before it at the same priority. Page 1
        // is only needed by the less wanted page 3.
        assert_eq!(fetch, vec![2, 4, 1, 3]);
    }

    #[test]
    fn nothing_is_fetched_twice_or_past_the_limit() {
        let mut r = dag(5);
        let requests = want(&[(3, 10), (4, 10)], 5);
        assert_eq!(r.plan(1, &requests, 2).len(), 2);
        assert_eq!(
            r.plan(2, &requests, 2).len(),
            0,
            "both slots of the limit are used"
        );
        r.arrived(1);
        let next = r.plan(3, &requests, 2);
        assert_eq!(next.len(), 1);
        assert!(!next.contains(&1));
    }

    #[test]
    fn occluded_pages_are_kept_but_not_loaded() {
        let mut r = dag(5);
        assert!(r.plan(1, &want(&[(1, REQUEST_KEEP)], 5), 16).is_empty());
    }

    #[test]
    fn installs_wait_for_dependencies() {
        let mut r = dag(5);
        for page in r.plan(1, &want(&[(3, 10)], 5), 16) {
            assert!(r.dependencies[page as usize].iter().all(|&d| d != 3));
        }
        assert!(r.arrived(3));
        assert_eq!(r.next_install(1), None, "parents haven't come");
        assert!(r.arrived(1));
        assert_eq!(r.next_install(1).map(|i| i.page), Some(1));
        assert_eq!(r.next_install(1), None, "page 2 is still missing");
        assert!(r.arrived(2));
        let pages: Vec<u32> = install_all(&mut r, 1).iter().map(|i| i.page).collect();
        assert_eq!(pages, vec![2, 3]);
    }

    /// Fills a pool of 3 with pages 0, 1 and 2 at `frame`.
    fn filled(frame: u32) -> Residency {
        let mut r = dag(3);
        let fetch = r.plan(frame, &want(&[(1, 9), (2, 9)], 5), 16);
        for page in fetch {
            r.arrived(page);
        }
        install_all(&mut r, frame);
        r
    }

    #[test]
    fn eviction_only_takes_old_pages_with_no_installed_dependants() {
        let mut r = filled(1);
        // Page 4 needs page 2, so page 1 must leave. Page 2 is still wanted through page 4.
        let later = PROTECTED_FRAMES + 2;
        let fetch = r.plan(later, &want(&[(4, 9)], 5), 16);
        assert_eq!(fetch, vec![4]);
        r.arrived(4);
        let install = r.next_install(later).unwrap();
        assert_eq!(install.page, 4);
        assert_eq!(install.evicted, Some(1));
        assert_eq!(r.installed_dependants[2], 1);
    }

    #[test]
    fn recently_wanted_pages_are_safe_and_the_load_waits() {
        let mut r = filled(1);
        // Page 1 and 2 are both protected, so there is no room to fetch page 4.
        let fetch = r.plan(2, &want(&[(4, 9), (1, 9)], 5), 16);
        assert!(fetch.is_empty());
        assert_eq!(r.starved(), 1);
        assert!(r.is_installed(1));
    }

    #[test]
    fn pinned_pages_and_parents_of_installed_pages_never_leave() {
        let mut r = dag(4);
        for page in r.plan(1, &want(&[(4, 9), (1, 9)], 5), 16) {
            r.arrived(page);
        }
        install_all(&mut r, 1);
        // Pages 0 (pinned), 1, 2 and 4 fill the pool. Far in the future, only leaves can go.
        let frame = 1000;
        for page in r.plan(frame, &want(&[(3, 9)], 5), 16) {
            r.arrived(page);
        }
        // Page 3 needs 1 and 2, so the only page free to leave is 4.
        let install = r.next_install(frame).unwrap();
        assert_eq!((install.page, install.evicted), (3, Some(4)));
        assert!(r.is_installed(0) && r.is_installed(1) && r.is_installed(2));
    }

    #[test]
    fn a_failed_fetch_can_be_tried_again() {
        let mut r = dag(5);
        let requests = want(&[(1, 9)], 5);
        assert_eq!(r.plan(1, &requests, 16), vec![1]);
        r.failed(1);
        assert_eq!(r.plan(2, &requests, 16), vec![1]);
    }

    #[test]
    fn a_page_nobody_wants_stops_waiting_for_a_slot() {
        let mut r = dag(1);
        r.mark_fetching(1);
        assert!(r.arrived(1));
        assert!(r.is_waiting(1));
        r.plan(ARRIVED_PATIENCE_FRAMES + 5, &[0; 5], 16);
        assert!(!r.is_waiting(1));
        assert_eq!(r.in_transit(), 0);
    }

    /// A pool of three, full of pages 0, 1 and 2, each last asked for at `frame` with the
    /// priority given. Three more pages have no slot.
    fn full_of(priorities: [u32; 3], frame: u32) -> Residency {
        let mut r = Residency::new(vec![Vec::new(); 6], 0, 3);
        for page in 0..3 {
            r.install_now(page);
        }
        let asked = [(0, priorities[0]), (1, priorities[1]), (2, priorities[2])];
        assert!(r.plan(frame, &want(&asked, 6), 16).is_empty());
        r
    }

    #[test]
    fn a_seen_page_takes_the_slot_of_one_kept_hidden_or_fetched_ahead() {
        // One page hidden, one fetched ahead of need, one in plain view: all asked for every
        // frame, so none would ever leave by going unwanted.
        let standing = [(0, REQUEST_KEEP), (1, REQUEST_AHEAD), (2, 40)];
        let mut r = full_of([REQUEST_KEEP, REQUEST_AHEAD, 40], 1);
        let mut requests = want(&standing, 6);
        requests[3] = 40;
        requests[4] = 6;
        assert_eq!(r.plan(2, &requests, 16), [3, 4]);
        assert_eq!(r.starved(), 0);
        r.arrived(3);
        r.arrived(4);
        // The least wanted leaves first.
        let installs = install_all(&mut r, 2);
        assert_eq!(installs[0].evicted, Some(0));
        assert_eq!(installs[1].evicted, Some(1));
        assert!(r.is_installed(2) && r.is_installed(3) && r.is_installed(4));
    }

    #[test]
    fn a_page_must_be_wanted_twice_as_much_to_take_a_slot() {
        let standing = [(0, 9), (1, 9), (2, 9)];
        let mut r = full_of([9, 9, 9], 1);
        let mut requests = want(&standing, 6);
        requests[3] = 9 * OUTBID - 1;
        assert!(r.plan(2, &requests, 16).is_empty());
        assert_eq!(r.starved(), 1);
        requests[3] = 9 * OUTBID;
        assert_eq!(r.plan(3, &requests, 16), [3]);
        // The finest page of those wanted least makes way.
        r.arrived(3);
        assert_eq!(r.next_install(3).unwrap().evicted, Some(2));
    }

    #[test]
    fn a_page_fetched_ahead_takes_no_slot_from_one_kept_hidden() {
        let mut r = full_of([REQUEST_KEEP; 3], 1);
        let mut requests = vec![REQUEST_KEEP; 6];
        requests[3] = REQUEST_AHEAD;
        assert!(r.plan(2, &requests, 16).is_empty());
        requests[3] = REQUEST_AHEAD + 1;
        assert_eq!(r.plan(3, &requests, 16), [3]);
    }

    #[test]
    fn pages_do_not_keep_changing_places() {
        let mut r = full_of([10, 30, 30], 1);
        let requests = want(&[(0, 10), (1, 30), (2, 30), (3, 20)], 6);
        assert_eq!(r.plan(2, &requests, 16), [3]);
        r.arrived(3);
        assert_eq!(r.next_install(2).unwrap().evicted, Some(0));
        // The page that left is still wanted, at half what the page that took its slot is: it
        // waits, and so does the next plan.
        for frame in 3..6 {
            assert!(r.plan(frame, &requests, 16).is_empty());
            assert!(install_all(&mut r, frame).is_empty());
            assert_eq!(r.starved(), 1);
        }
        assert!(r.is_installed(3) && !r.is_installed(0));
    }

    #[test]
    fn a_page_asked_for_by_turns_is_wanted_as_much_as_the_most_of_them() {
        let mut r = full_of([40, 40, 40], 1);
        // A view drawn now and then needs page 0 badly. The view drawn every frame hardly does,
        // and it is by that view's asking that a page half as wanted as the first would take
        // its slot, to lose it at the next redraw.
        let mut frame = 2;
        for turn in 0..3 * ASKED_FRAMES {
            let first = if turn % 20 == 0 { 40 } else { 3 };
            let requests = want(&[(0, first), (1, 40), (2, 40), (3, 20)], 6);
            assert!(r.plan(frame, &requests, 16).is_empty(), "at frame {frame}");
            frame += 1;
        }
        // Once that view has left it alone for long enough, the page is worth what the other
        // gives it.
        let requests = want(&[(0, 3), (1, 40), (2, 40), (3, 20)], 6);
        let started: Vec<u32> = (0..2 * ASKED_FRAMES)
            .flat_map(|turn| r.plan(frame + turn, &requests, 16))
            .collect();
        assert_eq!(started, [3]);
    }

    #[test]
    fn a_page_a_kept_view_asked_for_stays_as_wanted_for_long() {
        let mut r = full_of([40, 40, 40], 1);
        // A view kept from frame to frame asks once for page 0. From then on only the view
        // drawn every frame asks for it, and another kept view that sees little of it: both
        // with far less. A kept view of page 1 is redrawn every frame.
        r.plan_holding(
            2,
            &want(&[(0, 40), (1, 40), (2, 40), (3, 20)], 6),
            &[&[1]],
            16,
        );
        let requests = want(&[(0, 3), (1, 40), (2, 40), (3, 20)], 6);
        for frame in 3..KEPT_ASKED_FRAMES {
            let held: &[&[u32]] = if frame % 50 == 0 {
                &[&[0b11]]
            } else {
                &[&[0b10]]
            };
            let started = r.plan_holding(frame, &requests, held, 16);
            assert!(started.is_empty(), "at frame {frame}");
        }
        let started: Vec<u32> = (KEPT_ASKED_FRAMES..KEPT_ASKED_FRAMES + 2 * ASKED_FRAMES)
            .flat_map(|frame| r.plan_holding(frame, &requests, &[&[0b10]], 16))
            .collect();
        assert_eq!(started, [3]);
    }

    #[test]
    fn a_page_a_kept_view_asked_for_stays_as_wanted_while_none_is_redrawn() {
        let mut r = full_of([40, 40, 40], 1);
        r.plan_holding(
            2,
            &want(&[(0, 40), (1, 40), (2, 40), (3, 20)], 6),
            &[&[1]],
            16,
        );
        // No kept view is redrawn again, so none asks, and the one that asked stands on page 0
        // as it did. By the view drawn every frame alone, page 3 would take the slot.
        let requests = want(&[(0, 3), (1, 40), (2, 40), (3, 20)], 6);
        for frame in 3..KEPT_ASKED_FRAMES + 1000 {
            assert!(r.plan(frame, &requests, 16).is_empty(), "at frame {frame}");
        }
    }

    #[test]
    fn a_page_is_outbid_only_until_the_next_plan() {
        let mut r = full_of([REQUEST_KEEP, 30, 30], 1);
        let mut requests = want(&[(0, REQUEST_KEEP), (1, 30), (2, 30), (3, 20)], 6);
        assert_eq!(r.plan(2, &requests, 16), [3]);
        // Before the page comes, what it made way for comes into view itself.
        requests[0] = 30;
        r.plan(3, &requests, 16);
        r.arrived(3);
        assert!(install_all(&mut r, 3).is_empty(), "nothing may leave now");
        assert!(r.is_installed(0));
    }

    #[test]
    fn pinned_pages_and_parents_are_never_outbid() {
        // Page 0 is pinned, and page 2 hangs off page 1.
        let dependencies = vec![vec![], vec![0], vec![1], vec![], vec![]];
        let mut r = Residency::new(dependencies, 1, 3);
        for page in 0..3 {
            r.install_now(page);
        }
        r.plan(1, &want(&[(2, 50)], 5), 16);
        // The parent is now only kept, but its child, seen a frame ago, still stands on it.
        let requests = want(&[(0, REQUEST_KEEP), (1, REQUEST_KEEP), (3, 40)], 5);
        assert!(r.plan(2, &requests, 16).is_empty());
        assert_eq!(r.starved(), 1);
        // Once the child has been wanted little enough for long enough, it goes, and the parent
        // may follow it.
        let later = 2 * ASKED_FRAMES + 3;
        let requests = want(&[(0, REQUEST_KEEP), (2, REQUEST_KEEP), (3, 40), (4, 40)], 5);
        assert_eq!(r.plan(later, &requests, 16), [3]);
        r.arrived(3);
        assert_eq!(r.next_install(later).unwrap().evicted, Some(2));
        assert_eq!(r.plan(later + 1, &requests, 16), [4]);
        r.arrived(4);
        assert_eq!(r.next_install(later + 1).unwrap().evicted, Some(1));
        assert!(r.is_installed(0));
    }

    #[test]
    fn a_held_page_makes_way_for_one_wanted_far_more() {
        let mut r = Residency::new(vec![Vec::new(); 3], 0, 1);
        r.install_now(0);
        // A view kept from frame to frame asks once, and its page is held long after.
        r.plan_holding(1, &[4, 0, 0], &[&[1]], 16);
        assert!(r.plan(2, &[0, 4 * OUTBID - 1, 0], 16).is_empty());
        assert_eq!(r.plan(3, &[0, 0, 4 * OUTBID], 16), [2]);
    }

    #[test]
    fn pages_on_their_way_count_against_those_outbid() {
        let mut r = full_of([REQUEST_KEEP; 3], 1);
        let mut requests = vec![REQUEST_KEEP; 6];
        // One more page comes into view at each plan, and none has come yet: each takes the
        // slot of one page more.
        for (frame, page) in [(2, 3), (3, 4), (4, 5)] {
            requests[page as usize] = 40;
            assert_eq!(r.plan(frame, &requests, 16), [page]);
        }
        assert!(r.plan(5, &requests, 16).is_empty());
        for page in 3..6 {
            r.arrived(page);
        }
        assert_eq!(install_all(&mut r, 5).len(), 3);
        assert_eq!(r.installed_count(), 3);
        assert_eq!(r.in_transit(), 0);
    }

    /// A deterministic random run over a larger DAG: whatever is asked for, the pool never
    /// overflows, and a page is never installed without its dependencies.
    #[test]
    fn random_requests_never_break_the_rules() {
        let mut state = 0x2545_f491_4f6c_dd1du64;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        let count = 300usize;
        let pinned = 6u32;
        let mut dependencies = vec![Vec::new(); count];
        for (page, list) in dependencies.iter_mut().enumerate().skip(pinned as usize) {
            for _ in 0..1 + next() % 3 {
                list.push((next() % page as u64) as u32);
            }
            list.sort_unstable();
            list.dedup();
        }
        let slots = 40;
        let mut r = Residency::new(dependencies.clone(), pinned, slots);
        for page in 0..pinned {
            r.install_now(page);
        }

        let mut in_flight: Vec<u32> = Vec::new();
        let mut used = pinned;
        for frame in 1..3000 {
            let mut requests = vec![0; count];
            let focus = next() % count as u64;
            for _ in 0..8 {
                let page = (focus + next() % 20).min(count as u64 - 1) as usize;
                requests[page] = if next() % 4 == 0 {
                    REQUEST_KEEP
                } else {
                    2 + (next() % 100) as u32
                };
            }
            in_flight.extend(r.plan(frame, &requests, 8));
            // Some fetches finish, a few fail, in no particular order.
            for _ in 0..3 {
                if !in_flight.is_empty() {
                    let at = (next() % in_flight.len() as u64) as usize;
                    let page = in_flight.swap_remove(at);
                    if next() % 16 == 0 {
                        r.failed(page);
                    } else {
                        r.arrived(page);
                    }
                }
            }
            in_flight.extend(r.refill(8));
            while let Some(install) = r.next_install(frame) {
                used += 1;
                if install.evicted.is_some() {
                    used -= 1;
                }
                assert!(install.evicted.is_none_or(|page| page >= pinned));
            }
            assert!(used <= slots);
            assert_eq!(used, r.installed_count());
            for page in 0..count as u32 {
                if r.is_installed(page) {
                    for &dependency in &dependencies[page as usize] {
                        assert!(r.is_installed(dependency), "{page} lost {dependency}");
                    }
                }
            }
            for page in 0..pinned {
                assert!(r.is_installed(page));
            }
        }
    }
}
