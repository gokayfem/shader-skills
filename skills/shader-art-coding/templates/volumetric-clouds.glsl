// Cumulus cloud layer lit by a low sun, raymarched as a participating medium.
// Shows: coverage-remapped 3D fBm density with a height profile, energy-conserving integration (volumeStep),
// dual-lobe phase for silver linings, Beer-Powder self-shadowing from a short light march, cheap density for
// lighting, jittered start (no banding), sky and sun, AgX.
// @render time=4.0

#define DEBUG_MODE 0   // 0 final | 4 march cost | 5 density / transmittance | 6 light vs ambient

// ---- art controls ----
const vec3  SUN_DIR = normalize(vec3(0.6, 0.25, -0.75));
const vec3  SUN_COLOR = vec3(6.0, 5.2, 4.2);
const float CLOUD_BOTTOM = 1.5;
const float CLOUD_TOP = 3.2;
const float COVERAGE = 0.42;       // 0 = clear, 1 = overcast
const float DENSITY = 9.0;         // extinction scale: higher = darker, more opaque cores
const float WIND = 0.06;
const float EXPOSURE = 1.0;

// Density in [0, DENSITY]. Use fewer octaves for the light-march samples.
float cloudDensity(vec3 p, int octaves) {
    if (COVERAGE <= 0.0) return 0.0;
    float h = (p.y - CLOUD_BOTTOM) / (CLOUD_TOP - CLOUD_BOTTOM);
    if (h < 0.0 || h > 1.0) return 0.0;
    float profile = smoothstep(0.0, 0.12, h) * (1.0 - smoothstep(0.45, 1.0, h));   // flat-ish bottoms, rounded tops
    vec3 q = p * 0.55 + vec3(iTime * WIND, 0.0, 0.0);
    float n = 0.5 + 0.5 * fbm(q, octaves, 2.02, 0.5);
    float d = saturate((n * profile - (1.0 - COVERAGE)) / COVERAGE);
    return d * DENSITY;
}

vec3 render(vec2 fragCoord) {
    vec2 uv = (2.0 * fragCoord - iResolution.xy) / iResolution.y;
    vec3 ro = vec3(0.0, 0.3, 0.0);
    mat3 cam = cameraMatrix(ro, ro + vec3(0.35, 0.32, -1.0), 0.0);
    vec3 rd = cameraRay(cam, uv, 1.6);

    vec3 sky = simpleSky(rd, SUN_DIR) * 1.2;
    vec3 col = sky + SUN_COLOR * sunDisk(rd, SUN_DIR, 0.03, 2.0 / (iResolution.y * 1.6));
    if (rd.y <= 0.01) return col;

    // clip the march to the cloud slab
    float t0 = (CLOUD_BOTTOM - ro.y) / rd.y;
    float t1 = min((CLOUD_TOP - ro.y) / rd.y, 30.0);
    if (t1 <= t0) return col;
    const int STEPS = 48;
    float dt = (t1 - t0) / float(STEPS);
    float t = t0 + dt * interleavedGradientNoise(floor(fragCoord), iFrame);   // jitter hides slicing

    float mu = dot(rd, SUN_DIR);
    float phase = 4.0 * PI * phaseDualHG(mu, 0.75, -0.25, 0.75);   // artist units: isotropic scattering = 1
    vec3 radiance = vec3(0.0), transmittance = vec3(1.0);
    float lightAcc = 0.0, ambientAcc = 0.0;
    int steps = 0;
    for (int i = 0; i < STEPS; i++) {
        vec3 p = ro + rd * t;
        float sigma = cloudDensity(p, 5);
        if (sigma > 0.001) {
            // short light march toward the sun with the cheap (3-octave) density
            float depth = 0.0;
            for (int j = 0; j < 5; j++) depth += cloudDensity(p + SUN_DIR * (0.1 + 0.25 * float(j)), 3) * 0.25;
            float h = (p.y - CLOUD_BOTTOM) / (CLOUD_TOP - CLOUD_BOTTOM);
            vec3 sunLight = SUN_COLOR * phase * mix(exp(-depth), beerPowder(depth), 0.5 - 0.5 * mu);
            vec3 ambient = vec3(0.22, 0.28, 0.38) * (0.3 + 0.7 * h);
            volumeStep(radiance, transmittance, vec3(sigma), sigma * (sunLight + ambient), dt);
            lightAcc += luminance(sunLight) * transmittance.x;
            ambientAcc += luminance(ambient) * transmittance.x;
        }
        steps++;
        if (transmittance.x < 0.01) break;
        t += dt;
    }

#if DEBUG_MODE == 4
    return debugHeat(float(steps) / float(STEPS));
#elif DEBUG_MODE == 5
    return vec3(1.0 - transmittance.x, cloudDensity(ro + rd * (t0 + 0.5 * (t1 - t0)), 5) / DENSITY, 0.0);
#elif DEBUG_MODE == 6
    return vec3(lightAcc, ambientAcc, 0.0) * 0.1;
#endif

    // distant clouds fade into the sky
    float fade = exp(-0.04 * max(t0, 0.0));
    return mix(col, col * transmittance + radiance, fade);
}

void mainImage(out vec4 fragColor, in vec2 fragCoord) {
    vec3 col = render(fragCoord);
#if DEBUG_MODE == 0
    col = linearToSrgb(tonemapAgX(col * EXPOSURE)) + ditherTPDF(fragCoord, iFrame);
#endif
    fragColor = vec4(col, 1.0);
}
