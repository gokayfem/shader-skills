// Procedural material swatches: six texture-free patterns on one board.
// Shows: two-level domain warping (marble), exact Voronoi borders (cracked earth), hex tiles with bevels,
// Truchet weaves, smooth Voronoi (organic cells) and a box-filtered checker. Each tile uses explicit pixel-size
// filtering instead of fwidth, so it stays valid inside branches.
// @render time=2.0

#define DEBUG_MODE 0   // 0 final | 3 tile ids

vec3 marble(vec2 p) {
    vec2 q, r;
    float f = warpedFbm(p * 1.5, 3.0, 5, q, r);
    vec3 col = mixOklab(srgbToLinear(vec3(0.92, 0.90, 0.86)), srgbToLinear(vec3(0.35, 0.38, 0.45)), saturate(length(q) * 0.9));
    col = mix(col, srgbToLinear(vec3(0.62, 0.42, 0.30)), saturate(r.y * r.y * 2.0));
    return col * (0.85 + 0.25 * f);
}

vec3 crackedEarth(vec2 p, float px) {
    vec3 v = voronoiBorder(p * 4.0, 0.9);
    vec2 cell = floor(p * 4.0 + v.yz);
    float tint = hash12(cell);
    vec3 col = mix(srgbToLinear(vec3(0.62, 0.44, 0.30)), srgbToLinear(vec3(0.75, 0.58, 0.40)), tint);
    col *= 0.75 + 0.25 * smoothstep(0.0, 0.25, v.x);          // plates darken toward their curled edges
    float plate = fillAA(0.03 - v.x, px * 4.0);                 // cracks: border distance < 0.03 cell units
    return mix(srgbToLinear(vec3(0.12, 0.08, 0.06)), col, plate);
}

vec3 hexTiles(vec2 p, float px) {
    vec4 h = hexGrid(p * 5.0);
    float edge = hexEdgeDistance(h.xy);
    vec3 base = oklchToLinearSrgb(vec3(0.7, 0.09, 3.6 + 0.8 * hash12(h.zw)));
    float bevel = smoothstep(0.0, 0.12, edge);
    return mix(vec3(0.02), base * (0.7 + 0.3 * bevel), smoothstep(0.03 - px * 5.0, 0.03 + px * 5.0, edge));
}

vec3 truchet(vec2 p, float px) {
    float d = truchetArcs(p * 5.0, 0.12) / 5.0;
    float stripe = truchetArcs(p * 5.0, 0.03) / 5.0;
    vec3 col = srgbToLinear(vec3(0.08, 0.10, 0.16));
    col = mix(col, srgbToLinear(vec3(0.95, 0.75, 0.30)), fillAA(d, px));
    return mix(col, srgbToLinear(vec3(0.55, 0.20, 0.10)), fillAA(stripe, px));
}

vec3 organicCells(vec2 p) {
    float s = smoothVoronoi(p * 5.0, 12.0);
    vec3 col = mixOklab(srgbToLinear(vec3(0.95, 0.55, 0.55)), srgbToLinear(vec3(0.35, 0.05, 0.15)), saturate(s * 1.6));
    return col * (0.6 + 0.4 * smoothstep(0.1, 0.5, s));
}

vec3 checker(vec2 p, float px) {
    // perspective floor: the filtered checker stays grey instead of moire in the distance
    float z = 1.0 / max(0.55 - p.y, 0.02);
    vec2 floorUV = vec2(p.x * z, z) * 1.5;
    float c = filteredChecker(floorUV, vec2(px * z * z * 1.5 + px * z * 1.5));
    return mix(vec3(0.05), vec3(0.8), c) * (1.0 - smoothstep(0.3, 0.55, p.y));
}

void mainImage(out vec4 fragColor, in vec2 fragCoord) {
    vec2 uv = fragCoord / iResolution.y;
    float px = 1.0 / iResolution.y;
    float aspect = iResolution.x / iResolution.y;
    vec2 grid = vec2(3.0, 2.0);
    vec2 tileSize = vec2(aspect, 1.0) / grid;
    vec2 id = floor(uv / tileSize);
    vec2 local = (uv - id * tileSize) / tileSize.y;        // 0..(tile aspect) x 0..1
    vec2 p = local - 0.5 * vec2(tileSize.x / tileSize.y, 1.0);
    float k = id.x + 3.0 * (1.0 - id.y);
    float lpx = px / tileSize.y;

    vec3 col;
    if (k < 0.5) col = marble(p + 0.05 * iTime);
    else if (k < 1.5) col = crackedEarth(p, lpx);
    else if (k < 2.5) col = hexTiles(p, lpx);
    else if (k < 3.5) col = truchet(p, lpx);
    else if (k < 4.5) col = organicCells(p + vec2(0.0, 0.03 * iTime));
    else col = checker(p, lpx);

    // tile frame
    vec2 e = min(local, vec2(tileSize.x / tileSize.y, 1.0) - local);
    col *= smoothstep(0.0, 0.02, min(e.x, e.y));

#if DEBUG_MODE == 3
    col = debugIdColor(k);
#endif
    fragColor = vec4(linearToSrgb(col) + ditherTPDF(fragCoord, iFrame), 1.0);
}
