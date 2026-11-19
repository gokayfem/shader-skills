// A stamped brass panel. The repeat supplies fine breakup and relief; a separate
// object-local mask allows patina to collect inside engraved paths.
// @render time=1.2
// @variant DEBUG_MODE=2
// @variant DEBUG_MODE=5
#define DEBUG_MODE 0
const float SHUFFLE = 1.0;

// Periodic brushed grain. Integer frequencies make both axes wrap.
float rolledGrain(vec2 uv) {
    vec2 phase = TAU * uv;
    return 0.5 + 0.22 * sin(phase.y + 2.6 * sin(phase.x))
               + 0.18 * sin(3.0 * phase.y + 1.8 * cos(phase.x))
               + 0.10 * cos(phase.x + 5.0 * phase.y);
}

float sheetGrain(vec2 p) {
    mat3 tiles = triangleTileWeights(p);
    float grain = 0.0;
    float total = 0.0;
    for (int i = 0; i < 3; i++) {
        vec3 vertex = tiles[i];
        vec3 seed = hash32(vertex.xy);
        float weight = vertex.z * vertex.z;
        vec2 uv = rot2(TAU * seed.x) * p + seed.yz;
        grain += weight * rolledGrain(uv);
        total += weight;
    }
    return mix(rolledGrain(p), grain / total, SHUFFLE);
}

void mainImage(out vec4 fragColor, in vec2 fragCoord) {
    vec2 p = (2.0 * fragCoord - iResolution.xy) / iResolution.y;
    vec2 cell = fract(p * 1.35 + 0.5) - 0.5;
    float r = length(cell);
    float ring = min(abs(r - 0.33), abs(r - 0.385));
    float diamond = abs(abs(cell.x) + abs(cell.y) - 0.42) * 0.7071;
    float groove = min(ring, diamond);
    float width = max(fwidth(groove), 0.0001);
    float recess = 1.0 - smoothstep(0.004, 0.019 + width, groove);
    float grain = sheetGrain(p * 4.3);
    float deposit = smoothstep(0.20, 0.72, grain);
    float patina = recess * deposit;
    float height = 0.01 * grain - 0.022 * recess;
    float pixel = 2.0 / iResolution.y;
    vec3 normal = normalize(vec3(-dFdx(height), -dFdy(height), pixel));
    vec3 light = normalize(vec3(cos(iTime * 0.45), sin(iTime * 0.45), 1.1));
    vec3 brass = mix(vec3(0.29, 0.13, 0.035), vec3(0.58, 0.34, 0.11), grain);
    vec3 base = mix(brass, vec3(0.025, 0.17, 0.13), patina);
    float spec = pow(max(dot(normal, normalize(light + vec3(0.0, 0.0, 1.0))), 0.0), 48.0);
    vec3 lit = base * (0.28 + 0.72 * max(dot(normal, light), 0.0))
             + brass * spec * (1.0 - patina);
#if DEBUG_MODE == 2
    fragColor = vec4(0.5 + 0.5 * normal, 1.0);
#elif DEBUG_MODE == 5
    fragColor = vec4(deposit, recess, patina, 1.0);
#else
    fragColor = vec4(linearToSrgb(lit), 1.0);
#endif
}
