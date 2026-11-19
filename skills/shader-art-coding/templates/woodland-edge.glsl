// A plan view of open meadow, oak groves and alder along a stream.
// @render time=1.0
// @variant DEBUG_MODE=3
// @variant DEBUG_MODE=5
#define DEBUG_MODE 0
const float CLEARING = 0.7;
const float BANK_WIDTH = 1.5;

// Lateral distance from the stream centre, in world units.
float bankDistance(vec2 p) {
    float centre = 1.1 * sin(0.52 * p.y) + 0.32 * sin(1.47 * p.y + 0.5);
    return abs(p.x - centre);
}

float dampGround(vec2 p) {
    return 1.0 - smoothstep(0.32, 0.32 + BANK_WIDTH, bankDistance(p));
}

// Accumulate nearby crowns at the shading point and at the ground plant's root.
// Each crown stays within 1.6 units of its 2-unit founder cell.
void stand(vec2 p, vec2 root, float px, out float rootCover, out float shade,
           out vec3 crownColor, out float crownAlpha) {
    vec2 cell = floor(p * 0.5);
    rootCover = 0.0;
    shade = 0.0;
    crownColor = vec3(0.0);
    crownAlpha = 0.0;
    float top = -1.0;
    for (int y = -1; y <= 1; y++) {
        for (int x = -1; x <= 1; x++) {
            vec2 id = cell + vec2(float(x), float(y));
            vec3 seed = hash32(id);
            vec2 founder = 2.0 * (id + 0.2 + 0.6 * seed.xy);
            float wet = dampGround(founder);
            float alder = step(seed.z, wet);
            float suitability = max(wet, smoothstep(0.25, 0.75, valueNoise(founder * 0.26)));
            if (hash12(id + 91.0) > suitability) continue;
            for (int j = 0; j < 3; j++) {
                vec3 r = hash33(vec3(id, float(j)));
                vec2 centre = founder;
                float radius = mix(0.44, 0.76, r.z);
                if (j > 0) {
                    float angle = TAU * r.x;
                    centre += mix(0.40, 0.94, r.y) * vec2(cos(angle), sin(angle));
                    radius *= 0.64;
                }
                if (bankDistance(centre) < 0.34 || distance(centre, vec2(-2.5, 0.0)) < CLEARING) continue;
                vec2 q = (p - centre) / radius;
                float d = length(q);
                rootCover = max(rootCover, 1.0 - smoothstep(0.70, 1.42, distance(root, centre) / radius));
                shade = max(shade, 1.0 - smoothstep(0.77, 1.12, length(q - vec2(0.27, -0.24))));
                if (d > 1.18) continue;
                float lobes = valueNoise(3.5 * q + 16.0 * r.xy);
                float edge = d - (0.82 + 0.32 * lobes);
                float alpha = 1.0 - smoothstep(-px / radius, px / radius, edge);
                crownAlpha = max(crownAlpha, alpha);
                float height = radius * (1.25 - d * d + 0.24 * lobes);
                if (alpha > 0.0 && height > top) {
                    top = height;
                    float twigs = valueNoise(q * 8.0 + 24.0 * r.yz);
                    vec3 leaves = mix(vec3(0.16, 0.25, 0.061), vec3(0.072, 0.22, 0.13), alder);
                    float light = clamp(0.68 + dot(q, vec2(-0.34, 0.36)), 0.23, 1.0);
                    crownColor = leaves * light * (0.40 + lobes + 0.42 * twigs) * (0.84 + 0.3 * r.z);
                }
            }
        }
    }
}

void mainImage(out vec4 fragColor, in vec2 fragCoord) {
    float px = 8.0 / iResolution.y;
    vec2 p = (fragCoord - 0.5 * iResolution.xy) * px;
    vec2 id = floor(p * 3.0);
    vec3 seed = hash32(id);
    vec2 root = (id + 0.28 + 0.44 * seed.xy) / 3.0;
    float cover, shade, alpha;
    vec3 crown;
    stand(p, root, px, cover, shade, crown, alpha);

    float wet = dampGround(root);
    float edge = 4.0 * cover * (1.0 - cover);
    float fern = cover * wet;
    float shrub = edge * (1.0 - 0.5 * wet);
    float flowers = (1.0 - cover) * (1.0 - wet);
    float grain = valueNoise(p * 3.0) + 0.45 * valueNoise(p * 13.0);
    vec3 ground = mix(vec3(0.34, 0.32, 0.115), vec3(0.13, 0.21, 0.086), dampGround(p));
    ground = mix(ground, vec3(0.13, 0.085, 0.042), shade * 0.85) * (0.67 + 0.46 * grain);

    vec2 blade = rot2(seed.y * TAU) * (p - root);
    float fernShape = min(length(blade * vec2(0.7, 2.0)), length(blade * vec2(2.0, 0.7)));
    vec3 understory = vec3(0.0);
    float plant = 0.0;
    if (bankDistance(root) > 0.34) {
        if (seed.z < fern) {
            plant = 1.0 - smoothstep(0.040 - px, 0.040 + px, fernShape);
            understory = vec3(0.18, 0.36, 0.09);
        } else if (seed.z < fern + shrub) {
            plant = 1.0 - smoothstep(0.072 - px, 0.072 + px, length(blade));
            understory = vec3(0.085, 0.17, 0.047);
        } else if (seed.z > 1.0 - 0.36 * flowers) {
            plant = 1.0 - smoothstep(0.028 - px, 0.028 + px, length(blade));
            understory = vec3(0.82, 0.66, 0.26);
        }
    }
    float water = 1.0 - smoothstep(0.23, 0.31, bankDistance(p));
    vec3 stream = vec3(0.043, 0.19, 0.20) * (0.8 + 0.3 * valueNoise(vec2(p.x * 19.0, p.y * 3.0 - iTime)));
    vec3 col = mix(mix(ground, understory, plant), stream, water);
    col = mix(col * (1.0 - 0.35 * shade), crown, alpha);
#if DEBUG_MODE == 3
    col = vec3(fern, shrub, flowers) * (1.0 - water);
#elif DEBUG_MODE == 5
    col = vec3(cover, wet, alpha);
#endif
    fragColor = vec4(linearToSrgb(col), 1.0);
}
