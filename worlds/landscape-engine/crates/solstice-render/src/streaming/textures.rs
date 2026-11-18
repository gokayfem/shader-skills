//! Decides which textures should be sharp. No GPU code, so all of it can be tested alone.
//!
//! WebGPU has no sparse textures, and all layers of an array share one size. So each material's
//! texture lives in one of three tiers, each an array allocated up front:
//!
//! * **Base**: every layer at a small size, from the manifest. Always there.
//! * **Middle**: every layer at a medium size. Fetched when a layer is first wanted that sharp.
//! * **Top**: a few slots at the largest size the budget allows, for the layers wanted most.
//!
//! Moving up a tier only fetches the levels the tier below lacks; the rest copy across on the GPU.

use solstice_asset::TextureEncoding;
use std::collections::BTreeSet;

/// Largest edge of the middle tier, where a layer costs about 1 MB in block-compressed form.
const MIDDLE_MAX_EDGE: u32 = 512;
/// A top tier with fewer slots than this isn't worth having, so a smaller edge is used.
const MIN_TOP_SLOTS: u32 = 6;
/// Frames a top slot stays with its layer after that layer was last wanted at that size.
const TOP_PROTECTED_FRAMES: u32 = 120;
/// A layer already in the top tier gives up its slot to one wanting this many times its detail.
const TOP_STEAL_RATIO: u32 = 2;

/// Sizes of the texture tiers, fixed once chosen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TierPlan {
    pub base_edge: u32,
    /// `None` if the largest level already fits in the base tier.
    pub middle_edge: Option<u32>,
    /// Edge and number of slots, if any larger tier fits the budget.
    pub top: Option<(u32, u32)>,
}

/// Bytes one layer takes on the GPU at this edge, with its full mip chain and all three maps.
pub fn layer_bytes(edge: u32, encoding: TextureEncoding) -> u64 {
    let mut total = 0;
    let mut level = edge;
    while level > 0 {
        total += (encoding.color_mip_bytes(level) + 2 * encoding.data_mip_bytes(level)) as u64;
        level /= 2;
    }
    total
}

impl TierPlan {
    /// Picks tier sizes for `layers` textures of `size`, to fit `budget` bytes.
    ///
    /// `largest_allowed` caps the edge, for GPUs that must unpack textures. The base tier is
    /// the one part that can't shrink, so it's the only thing allowed to exceed a tiny budget.
    pub fn choose(
        size: u32,
        layers: u32,
        base_edge: u32,
        encoding: TextureEncoding,
        budget: u64,
        largest_allowed: u32,
    ) -> Self {
        let size = size.min(largest_allowed);
        let base_edge = base_edge.min(size);
        let layers64 = u64::from(layers);
        let base_cost = layers64 * layer_bytes(base_edge, encoding);
        let mut left = budget.saturating_sub(base_cost);

        let mut middle_edge = None;
        let mut edge = size.min(MIDDLE_MAX_EDGE);
        while edge > base_edge {
            let cost = layers64 * layer_bytes(edge, encoding);
            if cost <= left {
                middle_edge = Some(edge);
                left -= cost;
                break;
            }
            edge /= 2;
        }

        let mut top = None;
        let floor = middle_edge.unwrap_or(base_edge);
        if layers > 0 && middle_edge.is_some() {
            let mut edge = size;
            while edge > floor {
                let slots = left / layer_bytes(edge, encoding);
                if slots >= u64::from(MIN_TOP_SLOTS.min(layers)) {
                    top = Some((edge, (slots as u32).min(layers)));
                    break;
                }
                edge /= 2;
            }
        }
        Self {
            base_edge,
            middle_edge,
            top,
        }
    }

    /// Bytes all the tiers take together.
    pub fn bytes(&self, layers: u32, encoding: TextureEncoding) -> u64 {
        let mut total = u64::from(layers) * layer_bytes(self.base_edge, encoding);
        if let Some(edge) = self.middle_edge {
            total += u64::from(layers) * layer_bytes(edge, encoding);
        }
        if let Some((edge, slots)) = self.top {
            total += u64::from(slots) * layer_bytes(edge, encoding);
        }
        total
    }
}

/// The tiers, as materials know them.
pub const TIER_BASE: u32 = 0;
pub const TIER_MIDDLE: u32 = 1;
pub const TIER_TOP: u32 = 2;

/// What a layer is using and what's on its way.
#[derive(Clone, Debug, Default)]
struct LayerState {
    middle_ready: bool,
    /// Levels of the middle tier not yet received.
    middle_waiting: BTreeSet<u32>,
    /// Levels of the middle tier not yet asked for.
    middle_unasked: Vec<u32>,
    top_slot: Option<u32>,
    top_ready: bool,
    top_waiting: BTreeSet<u32>,
    top_unasked: Vec<u32>,
    top_since: u32,
    /// Latest detail wanted, as a texture edge.
    demand: u32,
    last_wanted_top: u32,
}

/// One level of one layer to fetch.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct MipFetch {
    pub layer: u32,
    /// Level in the file: 0 is the largest level baked.
    pub mip: u32,
}

/// Where a fetched level goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MipTarget {
    pub tier: u32,
    pub slot: u32,
    /// Level within the tier's array.
    pub tier_mip: u32,
    /// All levels of this tier for this layer are now there, so it can be used.
    pub completes: bool,
}

/// A change of which tier a layer draws from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TierChange {
    pub layer: u32,
    pub tier: u32,
    pub slot: u32,
}

/// Tracks each layer's tier, and allocates the top tier's slots.
pub struct TextureResidency {
    plan: TierPlan,
    size: u32,
    layers: Vec<LayerState>,
    free_top_slots: Vec<u32>,
    fetching: u32,
    changes: Vec<TierChange>,
}

impl TextureResidency {
    /// `size` is the largest level in the file, which is the tiers' scale.
    pub fn new(plan: TierPlan, size: u32, layers: u32) -> Self {
        let slots = plan.top.map_or(0, |(_, slots)| slots);
        Self {
            plan,
            size,
            layers: vec![LayerState::default(); layers as usize],
            free_top_slots: (0..slots).rev().collect(),
            fetching: 0,
            changes: Vec::new(),
        }
    }

    /// Levels of the file between two edges: larger than `below`, up to and including `top`.
    fn file_mips(&self, top: u32, below: u32) -> Vec<u32> {
        let mut mips = Vec::new();
        let mut edge = top;
        while edge > below {
            mips.push((self.size / edge).ilog2());
            edge /= 2;
        }
        mips
    }

    /// Takes the detail each layer wants, as a texture edge or 0 if it wasn't seen, and returns
    /// levels to start fetching, at most `max_fetching` at a time. The most wanted come first.
    pub fn plan(&mut self, frame: u32, demands: &[u32], max_fetching: u32) -> Vec<MipFetch> {
        for (layer, &demand) in self.layers.iter_mut().zip(demands) {
            layer.demand = demand;
        }
        let mut order: Vec<usize> = (0..self.layers.len())
            .filter(|&l| self.layers[l].demand > 0)
            .collect();
        order.sort_by_key(|&l| (std::cmp::Reverse(self.layers[l].demand), l));

        for &index in &order {
            let demand = self.layers[index].demand;
            if let Some((top_edge, _)) = self.plan.top {
                let middle_edge = self.plan.middle_edge.unwrap_or(self.plan.base_edge);
                if demand > middle_edge {
                    self.layers[index].last_wanted_top = frame;
                    if self.layers[index].middle_ready && self.layers[index].top_slot.is_none() {
                        self.try_start_top(index, frame, top_edge, middle_edge);
                    }
                }
            }
            if let Some(middle_edge) = self.plan.middle_edge {
                let layer = &self.layers[index];
                let untouched = !layer.middle_ready
                    && layer.middle_waiting.is_empty()
                    && layer.middle_unasked.is_empty();
                if demand > self.plan.base_edge && untouched {
                    let mips = self.file_mips(middle_edge, self.plan.base_edge);
                    self.layers[index].middle_waiting = mips.iter().copied().collect();
                    self.layers[index].middle_unasked = mips;
                }
            }
        }

        let mut out = Vec::new();
        for &index in &order {
            for _ in 0..16 {
                if self.fetching >= max_fetching {
                    return out;
                }
                let layer = &mut self.layers[index];
                let Some(mip) = layer
                    .middle_unasked
                    .pop()
                    .or_else(|| layer.top_unasked.pop())
                else {
                    break;
                };
                self.fetching += 1;
                out.push(MipFetch {
                    layer: index as u32,
                    mip,
                });
            }
        }
        out
    }

    fn try_start_top(&mut self, index: usize, frame: u32, top_edge: u32, middle_edge: u32) {
        let demand = self.layers[index].demand;
        let slot = self.free_top_slots.pop().or_else(|| {
            // Whichever slot holder is least wanted may have to give way.
            let holder = (0..self.layers.len())
                .filter(|&l| self.layers[l].top_ready)
                .min_by_key(|&l| {
                    let layer = &self.layers[l];
                    let recent =
                        frame.saturating_sub(layer.last_wanted_top) <= TOP_PROTECTED_FRAMES;
                    (recent, layer.demand, layer.last_wanted_top)
                })?;
            let layer = &self.layers[holder];
            let idle = frame.saturating_sub(layer.last_wanted_top) > TOP_PROTECTED_FRAMES;
            let settled = frame.saturating_sub(layer.top_since) > TOP_PROTECTED_FRAMES;
            let outbid = settled && layer.demand.saturating_mul(TOP_STEAL_RATIO) < demand;
            if !(idle || outbid) {
                return None;
            }
            let slot = layer.top_slot;
            let layer = &mut self.layers[holder];
            layer.top_slot = None;
            layer.top_ready = false;
            self.changes.push(TierChange {
                layer: holder as u32,
                tier: TIER_MIDDLE,
                slot: holder as u32,
            });
            slot
        });
        let Some(slot) = slot else {
            return;
        };
        let mips = self.file_mips(top_edge, middle_edge);
        let layer = &mut self.layers[index];
        layer.top_slot = Some(slot);
        layer.top_since = frame;
        layer.top_waiting = mips.iter().copied().collect();
        layer.top_unasked = mips;
    }

    /// A fetched level has come. Returns where it belongs, or `None` if nothing is waiting for
    /// it any more. Once a tier completes, call [`Self::commit`] after its lower levels copy.
    pub fn arrived(&mut self, fetch: MipFetch) -> Option<MipTarget> {
        self.fetching = self.fetching.saturating_sub(1);
        let edge = self.size >> fetch.mip;
        let layer = &mut self.layers[fetch.layer as usize];
        if layer.middle_waiting.remove(&fetch.mip) {
            let middle_edge = self.plan.middle_edge?;
            return Some(MipTarget {
                tier: TIER_MIDDLE,
                slot: fetch.layer,
                tier_mip: (middle_edge / edge).ilog2(),
                completes: layer.middle_waiting.is_empty() && layer.middle_unasked.is_empty(),
            });
        }
        if layer.top_waiting.remove(&fetch.mip) {
            let (top_edge, _) = self.plan.top?;
            return Some(MipTarget {
                tier: TIER_TOP,
                slot: layer.top_slot?,
                tier_mip: (top_edge / edge).ilog2(),
                completes: layer.top_waiting.is_empty() && layer.top_unasked.is_empty(),
            });
        }
        None
    }

    /// A fetch failed. Its level will be asked for again.
    pub fn failed(&mut self, fetch: MipFetch) {
        self.fetching = self.fetching.saturating_sub(1);
        let layer = &mut self.layers[fetch.layer as usize];
        if layer.middle_waiting.contains(&fetch.mip) {
            layer.middle_unasked.push(fetch.mip);
        } else if layer.top_waiting.contains(&fetch.mip) {
            layer.top_unasked.push(fetch.mip);
        }
    }

    /// The layer may now draw from the tier just completed.
    pub fn commit(&mut self, layer: u32, tier: u32) {
        let state = &mut self.layers[layer as usize];
        let slot = if tier == TIER_TOP {
            state.top_ready = true;
            state.top_slot.unwrap_or(layer)
        } else {
            state.middle_ready = true;
            layer
        };
        // A layer in the top tier stays there: a late middle tier doesn't demote it.
        if tier == TIER_TOP || !state.top_ready {
            self.changes.push(TierChange { layer, tier, slot });
        }
    }

    /// Tier changes since the last call, in order.
    pub fn take_changes(&mut self) -> Vec<TierChange> {
        std::mem::take(&mut self.changes)
    }

    /// The top tier slot a layer has, or is filling.
    pub fn top_slot(&self, layer: u32) -> Option<u32> {
        self.layers[layer as usize].top_slot
    }

    /// Levels being fetched.
    pub fn fetching(&self) -> u32 {
        self.fetching
    }

    /// How many layers are in the middle and top tiers.
    pub fn counts(&self) -> (u32, u32) {
        let count = |f: fn(&LayerState) -> bool| self.layers.iter().filter(|l| f(l)).count() as u32;
        (count(|l| l.middle_ready), count(|l| l.top_ready))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MB: u64 = 1 << 20;

    #[test]
    fn layer_costs_match_the_format() {
        // BC at 16 bytes per block: 3 maps at 1 byte per texel for the top level alone.
        assert_eq!(
            layer_bytes(4, TextureEncoding::Block),
            3 * (16 + 16 + 16) // 4x4, 2x2 and 1x1 each still fill one block.
        );
        let full = layer_bytes(1024, TextureEncoding::Block);
        assert!(full > 3 * 1024 * 1024 && full < 5 * 1024 * 1024);
    }

    #[test]
    fn the_browser_cove_gets_every_layer_at_512_and_some_at_1024() {
        let encoding = TextureEncoding::Block;
        let plan = TierPlan::choose(1024, 23, 128, encoding, 64 * MB, u32::MAX);
        assert_eq!(plan.base_edge, 128);
        assert_eq!(plan.middle_edge, Some(512));
        let (edge, slots) = plan.top.unwrap();
        assert_eq!(edge, 1024);
        assert!(slots >= 6, "{slots} slots");
        assert!(plan.bytes(23, encoding) <= 64 * MB);
    }

    #[test]
    fn a_larger_bake_doesnt_cost_more_but_uses_a_smaller_top_tier() {
        let encoding = TextureEncoding::Block;
        let plan = TierPlan::choose(2048, 23, 128, encoding, 64 * MB, u32::MAX);
        assert_eq!(plan.top.unwrap().0, 1024, "too few slots at 2048");
        assert!(plan.bytes(23, encoding) <= 64 * MB);
        let large = TierPlan::choose(2048, 23, 128, encoding, 512 * MB, u32::MAX);
        assert_eq!(large.top, Some((2048, 23)));
    }

    #[test]
    fn a_scene_without_textures_has_no_tiers() {
        let plan = TierPlan::choose(0, 0, 128, TextureEncoding::Raw, 64 * MB, u32::MAX);
        assert_eq!((plan.middle_edge, plan.top), (None, None));
        assert_eq!(plan.bytes(0, TextureEncoding::Raw), 0);
    }

    #[test]
    fn small_textures_and_small_budgets_keep_fewer_tiers() {
        let encoding = TextureEncoding::Block;
        let small = TierPlan::choose(128, 23, 128, encoding, 64 * MB, u32::MAX);
        assert_eq!((small.middle_edge, small.top), (None, None));

        let medium = TierPlan::choose(256, 4, 128, encoding, 64 * MB, u32::MAX);
        assert_eq!((medium.middle_edge, medium.top), (Some(256), None));

        let tight = TierPlan::choose(1024, 23, 128, encoding, 10 * MB, u32::MAX);
        assert_eq!(tight.middle_edge, Some(256), "512 doesn't fit");
        assert!(tight.bytes(23, encoding) <= 10 * MB);

        let starved = TierPlan::choose(1024, 23, 128, encoding, MB, u32::MAX);
        assert_eq!((starved.middle_edge, starved.top), (None, None));
    }

    #[test]
    fn unpacking_gpus_stop_at_the_cap() {
        let plan = TierPlan::choose(2048, 23, 128, TextureEncoding::Raw, 512 * MB, 512);
        assert_eq!(plan.middle_edge, Some(512));
        assert_eq!(plan.top, None);
    }

    fn residency(layers: u32) -> TextureResidency {
        let plan = TierPlan {
            base_edge: 128,
            middle_edge: Some(512),
            top: Some((2048, 2)),
        };
        TextureResidency::new(plan, 2048, layers)
    }

    /// Delivers every fetch and commits what completes.
    fn fulfil(r: &mut TextureResidency, fetches: &[MipFetch]) {
        for &fetch in fetches {
            let target = r.arrived(fetch).unwrap();
            if target.completes {
                r.commit(fetch.layer, target.tier);
            }
        }
    }

    #[test]
    fn nothing_loads_until_wanted_sharper_than_it_is() {
        let mut r = residency(3);
        assert!(r.plan(1, &[0, 64, 128], 16).is_empty());
    }

    #[test]
    fn a_layer_climbs_through_the_middle_tier_to_the_top() {
        let mut r = residency(1);
        let first = r.plan(1, &[4096], 16);
        // 512 and 256 are what the base tier lacks.
        let mut mips: Vec<u32> = first.iter().map(|f| f.mip).collect();
        mips.sort_unstable();
        assert_eq!(mips, vec![2, 3]);

        // The middle tier isn't usable until both have come.
        assert!(!r.arrived(first[0]).unwrap().completes);
        let target = r.arrived(first[1]).unwrap();
        assert!(target.completes);
        assert_eq!(target.tier, TIER_MIDDLE);
        r.commit(0, target.tier);
        assert_eq!(
            r.take_changes(),
            vec![TierChange {
                layer: 0,
                tier: TIER_MIDDLE,
                slot: 0
            }]
        );

        let second = r.plan(2, &[4096], 16);
        let mut mips: Vec<u32> = second.iter().map(|f| f.mip).collect();
        mips.sort_unstable();
        assert_eq!(mips, vec![0, 1], "2048 and 1024");
        fulfil(&mut r, &second);
        assert_eq!(
            r.take_changes(),
            vec![TierChange {
                layer: 0,
                tier: TIER_TOP,
                slot: 0
            }]
        );
        assert_eq!(r.counts(), (1, 1));
        assert!(r.plan(3, &[4096], 16).is_empty(), "nothing more to do");
    }

    #[test]
    fn levels_map_to_their_place_in_each_tier() {
        let mut r = residency(1);
        for fetch in r.plan(1, &[4096], 16) {
            let target = r.arrived(fetch).unwrap();
            assert_eq!(
                target.tier_mip,
                fetch.mip - 2,
                "512 is level 0 of the middle tier"
            );
            if target.completes {
                r.commit(0, target.tier);
            }
        }
        for fetch in r.plan(2, &[4096], 16) {
            assert_eq!(r.arrived(fetch).unwrap().tier_mip, fetch.mip);
        }
    }

    #[test]
    fn the_most_wanted_layers_fetch_first_within_the_limit() {
        let mut r = residency(3);
        let fetches = r.plan(1, &[300, 1000, 600], 2);
        assert!(fetches.iter().all(|f| f.layer == 1));
        assert_eq!(fetches.len(), 2);
        assert!(
            r.plan(1, &[300, 1000, 600], 2).is_empty(),
            "the limit holds"
        );
    }

    /// Brings `count` layers to the top tier at `frame`, all wanted at `demand`.
    fn climb(r: &mut TextureResidency, count: usize, demand: u32, frame: u32) -> Vec<u32> {
        let demands = vec![demand; count];
        for _ in 0..4 {
            let fetches = r.plan(frame, &demands, 64);
            fulfil(r, &fetches);
        }
        demands
    }

    #[test]
    fn slots_are_kept_while_wanted_and_pass_on_when_idle() {
        let mut r = residency(3);
        climb(&mut r, 2, 4096, 1);
        assert_eq!(r.counts().1, 2);
        r.take_changes();

        // A third layer wants the same detail: the slots are recent, so it waits on the middle tier.
        let wants_third = [4096, 4096, 4096];
        for frame in 2..6 {
            let fetches = r.plan(frame, &wants_third, 64);
            fulfil(&mut r, &fetches);
        }
        assert_eq!(r.counts().1, 2);

        // Later, layer 0 goes out of view and its slot passes on.
        let far = TOP_PROTECTED_FRAMES + 100;
        for frame in far..far + 4 {
            let fetches = r.plan(frame, &[0, 4096, 4096], 64);
            fulfil(&mut r, &fetches);
        }
        assert_eq!(r.counts().1, 2);
        let changes = r.take_changes();
        assert!(changes.contains(&TierChange {
            layer: 0,
            tier: TIER_MIDDLE,
            slot: 0
        }));
        assert!(changes.contains(&TierChange {
            layer: 2,
            tier: TIER_TOP,
            slot: 0
        }));
    }

    #[test]
    fn a_much_needier_layer_takes_a_slot_after_a_while() {
        let mut r = residency(3);
        climb(&mut r, 2, 600, 1);
        let later = TOP_PROTECTED_FRAMES + 100;
        // Still wanted, at 600 and 4096: only the second is enough to win a slot at 3 x 600.
        for frame in later..later + 4 {
            let fetches = r.plan(frame, &[600, 4096, 4096], 64);
            fulfil(&mut r, &fetches);
        }
        assert_eq!(r.counts().1, 2);
        assert!(r
            .take_changes()
            .iter()
            .any(|c| c.layer == 0 && c.tier == TIER_MIDDLE));
    }

    #[test]
    fn a_failed_level_is_asked_for_again() {
        let mut r = residency(1);
        let fetches = r.plan(1, &[300], 16);
        assert_eq!(fetches.len(), 2);
        r.failed(fetches[0]);
        assert_eq!(r.fetching(), 1);
        assert_eq!(r.plan(2, &[300], 16), vec![fetches[0]]);
    }

    #[test]
    fn a_level_nobody_is_waiting_for_is_ignored() {
        let mut r = residency(1);
        assert_eq!(r.arrived(MipFetch { layer: 0, mip: 0 }), None);
    }
}
