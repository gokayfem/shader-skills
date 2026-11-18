// Eroded mountains at golden hour under a physically based sky.
// Shows: derivative-damped fBm terrain, heightfield marching as a scaled bound, slope/altitude material masks,
// soft terrain shadows, single-scattering atmosphere (Rayleigh/Mie/ozone), aerial perspective, sun disk, AgX.
// Units are kilometers so the atmosphere constants apply directly.
// @render time=0.0

#define DEBUG_MODE 0   // 0 final | 1 height map | 2 normals | 3 material masks | 4 steps | 5 shadow | 6 light terms

// ---- art controls ----
const vec3  SUN_DIR = normalize(vec3(0.85, 0.2, -0.45));
const float SUN_INTENSITY = 20.0;
const float TERRAIN_SCALE = 0.32;    // horizontal frequency: smaller = broader mountains
const float TERRAIN_HEIGHT = 2.0;    // vertical scale in km
const float SNOW_LINE = 1.05;        // km
const float EXPOSURE = 1.4;
const float FOCAL = 1.8;

float terrainHeight(vec2 xz) { return TERRAIN_HEIGHT * erodedFbm(xz * TERRAIN_SCALE, 9); }

vec2 map(vec3 p) {
    // p.y - h(xz) is not a distance on slopes. Scaling by 0.45 keeps the march safe for slopes up to about 63 degrees.
    return vec2(0.45 * (p.y - terrainHeight(p.xz)), 1.0);
}

// Transmittance from a point at heightKm toward dir, through the atmosphere (colors the sunlight).
vec3 sunTransmittance(float heightKm, vec3 dir) {
    vec3 ro = vec3(0.0, ATMOSPHERE_GROUND_KM + heightKm, 0.0);
    if (iSphere(ro, dir, vec3(0.0), ATMOSPHERE_GROUND_KM).x > 0.0) return vec3(0.0);
    float tl = iSphere(ro, dir, vec3(0.0), ATMOSPHERE_TOP_KM).y;
    vec3 depth = vec3(0.0);
    for (int i = 0; i < 12; i++) {
        vec3 q = ro + dir * ((float(i) + 0.5) * tl / 12.0);
        depth += atmosphereExtinction(atmosphereDensity(length(q) - ATMOSPHERE_GROUND_KM)) * tl / 12.0;
    }
    return exp(-depth);
}

vec3 render(vec2 fragCoord) {
    vec2 uv = (2.0 * fragCoord - iResolution.xy) / iResolution.y;

#if DEBUG_MODE == 1
    return vec3(terrainHeight(uv * 8.0) / TERRAIN_HEIGHT * 0.5 + 0.3);
#endif

    vec3 ro = vec3(-3.0 + 0.2 * iTime, 0.0, 4.0 - 0.6 * iTime);
    ro.y = terrainHeight(ro.xz) + 0.9;
    vec3 ta = ro + vec3(0.25, -0.16, -1.0);
    mat3 cam = cameraMatrix(ro, ta, 0.0);
    vec3 rd = cameraRay(cam, uv, FOCAL);

    vec3 sunColor = SUN_INTENSITY * 0.12 * sunTransmittance(ro.y, SUN_DIR);
    vec3 hit = raymarch(ro, rd, 0.01, 40.0, 1.0);
#if DEBUG_MODE == 4
    return debugHeat(hit.z / float(RAYMARCH_MAX_STEPS));
#endif
    if (hit.x < 0.0) {
        return atmosphereSky(rd, SUN_DIR, ro.y, SUN_INTENSITY) + sunColor * 40.0 * sunDisk(rd, SUN_DIR, 0.012, 2.0 / (iResolution.y * FOCAL));
    }

    float t = hit.x;
    vec3 p = ro + rd * t;
    vec3 n = calcNormal(p, 0.001 * t);

    // material masks: rock on steep slopes, grass on flats, snow on high upward-facing ground
    // Project a pixel cone onto the local surface, stretching it at grazing incidence.
    float footprint = (2.0 * t / (iResolution.y * FOCAL)) / max(abs(dot(n, rd)), 0.05);
    float flatness = smoothstep(0.72, 0.88, n.y);
    float snow = smoothstep(SNOW_LINE - 0.15, SNOW_LINE + 0.1, p.y + 0.25 * fbmLod(p.xz * 6.0, 3, footprint * 6.0))
               * smoothstep(0.55, 0.8, n.y);
    float grain = 0.5 + 0.5 * fbmLod(p.xz * 40.0, 4, footprint * 40.0);
    vec3 rock = mix(vec3(0.10, 0.085, 0.07), vec3(0.20, 0.17, 0.14), grain);
    vec3 grass = mix(vec3(0.06, 0.08, 0.03), vec3(0.11, 0.12, 0.05), grain);
    vec3 albedo = mix(rock, grass, flatness * (1.0 - smoothstep(0.6, 1.0, p.y)));
    albedo = mix(albedo, vec3(0.85, 0.88, 0.92), snow);

    float sha = softShadow(p + n * 0.01, SUN_DIR, 0.02, 20.0, 16.0);
    float NoL = clamp(dot(n, SUN_DIR), 0.0, 1.0);
    vec3 skyAmbient = atmosphereSky(vec3(0.0, 1.0, 0.0), SUN_DIR, p.y, SUN_INTENSITY);
    vec3 key = albedo * sunColor * NoL * sha;
    vec3 fill = albedo * skyAmbient * (0.5 + 0.5 * n.y) * 2.0;
    vec3 bounce = albedo * sunColor * 0.08 * clamp(dot(n, normalize(vec3(-SUN_DIR.x, 0.0, -SUN_DIR.z))), 0.0, 1.0);

#if DEBUG_MODE == 2
    return debugNormal(n);
#elif DEBUG_MODE == 3
    return vec3(1.0 - flatness, flatness, snow);
#elif DEBUG_MODE == 5
    return vec3(sha);
#elif DEBUG_MODE == 6
    return vec3(luminance(key), luminance(fill), luminance(bounce)) * 2.0;
#endif

    vec3 col = key + fill + bounce;
    // aerial perspective: attenuate by the air between camera and hit and fill with sky color. Use the sky at the
    // horizon-clamped direction: below the planet's horizon the sky integral jumps, which would print a seam.
    vec3 fogColor = atmosphereSky(normalize(vec3(rd.x, max(rd.y, 0.02), rd.z)), SUN_DIR, ro.y, SUN_INTENSITY);
    vec3 airT = exp(-t * atmosphereExtinction(atmosphereDensity(0.5 * (ro.y + p.y))) * 4.0);
    return col * airT + fogColor * (1.0 - airT);
}

void mainImage(out vec4 fragColor, in vec2 fragCoord) {
    vec3 col = render(fragCoord);
#if DEBUG_MODE == 0
    col = linearToSrgb(tonemapAgX(col * EXPOSURE)) + ditherTPDF(fragCoord, iFrame);
#endif
    fragColor = vec4(col, 1.0);
}
