//! Measures how a baked scene's pages use their room: `page_stats FILE [SAMPLE_EVERY]`.
//!
//! It reads the manifest for the sizes of every page, and unpacks every Nth page to see what
//! its clusters hold. `docs/streaming.md` has what this found for one large scene, under
//! "Tighter pages".

use solstice_asset::file::{
    decode_header, decode_manifest, inflate, page_range, unpack_page, HEADER_BYTES,
};
use solstice_asset::gpu::{triangle_at, triangle_words, VERTEX_WORDS};
use solstice_asset::paged::PAGE_BYTES;
use solstice_asset::PackedVertex;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};

const MIB: f64 = 1_048_576.0;

fn read_at(file: &mut File, start: u64, len: usize) -> Vec<u8> {
    let mut bytes = vec![0; len];
    file.seek(SeekFrom::Start(start)).expect("seek");
    file.read_exact(&mut bytes).expect("read");
    bytes
}

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("page_stats FILE [SAMPLE_EVERY]");
    let every: usize = args.next().map_or(16, |n| n.parse().expect("a number"));
    let mut file = File::open(&path).expect("open");
    let header = decode_header(&read_at(&mut file, 0, HEADER_BYTES)).expect("header");
    let manifest = read_at(
        &mut file,
        header.manifest_offset,
        header.manifest_bytes as usize,
    );
    let manifest = inflate(&manifest).expect("inflate");
    let scene = decode_manifest(&manifest, header.manifest_offset + header.manifest_bytes)
        .expect("manifest");

    let pages = scene.pages.len();
    let pinned = scene.pinned_pages as usize;
    let streamed = &scene.pages[pinned..];
    let raw: u64 = scene.pages.iter().map(|p| p.raw_bytes() as u64).sum();
    let stored: u64 = scene.pages.iter().map(|p| u64::from(p.stored_bytes)).sum();
    let vertex_bytes: u64 = scene.pages.iter().map(|p| p.vertex_bytes() as u64).sum();
    let streamed_raw: u64 = streamed.iter().map(|p| p.raw_bytes() as u64).sum();
    let mut fill = [0u32; 10];
    for page in streamed {
        fill[(page.raw_bytes() * 10 / (PAGE_BYTES + 1)).min(9)] += 1;
    }
    let clusters = scene.cluster_count();
    println!("clusters            {clusters}");
    println!("sets                {}", scene.sets.len());
    println!("pages               {pages} ({pinned} pinned)");
    println!("page bytes, raw     {:.1} MiB", raw as f64 / MIB);
    println!("page bytes, stored  {:.1} MiB", stored as f64 / MIB);
    println!(
        "vertices            {:.1} MiB ({:.0}%)",
        vertex_bytes as f64 / MIB,
        vertex_bytes as f64 * 100.0 / raw as f64
    );
    println!(
        "triangles           {:.1} MiB",
        (raw - vertex_bytes) as f64 / MIB
    );
    println!(
        "bytes a cluster     {:.0} raw, {:.0} of pool with slots as full as they are",
        raw as f64 / clusters as f64,
        (pages * PAGE_BYTES) as f64 / clusters as f64
    );
    println!(
        "slots full to       {:.1}% of {} KiB",
        streamed_raw as f64 * 100.0 / (streamed.len() * PAGE_BYTES) as f64,
        PAGE_BYTES / 1024
    );
    println!("pages by tenths full {fill:?}");
    println!(
        "clusters a page     {:.1}",
        (pinned..pages)
            .map(|p| u64::from(scene.page_cluster_count(p as u32)))
            .sum::<u64>() as f64
            / streamed.len() as f64
    );

    // Sampled pages: every cluster is read back, which checks the page against its tables.
    let (mut sampled, mut vertices, mut triangles) = (0u64, 0u64, 0u64);
    for page in (pinned..pages).step_by(every) {
        let (start, end) = page_range(&scene, page as u32);
        let packed = read_at(&mut file, start, (end - start) as usize);
        let data = unpack_page(&scene, page as u32, &packed).expect("page");
        let info = scene.pages[page];
        let (vertex_part, triangle_part) = data.split_at(info.vertex_bytes());
        let records: &[PackedVertex] = bytemuck::cast_slice(vertex_part);
        let words: Vec<u32> = triangle_part
            .chunks_exact(4)
            .map(|w| u32::from_le_bytes([w[0], w[1], w[2], w[3]]))
            .collect();
        for set in &scene.sets[info.first_set as usize..(info.first_set + info.set_count) as usize]
        {
            for c in set.first_cluster..set.first_cluster + set.cluster_count {
                let cluster = scene.clusters[c as usize];
                let count = cluster.vertex_count();
                let first = cluster.vertex_offset / VERTEX_WORDS;
                assert!(
                    (first + count) as usize <= records.len(),
                    "vertices out of the page"
                );
                let triangle_count = cluster.triangle_count();
                let own = &words[cluster.triangle_offset as usize..]
                    [..triangle_words(triangle_count) as usize];
                for index in 0..triangle_count {
                    let packed = triangle_at(own, index);
                    assert!(
                        [packed & 0xff, (packed >> 8) & 0xff, packed >> 16]
                            .iter()
                            .all(|&corner| corner < count),
                        "a triangle names a vertex its cluster doesn't have"
                    );
                }
                sampled += 1;
                vertices += u64::from(count);
                triangles += u64::from(triangle_count);
            }
        }
    }
    println!("--- every {every}th streamed page, each cluster read back ---");
    println!("clusters            {sampled}");
    println!(
        "vertices a cluster  {:.1}",
        vertices as f64 / sampled as f64
    );
    println!(
        "triangles a cluster {:.1}",
        triangles as f64 / sampled as f64
    );
}
