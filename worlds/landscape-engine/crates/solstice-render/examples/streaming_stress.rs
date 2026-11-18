//! Deterministic streaming soak: changing views, delayed/failed requests, held views and
//! small pools. Every install and eviction checks dependency and slot invariants.
use solstice_render::streaming::residency::{Residency, Step};

fn random(state: &mut u64) -> u64 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    *state
}

fn hash(state: &mut u64, value: u32) {
    *state = (*state ^ u64::from(value)).wrapping_mul(0x100000001b3);
}

fn main() {
    const COUNT: usize = 16_384;
    const PINNED: u32 = 8;
    const FETCHES: u32 = 64;
    let dependencies: Vec<Vec<u32>> = (0..COUNT as u32)
        .map(|page| {
            if page < PINNED {
                vec![]
            } else {
                vec![page / 8]
            }
        })
        .collect();
    println!("bands,frames,fetches,installs,evictions,trace_hash");
    for slots in [vec![256], vec![64, 64, 128]] {
        let bands = if slots.len() == 1 {
            vec![0; COUNT]
        } else {
            (0..COUNT)
                .map(|page| {
                    if page < 64 {
                        2
                    } else if page < 512 {
                        1
                    } else {
                        0
                    }
                })
                .collect()
        };
        let mut residency = Residency::with_bands(dependencies.clone(), PINNED, bands, &slots);
        let mut placed = vec![None; COUNT];
        for page in 0..PINNED {
            placed[page as usize] = Some(residency.install_now(page));
        }
        let mut random_state = 0x2545_f491_4f6c_dd1d;
        let mut checksum = 0xcbf29ce484222325;
        let mut flying = Vec::new();
        let (mut fetched, mut installs, mut evictions) = (0, 0, 0);
        for frame in 1..=1_500 {
            let mut requests = vec![0; COUNT];
            let mut held = vec![0u32; COUNT / 32];
            let focus = (random(&mut random_state) % COUNT as u64) as usize;
            for n in 0..96 {
                let page = (focus + (random(&mut random_state) % 512) as usize) % COUNT;
                requests[page] = if n % 7 == 0 { 1 } else { 2 + n * 7 };
                if frame % 19 == 0 {
                    held[page / 32] |= 1 << (page % 32);
                }
            }
            let started = residency.plan_holding(frame, &requests, &[&held], FETCHES);
            fetched += started.len();
            for &page in &started {
                hash(&mut checksum, page);
            }
            flying.extend(started);
            // Complete in arbitrary order, with failures and enough delay to leave requests
            // from an old camera view in flight while the next view is requested.
            for _ in 0..48 {
                if flying.is_empty() {
                    break;
                }
                let at = random(&mut random_state) as usize % flying.len();
                let page = flying.swap_remove(at);
                if random(&mut random_state).is_multiple_of(17) {
                    residency.failed(page);
                } else {
                    assert!(residency.arrived(page));
                }
            }
            let started = residency.refill(FETCHES);
            fetched += started.len();
            for &page in &started {
                hash(&mut checksum, page);
            }
            flying.extend(started);
            for step in residency.install_ready(frame, 32) {
                match step {
                    Step::Evict(page) => {
                        assert!(page >= PINNED);
                        assert!(placed[page as usize].take().is_some());
                        assert!(!dependencies
                            .iter()
                            .enumerate()
                            .any(|(p, deps)| placed[p].is_some() && deps.contains(&page)));
                        hash(&mut checksum, page | (1 << 31));
                        evictions += 1;
                    }
                    Step::Install { page, slot } => {
                        assert!(dependencies[page as usize]
                            .iter()
                            .all(|&d| placed[d as usize].is_some()));
                        assert!(!placed.contains(&Some(slot)));
                        placed[page as usize] = Some(slot);
                        hash(&mut checksum, page);
                        hash(&mut checksum, slot);
                        installs += 1;
                    }
                }
            }
            assert!(flying.len() <= FETCHES as usize);
            assert!(residency.installed_count() <= slots.iter().sum());
            hash(&mut checksum, residency.starved());
            hash(&mut checksum, u32::from(residency.take_awaited_arrival()));
        }
        println!(
            "{},1500,{fetched},{installs},{evictions},{checksum}",
            slots.len()
        );
    }
}
