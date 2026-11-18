// Reads a cluster's vertices and triangles out of the page pool. Both are found by the word: a
// cluster's `vertex_offset` and `triangle_offset` are words of the pool. A shader that includes
// this defines `vertex_word(word: u32) -> u32` and `triangle_word(word: u32) -> u32`, which read
// the pool through whichever of its bindings holds each. Twin of solstice-asset's `gpu` module.

// A vertex is three words.
fn load_vertex(cluster: Cluster, local: u32) -> PackedVertex {
    let word = cluster.vertex_offset + local * 3u;
    return PackedVertex(vertex_word(word), vertex_word(word + 1u), vertex_word(word + 2u));
}

// A skinned cluster's vertices are followed by their skins, two words each.
fn vertex_skin(cluster: Cluster, local: u32) -> vec2u {
    let word = cluster.vertex_offset + cluster_vertex_count(cluster) * 3u + local * 2u;
    return vec2u(vertex_word(word), vertex_word(word + 1u));
}

// A triangle is three bytes, the places of its corners among its cluster's vertices, and a
// cluster's triangles lie end to end from the word they begin at. One that starts in the third
// or fourth byte of a word ends in the next word.
fn load_triangle(cluster: Cluster, index: u32) -> u32 {
    let bit = index * 24u;
    let word = cluster.triangle_offset + (bit >> 5u);
    let shift = bit & 31u;
    var packed = triangle_word(word) >> shift;
    if shift > 8u {
        packed |= triangle_word(word + 1u) << (32u - shift);
    }
    return packed & 0xffffffu;
}
