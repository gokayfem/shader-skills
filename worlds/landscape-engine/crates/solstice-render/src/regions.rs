//! Scenes too big for one GPU binding.
//!
//! wgpu limits a storage binding to 2 GiB, which a scene of billions of triangles passes twice:
//! its cluster records, and the page pool it wants. So the cluster table is split in two, and the
//! pool in three by level of detail, close, middle and far, and spare ones that no page calls home, taking what the others can't. A pool of its own also means far
//! pages can't push out near ones that are wanted. A pool that has no room borrows what another
//! has free or isn't using.
//!
//! Only such a scene pays for any of this. The shaders are rewritten for it as text, so a scene
//! that fits compiles the source it always did, with the bindings and limits it always had.
//!
//! For an RTX 5080 on x64, natively or in Chrome: the one setup these scenes are for. Other devices
//! may work, and fail with a clear message if they offer too few storage buffers.
//!
//! A pool address holds its band in the top three bits, above the place within that band's buffer.
//! Adding a cluster's local offset to it needs no special care, and `counts` is left alone.

use solstice_asset::paged::{PagedScene, PAGE_BYTES, ROOT_ROUND};

/// The most pools a scene that needs regions can have: three by level of detail, and spares.
pub const BANDS: usize = 8;
/// A pool address holds its band above this bit: a pool of 2 GiB is 2^29 words.
pub const BAND_SHIFT: u32 = 29;
/// The fewest pools a scene split into regions binds: the near, middle and far bands.
pub const FEWEST_POOLS: usize = 3;

/// Storage buffers per stage the heaviest pass binds with this many pools: the masked rasterizers.
pub fn required_storage_buffers(pools: usize) -> u32 {
    8 + pools as u32
}
/// Set to force regions on any scene, so that tests can exercise this path.
const FORCE_VARIABLE: &str = "SOLSTICE_FORCE_REGIONS";

/// The size of a [`solstice_asset::gpu::PackedCluster`] on the GPU.
const CLUSTER_BYTES: u64 = 48;

/// How a too-large scene is divided between bindings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RegionPlan {
    /// Cluster records in the first binding. The rest go in the second.
    pub cluster_chunk: u32,
    /// Page slots in each band, nearest first.
    pub band_slots: [u32; BANDS],
}

/// Which band a page belongs in, by how much its sets have been simplified.
///
/// The boundaries come from what views of Zorah installed: rounds 1 to 4 and 5 to 8 took about
/// two fifths of a view's pages each, and the far band's coarsest rounds the small remainder.
pub fn band_of_round(round: u32) -> usize {
    match round {
        ROOT_ROUND => 2,
        0..=4 => 0,
        5..=8 => 1,
        _ => 2,
    }
}

/// Pages one binding can hold, under these limits.
pub fn binding_slots(limits: &wgpu::Limits) -> u32 {
    (binding_bytes(limits) / PAGE_BYTES as u64) as u32
}

/// The most one binding may hold: what the device allows, and no more than an address has bits
/// for below its band. A browser passes on whatever limit it is given, and a pool larger than
/// that would have its addresses run into the next band's.
fn binding_bytes(limits: &wgpu::Limits) -> u64 {
    limits
        .max_storage_buffer_binding_size
        .min(limits.max_buffer_size)
        .min(4 << BAND_SHIFT)
}

/// Each page's band, with `cap` the most pages one binding can hold.
///
/// Pages are stored coarsest first, so the far band fills with the pinned pages that every other
/// page depends on. A scene with more of those than one binding holds, as Zorah is by a fifth,
/// puts the rest of the far band's pages in the middle band.
pub fn page_bands(scene: &PagedScene, cap: u32) -> Vec<u8> {
    let mut far = 0;
    scene
        .pages
        .iter()
        .map(|page| {
            let band = scene
                .sets
                .get(page.first_set as usize)
                .map_or(0, |set| band_of_round(set.round) as u8);
            if band == 2 && far == cap {
                return 1;
            }
            far += u32::from(band == 2);
            band
        })
        .collect()
}

/// How many pools these slots need bound: the three by level of detail, and the spares in use.
fn pools_of(band_slots: &[u32; BANDS]) -> usize {
    (3..BANDS)
        .rev()
        .find(|&band| band_slots[band] > 0)
        .map_or(3, |band| band + 1)
}

impl RegionPlan {
    /// How many pools the scene binds.
    pub fn pools(&self) -> usize {
        pools_of(&self.band_slots)
    }

    /// Why a scene with a pool of `slots` can't be drawn under these limits: it needs regions,
    /// and they take more storage buffers per stage than the device offers. [`Self::decide`]
    /// panics there; this says so first, for a message the host can show.
    pub fn unsupported(limits: &wgpu::Limits, scene: &PagedScene, slots: u32) -> Option<String> {
        let limit = binding_bytes(limits);
        let clusters = match scene.external {
            Some(_) => {
                u64::from(slots.min(scene.pages.len() as u32))
                    * u64::from(scene.max_page_clusters().max(1))
            }
            None => scene.clusters.len() as u64,
        };
        let pool_bytes = u64::from(slots) * PAGE_BYTES as u64;
        if clusters * CLUSTER_BYTES <= limit && pool_bytes <= limit {
            return None;
        }
        let needed = required_storage_buffers(pools_of(&band_slots(scene, slots, limit)));
        let offered = limits.max_storage_buffers_per_shader_stage;
        (offered < needed).then(|| {
            format!(
                "this scene is too large for this GPU: it needs {needed} storage buffers per                  stage, and this GPU offers {offered}"
            )
        })
    }

    /// The plan for a scene with a pool of `slots`, or `None` if it fits in single bindings.
    pub fn decide(limits: &wgpu::Limits, scene: &PagedScene, slots: u32) -> Option<Self> {
        let forced = std::env::var_os(FORCE_VARIABLE).is_some();
        let limit = binding_bytes(limits);
        // A scene that keeps its records outside its manifest has them only for the pages in its
        // pool, a block of the most a page can have for each slot.
        let stride = scene
            .external
            .map(|_| u64::from(scene.max_page_clusters().max(1)));
        let cluster_count = |slots: u32| match stride {
            Some(stride) => u64::from(slots.min(scene.pages.len() as u32)) * stride,
            None => scene.clusters.len() as u64,
        };
        let cluster_bytes = cluster_count(slots) * CLUSTER_BYTES;
        let pool_bytes = u64::from(slots) * PAGE_BYTES as u64;
        if !forced && cluster_bytes <= limit && pool_bytes <= limit {
            return None;
        }
        let band_slots = band_slots(scene, slots, limit);
        let needed = required_storage_buffers(pools_of(&band_slots));
        assert!(
            limits.max_storage_buffers_per_shader_stage >= needed,
            "this scene is too large for one storage binding and needs \
             {needed} storage buffers per stage, but this device offers {}",
            limits.max_storage_buffers_per_shader_stage
        );
        let clusters = match stride {
            Some(stride) => u64::from(band_slots.iter().sum::<u32>()) * stride,
            None => scene.clusters.len() as u64,
        };
        let cluster_chunk = if forced {
            // Half each, so that a small scene uses both bindings.
            clusters.div_ceil(2) as u32
        } else {
            // A scene may need regions only for its pool, and have fewer records than this.
            (limit / CLUSTER_BYTES).min(clusters) as u32
        };
        assert!(
            clusters <= u64::from(cluster_chunk) * 2,
            "this scene has {clusters} cluster records to hold, more than two bindings of \
             {cluster_chunk} can"
        );
        Some(Self {
            cluster_chunk,
            band_slots,
        })
    }
}

/// Divides the pool between bands, each within what one binding can hold.
fn band_slots(scene: &PagedScene, slots: u32, limit: u64) -> [u32; BANDS] {
    let cap = (limit / PAGE_BYTES as u64) as u32;
    let mut pages = [0u32; BANDS];
    // Each band's pinned pages must have slots, whichever band they ended up in.
    let mut pinned = [0u32; BANDS];
    for (page, band) in page_bands(scene, cap).into_iter().enumerate() {
        pages[band as usize] += 1;
        pinned[band as usize] += u32::from((page as u32) < scene.pinned_pages);
    }
    if slots >= scene.pages.len() as u32 {
        // Everything is on the GPU, so each band holds just its own.
        assert!(
            pages.iter().all(|&count| count <= cap),
            "a band of this scene's pages can't fit in one binding"
        );
        return std::array::from_fn(|band| if band >= 3 { 0 } else { pages[band].max(1) });
    }
    // A page can't install before the coarser pages it depends on, and the far band's pages are
    // the top of every chain: drawing detail anywhere needs them first. So it comes first, and
    // the rest of the budget is shared.
    let far = pages[2].min((slots / 2).max(pinned[2] + 1)).max(1);
    let rest = slots.saturating_sub(far);
    let near = rest / 2;
    let wanted: [u32; BANDS] =
        std::array::from_fn(|band| [near, rest - near, far].get(band).copied().unwrap_or(0));

    // No band gets more than it has pages for or one binding can hold, and what that leaves over
    // goes to whichever can use it.
    // The spare bands are home to no page: they take what the others can't hold.
    let room: [u32; BANDS] =
        std::array::from_fn(|band| if band >= 3 { cap } else { pages[band].min(cap) });
    let mut given: [u32; BANDS] =
        std::array::from_fn(|band| wanted[band].max(pinned[band]).min(room[band]));
    // Raising a band to its pinned pages can overdraw the budget: the others pay, nearest first.
    let mut over = given.iter().sum::<u32>().saturating_sub(slots);
    for band in 0..BANDS {
        let spare = over.min(given[band] - pinned[band].min(given[band]));
        given[band] -= spare;
        over -= spare;
    }
    let mut left = slots.saturating_sub(given.iter().sum());
    for band in 0..BANDS {
        let more = left.min(room[band] - given[band]);
        given[band] += more;
        left -= more;
    }
    // The spare band may have no slots; a scene's own bands need one to bind.
    std::array::from_fn(|band| {
        if band >= 3 {
            given[band]
        } else {
            given[band].max(1)
        }
    })
}

/// Rewrites `name[...]` as `function(...)`, leaving longer names that end in `name` alone.
fn replace_indexing(source: &str, name: &str, function: &str) -> String {
    let opening = format!("{name}[");
    let mut out = String::with_capacity(source.len());
    let mut rest = source;
    while let Some(found) = rest.find(&opening) {
        let identifier = rest[..found]
            .chars()
            .next_back()
            .is_some_and(|c| c.is_alphanumeric() || c == '_');
        out.push_str(&rest[..found]);
        if identifier {
            out.push_str(&opening);
            rest = &rest[found + opening.len()..];
            continue;
        }
        let inner = &rest[found + opening.len()..];
        let mut depth = 1;
        let end = inner
            .char_indices()
            .find_map(|(at, c)| {
                match c {
                    '[' => depth += 1,
                    ']' => depth -= 1,
                    _ => {}
                }
                (depth == 0).then_some(at)
            })
            .expect("an index has its closing bracket");
        out.push_str(function);
        out.push('(');
        out.push_str(&inner[..end]);
        out.push(')');
        rest = &inner[end + 1..];
    }
    out.push_str(rest);
    out
}

/// The next free binding in group 0.
fn next_binding(source: &str) -> u32 {
    const PREFIX: &str = "@group(0) @binding(";
    source
        .match_indices(PREFIX)
        .filter_map(|(at, _)| {
            let number = &source[at + PREFIX.len()..];
            number[..number.find(')')?].parse::<u32>().ok()
        })
        .max()
        .map_or(0, |highest| highest + 1)
}

/// The name a band's pool has in a rewritten shader.
fn pool_name(band: usize) -> String {
    match band {
        0 => "pool_near".to_owned(),
        1 => "pool_middle".to_owned(),
        2 => "pool_far".to_owned(),
        spare => format!("pool_spare{}", spare - 3),
    }
}

/// Reads a word of whichever pool an address is in.
fn pool_word(pools: usize) -> String {
    let low = (1u32 << BAND_SHIFT) - 1;
    let mut cases = String::new();
    for band in 1..pools {
        cases += &format!(
            "        case {band}u: {{ return {}[word]; }}\n",
            pool_name(band)
        );
    }
    format!(
        "
fn pool_word(address: u32) -> u32 {{
    let word = address & {low}u;
    switch address >> {BAND_SHIFT}u {{
{cases}        default: {{ return pool_near[word]; }}
    }}
}}
"
    )
}

/// Rewrites an expanded shader for a scene that needs regions, with this many pools.
pub fn rewrite_shader(source: &str, cluster_chunk: u32, pools: usize) -> String {
    let mut binding = next_binding(source);
    let mut take = || {
        binding += 1;
        binding - 1
    };
    let mut out = replace_indexing(source, "clusters", "cluster_at");
    let more = take();
    let mut extra = format!(
        "
@group(0) @binding({more}) var<storage, read> clusters_more: array<Cluster>;
const CLUSTER_CHUNK: u32 = {cluster_chunk}u;
fn cluster_at(index: u32) -> Cluster {{
    if index < CLUSTER_CHUNK {{ return clusters[index]; }}
    return clusters_more[index - CLUSTER_CHUNK];
}}
"
    );
    let mut declare = |band: usize, extra: &mut String| {
        *extra += &format!(
            "@group(0) @binding({}) var<storage, read> {}: array<u32>;\n",
            take(),
            pool_name(band)
        );
    };

    const POOL: &str = "var<storage, read> pool: array<u32>;";
    const VERTICES: &str = "var<storage, read> vertices: array<u32>;";
    const TRIANGLES: &str = "var<storage, read> triangles: array<u32>;";
    if out.contains(POOL) {
        // One binding of words: the rasterizers.
        out = replace_indexing(&out, "pool", "pool_word");
        out = out.replace(POOL, "var<storage, read> pool_near: array<u32>;");
        for band in 1..pools {
            declare(band, &mut extra);
        }
        extra += &pool_word(pools);
    } else if out.contains(VERTICES) {
        // A binding each for vertices and triangles, which carry the first two bands here.
        out = replace_indexing(&out, "vertices", "pool_word");
        out = replace_indexing(&out, "triangles", "pool_word");
        out = out.replace(VERTICES, "var<storage, read> pool_near: array<u32>;");
        assert!(out.contains(TRIANGLES), "the triangles binding has changed");
        out = out.replace(TRIANGLES, "var<storage, read> pool_middle: array<u32>;");
        for band in 2..pools {
            declare(band, &mut extra);
        }
        extra += &pool_word(pools);
    }
    out + &extra
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indexing_is_rewritten_through_nested_brackets() {
        assert_eq!(
            replace_indexing(
                "let a = clusters[entry[1].cluster]; let b = visible_clusters[3];",
                "clusters",
                "cluster_at"
            ),
            "let a = cluster_at(entry[1].cluster); let b = visible_clusters[3];"
        );
    }

    #[test]
    fn extra_bindings_come_after_the_last_one_in_group_zero() {
        let source = "@group(0) @binding(3) var a: u32;\n@group(1) @binding(9) var b: u32;\n\
                      @group(0) @binding(5) var c: u32;";
        assert_eq!(next_binding(source), 6);
    }

    #[test]
    fn coarser_rounds_go_in_farther_bands() {
        assert_eq!(band_of_round(1), 0);
        assert_eq!(band_of_round(4), 0);
        assert_eq!(band_of_round(5), 1);
        assert_eq!(band_of_round(8), 1);
        assert_eq!(band_of_round(9), 2);
        assert_eq!(band_of_round(ROOT_ROUND), 2);
    }

    fn scene(rounds: &[u32], pinned: u32) -> PagedScene {
        use solstice_asset::paged::{ClusterSet, PageInfo};
        PagedScene {
            pages: (0..rounds.len() as u32)
                .map(|first_set| PageInfo {
                    first_set,
                    set_count: 1,
                    ..Default::default()
                })
                .collect(),
            sets: rounds
                .iter()
                .map(|&round| ClusterSet {
                    round,
                    ..Default::default()
                })
                .collect(),
            pinned_pages: pinned,
            ..Default::default()
        }
    }

    /// A scene that fits one binding is fine anywhere; one that needs regions is refused, with
    /// a reason, by a device with too few storage buffers, and accepted by one with enough.
    #[test]
    fn a_scene_needing_regions_is_refused_where_they_cannot_be_bound() {
        let few = wgpu::Limits {
            max_storage_buffer_binding_size: 4 * PAGE_BYTES as u64,
            max_buffer_size: 1 << 31,
            max_storage_buffers_per_shader_stage: 8,
            ..wgpu::Limits::defaults()
        };
        let many = wgpu::Limits {
            max_storage_buffers_per_shader_stage: required_storage_buffers(BANDS),
            ..few.clone()
        };
        let rounds = [1, 1, 5, 5, 9, 9];
        let scene = scene(&rounds, 2);
        assert_eq!(RegionPlan::unsupported(&few, &scene, 4), None);
        let reason = RegionPlan::unsupported(&few, &scene, 6).expect("refused");
        assert!(reason.contains("offers 8"), "{reason}");
        assert_eq!(RegionPlan::unsupported(&many, &scene, 6), None);
    }

    /// A pool over the limit doesn't mean a cluster table over it.
    #[test]
    fn a_large_pool_with_few_clusters_leaves_the_second_binding_empty() {
        use solstice_asset::gpu::PackedCluster;
        if std::env::var_os(FORCE_VARIABLE).is_some() {
            return;
        }
        let mut scene = scene(&[ROOT_ROUND, 1, 5, 9], 1);
        scene.clusters = vec![PackedCluster::default(); 10];
        let limits = wgpu::Limits {
            max_storage_buffer_binding_size: 1 << 31,
            max_buffer_size: 1 << 31,
            max_storage_buffers_per_shader_stage: required_storage_buffers(BANDS),
            ..Default::default()
        };
        // 40,000 slots of 128 KiB is 5 GiB, over the limit, but only ten clusters.
        let plan = RegionPlan::decide(&limits, &scene, 40_000).expect("the pool needs regions");
        assert_eq!(plan.cluster_chunk, 10);
    }

    #[test]
    fn a_scene_within_every_limit_has_no_regions() {
        if std::env::var_os(FORCE_VARIABLE).is_some() {
            return;
        }
        let limits = wgpu::Limits {
            max_storage_buffer_binding_size: 1 << 31,
            max_buffer_size: 1 << 31,
            ..Default::default()
        };
        assert_eq!(RegionPlan::decide(&limits, &scene(&[1], 0), 100), None);
    }

    /// More pinned pages than one binding holds, as in the whole of Zorah.
    #[test]
    fn what_the_far_band_cannot_hold_goes_in_the_middle_band() {
        let mut rounds = vec![ROOT_ROUND; 7];
        rounds.extend([9, 9, 5, 1, 1]);
        let scene = scene(&rounds, 7);
        assert_eq!(
            page_bands(&scene, 5),
            [2, 2, 2, 2, 2, 1, 1, 1, 1, 1, 0, 0],
            "pinned pages first, and all that would have been far after them"
        );
        let cap = 5 * PAGE_BYTES as u64;
        let slots = band_slots(&scene, 11, cap);
        assert_eq!(slots[2], 5);
        assert!(
            slots[1] >= 3,
            "the middle band can hold its pinned pages: {slots:?}"
        );
    }

    /// The smallest pool the renderer makes holds just the pinned pages, wherever they are.
    #[test]
    fn every_band_holds_its_pinned_pages_at_the_smallest_budget() {
        let mut rounds = vec![ROOT_ROUND; 7];
        rounds.extend([9, 9, 5, 1, 1]);
        let scene = scene(&rounds, 7);
        let cap = 5 * PAGE_BYTES as u64;
        let slots = band_slots(&scene, 7, cap);
        assert_eq!(slots[2], 5, "{slots:?}");
        assert_eq!(slots[1], 2, "{slots:?}");
        let bands = page_bands(&scene, 5);
        let deps = vec![Vec::new(); bands.len()];
        // Panics if a band can't hold its pinned pages.
        crate::streaming::residency::Residency::with_bands(deps, 7, bands, &slots);
    }

    #[test]
    fn a_scene_whose_far_pages_fit_is_divided_as_before() {
        let scene = scene(&[ROOT_ROUND, ROOT_ROUND, 9, 5, 1], 2);
        assert_eq!(page_bands(&scene, 16_383), [2, 2, 2, 1, 0]);
    }

    #[test]
    fn a_resident_scene_gets_exactly_the_slots_it_needs() {
        let scene = scene(&[ROOT_ROUND, ROOT_ROUND, 9, 5, 4, 1, 1, 1], 2);
        assert_eq!(band_slots(&scene, 8, 1 << 31), [4, 1, 3, 0, 0, 0, 0, 0]);
    }

    #[test]
    fn a_streaming_scene_splits_its_budget_within_the_binding_limit() {
        let rounds: Vec<u32> = (0..200_000).map(|i| 1 + i % 12).collect();
        let scene = scene(&rounds, 100);
        let page = PAGE_BYTES as u64;
        // 40 GiB wanted, 2 GiB a binding.
        let slots = band_slots(&scene, 160_000, 2 * (1 << 30));
        assert!(
            slots.iter().all(|&s| u64::from(s) * page <= 2 << 30),
            "{slots:?}"
        );
        assert!(slots[2] >= 100, "the far band holds the pinned pages");
    }

    /// The far band's few coarse pages must not take room that other bands could use.
    #[test]
    fn slots_a_band_cannot_use_go_to_the_others() {
        // Mostly near pages, few far ones: the far band's share would be wasted.
        let mut rounds = vec![ROOT_ROUND; 4];
        rounds.extend(std::iter::repeat_n(1, 1000));
        let slots = band_slots(&scene(&rounds, 4), 400, 1 << 31);
        assert_eq!(slots[2], 4, "the far band only has its pinned pages");
        assert_eq!(slots[0], 396);
    }

    /// Whatever a page depends on sits in a band with room beyond its pinned pages.
    #[test]
    fn the_far_band_has_room_beyond_its_pinned_pages() {
        let mut rounds = vec![ROOT_ROUND; 4];
        rounds.extend((0..40).map(|i| 1 + i % 9));
        let slots = band_slots(&scene(&rounds, 4), 14, 1 << 31);
        assert!(slots[2] > 4, "{slots:?}");
    }
}
