//! pattern: tilings and antialiased procedural patterns. The filtered versions integrate the pattern over the
//! pixel footprint (box filter), so floors and stripes stay clean in the distance with no supersampling.
//! Filtering math follows https://iquilezles.org/articles/checkerfiltering/ and /filterableprocedurals/.

/// Triangle wave, the integral of a square wave. Building block for box-filtered patterns.
/// @use writing new filtered patterns
vec2 triangleWave(vec2 x) { return 1.0 - 2.0 * abs(fract(x * 0.5) - 0.5); }

/// Box-filtered checkerboard with unit squares: 0 or 1 up close, fading to 0.5 where squares are smaller than a pixel.
/// Compute the footprint before any branching: w = fwidth(p) (or max(abs(dFdx(p)), abs(dFdy(p)))).
/// @use floors, chessboards, test patterns, racing flags, tablecloths
/// @range [0,1]
float filteredChecker(vec2 p, vec2 w) {
    w = max(w, 1e-4);
    vec2 i = (triangleWave(p + 0.5 * w) - triangleWave(p - 0.5 * w)) / w;
    return 0.5 - 0.5 * i.x * i.y;
}

/// Box-filtered grid lines (unit cells, line width = 1/n of a cell). Returns 1 on lines, 0 inside cells.
/// @use blueprints, graph paper, tiles with grout, sci-fi floors, synthwave grids
/// @range [0,1]
float filteredGrid(vec2 p, vec2 w, float n) {
    w = max(w, 1e-4);
    vec2 a = p + 0.5 * w;
    vec2 b = p - 0.5 * w;
    vec2 i = (floor(a) + min(fract(a) * n, 1.0) - floor(b) - min(fract(b) * n, 1.0)) / (n * w);
    return 1.0 - (1.0 - i.x) * (1.0 - i.y);
}

/// Box-filtered stripes along x with period 1 and duty cycle 0.5. Returns 0..1.
/// @use awnings, candy, zebra (warp x first), barcodes, road markings
/// @range [0,1]
float filteredStripes(float x, float w) {
    w = max(w, 1e-4);
    float a = x + 0.5 * w, b = x - 0.5 * w;
    float ia = floor(a) * 0.5 + max(fract(a) - 0.5, 0.0);
    float ib = floor(b) * 0.5 + max(fract(b) - 0.5, 0.0);
    return (ia - ib) / w;
}

/// Hexagonal tiling with unit spacing between neighboring centers. Neighbors sit left/right, so hexes are
/// pointy-top. Returns vec4(local offset from the hex center, integer-valued hex id).
/// @use honeycomb, scales, tiles, game boards, hex pixelation
vec4 hexGrid(vec2 p) {
    const vec2 s = vec2(1.0, 1.7320508);
    vec2 a = mod(p, s) - 0.5 * s;
    vec2 b = mod(p - 0.5 * s, s) - 0.5 * s;
    vec2 local = (dot(a, a) < dot(b, b)) ? a : b;
    vec2 center = p - local;
    vec2 id = floor(vec2(center.x * 2.0, center.y * 1.1547005) + 0.5);
    return vec4(local, id);
}

/// Distance from a hexGrid local coordinate to its hexagon's edge (0.5 at the center, 0 on the edge).
/// @use hex borders, bevels, edge wear on tiles
float hexEdgeDistance(vec2 local) {
    vec2 a = abs(local);
    return 0.5 - max(a.x, dot(a, vec2(0.5, 0.8660254)));
}

/// Truchet tiling of quarter-circle arcs, randomly flipped per cell. Returns signed distance to arcs of half-width w.
/// @use maze-like ornaments, pipes, Celtic-style weaves, procedural circuitry
float truchetArcs(vec2 p, float w) {
    vec2 id = floor(p);
    vec2 f = fract(p) - 0.5;
    if (hash12(id) < 0.5) f.x = -f.x;
    vec2 q = f - 0.5 * sign(f.x + f.y + 1e-6);
    return abs(length(q) - 0.5) - w;
}

/// Equilateral triangle containing p (unit edge). Each column holds integer lattice ID in xy, barycentric weight in z.
/// @use stable per-vertex seeds for stochastic tiling of rock, leather, plaster or brushed metal
/// @see Mikkelsen, Practical Real-Time Hex-Tiling (2022)
mat3 triangleTileWeights(vec2 p) {
    float row = p.y * (2.0 / sqrt(3.0));
    vec2 lattice = vec2(p.x - 0.5 * row, row);
    vec2 base = floor(lattice);
    vec2 f = fract(lattice);
    vec2 tip = vec2(0.0);
    vec3 weights = vec3(1.0 - f.x - f.y, f.x, f.y);
    if (f.x + f.y > 1.0) {
        tip = vec2(1.0);
        weights = vec3(f.x + f.y - 1.0, 1.0 - f.y, 1.0 - f.x);
    }
    return mat3(vec3(base + tip, weights.x),
                vec3(base + vec2(1.0, 0.0), weights.y),
                vec3(base + vec2(0.0, 1.0), weights.z));
}
