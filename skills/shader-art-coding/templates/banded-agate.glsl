// Backlit agate cross-section: iron-oxide rind, chalcedony growth bands and a faceted quartz pocket.
// @render time=2.5
// @variant DEBUG_MODE=3
#define DEBUG_MODE 0   // 0 final | 3 quartz, chalcedony and rind masks

const float BAND_COUNT = 27.0;
const float BACKLIGHT = 1.25;
const float DRIFT = 0.035;

void mainImage(out vec4 fragColor, in vec2 fragCoord) {
    vec2 uv = (2.0 * fragCoord - iResolution.xy) / iResolution.y;
    vec2 p = rot2(-0.32) * (uv - vec2(-0.22, 0.02));
    p *= vec2(0.82, 1.32);
    float px = 2.64 / iResolution.y;
    float warp = fbmLod(p * 2.1 + vec2(3.2, 0.8), 6, px * 2.1);
    float field = length(p) + 0.23 * warp + 0.07 * sin(4.0 * p.x - p.y);
    float phase = field * BAND_COUNT + 0.36 * sin(18.0 * field);
    float veins = filteredStripes(phase, fwidth(phase));
    float micro = filteredStripes(phase * 3.0, 3.0 * fwidth(phase));
    float cloud = fbmLod(p * 10.0, 6, px * 10.0);
    vec3 teal = mix(vec3(0.012, 0.087, 0.11), vec3(0.10, 0.49, 0.43), veins);
    vec3 honey = mix(vec3(0.31, 0.069, 0.022), vec3(1.05, 0.50, 0.14), veins);
    float iron = smoothstep(-0.22, 0.65, sin(field * 19.0 + 1.0) + 0.45 * warp);
    vec3 bands = mix(teal, honey, iron) * (0.80 + 0.24 * micro + 0.36 * cloud);
    float milky = filteredStripes(phase * 0.27, fwidth(phase) * 0.27) * (0.17 + 0.12 * cloud);
    bands = mix(bands, vec3(0.83, 0.84, 0.66), milky);

    float w = max(fwidth(field), 0.0001);
    float core = 1.0 - smoothstep(0.35 - w, 0.35 + w, field);
    float rind = smoothstep(1.03 - w, 1.03 + w, field);
    vec3 stone = mix(bands, vec3(0.18, 0.087, 0.038) * (1.0 + cloud), rind);
    if (core > 0.0) {
        vec2 crystalUV = p * 18.0;
        vec3 cell = voronoiBorder(crystalUV, 0.95);
        vec2 id = floor(crystalUV + cell.yz);
        float face = hash12(id);
        float ridge = 1.0 - smoothstep(0.0, 0.085 + px * 18.0, cell.x);
        float flash = pow(max(0.0, cos(6.283 * face + DRIFT * iTime)), 18.0);
        vec3 quartz = mix(vec3(0.09, 0.19, 0.22), vec3(0.68, 0.81, 0.77), face);
        // Plane breaks meet at the seed inside each crystal.
        vec2 axis = vec2(cos(6.283 * face), sin(6.283 * face));
        float facet = dot(cell.yz, axis) + 0.35 * abs(dot(cell.yz, vec2(-axis.y, axis.x)));
        float plane = smoothstep(-px * 18.0, px * 18.0, facet);
        stone = mix(stone, quartz * (0.46 + 0.44 * plane + 0.5 * cell.x) + ridge * (0.18 + flash), core);
    }
    float outline = 1.0 - smoothstep(1.085 - w, 1.085 + w, field);
    float glow = exp(-1.9 * length(p - vec2(-0.42, 0.24)));
    vec3 backdrop = vec3(0.013, 0.020, 0.025) * (1.0 + 0.4 * uv.y);
    vec3 color = mix(backdrop, stone * (0.74 + BACKLIGHT * glow), outline);
#if DEBUG_MODE == 3
    color = outline * vec3(core, (1.0 - core) * (1.0 - rind), rind);
#endif
    fragColor = vec4(linearToSrgb(tonemapNeutral(color)) + ditherTPDF(fragCoord, iFrame), 1.0);
}
