// Null-ray orbits around a Schwarzschild black hole. Lengths in Schwarzschild radii.
// @render time=4.0
// @variant DEBUG_MODE=3
#define DISK_PASS 0 // 0 single pass, 1 bake camera transfer, 2 shade the retained transfer texture
#define DEBUG_MODE 0 // 3 direct disk (copper), higher-order disk (blue), capture (black)
const float INCLINATION = 0.17;  // camera elevation above disk in radians
const float CAMERA_RADIUS = 22.0;
const float ANGLE_STEP = 0.048;
const float EXPOSURE = 1.3;

vec3 diskLight(vec3 p, float shift) {
    float r = length(p);
    float az = atan(p.z, p.x);
    float phi = az - iTime * 0.85 / pow(r / 3.0, 1.5);
    vec2 orbit = vec2(cos(phi), sin(phi));
    float turbulent = fbm(vec2(r * 1.8, dot(orbit, vec2(2.0, 1.3))) + 0.22 * orbit, 4);
    float filaments = 0.55 + 0.45 * sin(13.0 * r + 9.0 * turbulent + 3.0 * sin(phi + r));
    float heat = (0.45 + 0.9 * filaments) * (0.75 + 0.6 * turbulent);
    float radial = smoothstep(3.0, 3.55, r) * (1.0 - smoothstep(9.0, 13.0, r));
    float temperature = 6200.0 * pow(3.0 / r, 0.72) * shift * (0.85 + 0.2 * heat);
    return blackbody(max(1000.0, temperature)) * radial * heat * pow(shift, 4.0) * 5.0 / sqrt(r);
}

vec3 distantStars(vec3 dir) {
    vec2 uv = vec2(atan(dir.z, dir.x), asin(clamp(dir.y, -1.0, 1.0)));
    vec2 q = uv * 115.0;
    vec2 id = floor(q);
    vec3 h = hash32(id);
    float star = exp(-65.0 * dot2(fract(q) - mix(vec2(0.12), vec2(0.88), h.xy))) * step(0.945, h.z);
    float dust = 0.5 + 0.5 * fbm(dir * 4.0, 4, 2.01, 0.5);
    return vec3(0.004, 0.005, 0.011) + vec3(0.065, 0.042, 0.065) * dust * dust
         + mix(vec3(0.65, 0.77, 1.0), vec3(1.0, 0.71, 0.40), h.x) * star;
}

vec4 diskTransfer(vec2 fragCoord) {
    vec2 uv = rot2(0.095) * ((2.0 * fragCoord - iResolution.xy) / iResolution.y);
    vec3 radial = vec3(0.0, sin(INCLINATION), cos(INCLINATION));
    vec3 ro = CAMERA_RADIUS * radial;
    vec3 rd = cameraRay(cameraMatrix(ro, vec3(0), 0.0), uv, 3.1);
    vec3 side = rd - dot(rd, radial) * radial;
    float sine = length(side);
    side /= max(sine, 1e-8);
    float u0 = 1.0 / CAMERA_RADIUS;
    vec2 orbit = vec2(u0, -dot(rd, radial) * u0 * sqrt(1.0 - u0) / max(sine, 1e-8));
    // Intersections with the disk plane repeat every pi radians in the ray plane.
    float crossing = atan(-radial.y, side.y);
    crossing = mod(crossing, PI);
    for (int i = 0; i < 220; ++i) {
        float phi = float(i) * ANGLE_STEP;
        vec2 next = nullOrbitStep(orbit, ANGLE_STEP);
        if (phi + ANGLE_STEP >= crossing) {
            vec2 atDisk = nullOrbitStep(orbit, clamp(crossing - phi, 0.0, ANGLE_STEP));
            if (atDisk.x > 1.0 / 13.0 && atDisk.x < 1.0 / 3.0) {
                vec3 er = cos(crossing) * radial + sin(crossing) * side;
                vec3 ep = -sin(crossing) * radial + cos(crossing) * side;
                vec3 view = normalize(atDisk.y * er - atDisk.x * sqrt(1.0 - atDisk.x) * ep);
                vec3 p = er / atDisk.x;
                float r = length(p);
                float speed = inversesqrt(2.0 * (r - 1.0));
                vec3 tangent = vec3(-p.z, 0.0, p.x) / r;
                float shift = sqrt((1.0 - 1.0 / r) * (1.0 - speed * speed))
                            / (1.0 - speed * dot(tangent, view));
                return vec4(p.xz,shift,crossing < PI ? 2.0 : 3.0);
            }
            crossing += PI;
        }
        if (next.x >= 1.0 || sine < 1e-7) break;
        if (next.x <= 0.0) {
            float escape = phi + ANGLE_STEP * orbit.x / (orbit.x - next.x);
            return vec4(cos(escape) * radial + sin(escape) * side, 1.0);
        }
        orbit = next;
    }
    return vec4(0.0);
}

void mainImage(out vec4 fragColor, in vec2 fragCoord) {
#if DISK_PASS == 2
    vec4 transfer = texelFetch(iChannel0, ivec2(fragCoord), 0);
#else
    vec4 transfer = diskTransfer(fragCoord);
#endif
#if DISK_PASS == 1
    fragColor = transfer;
    return;
#endif
    vec3 col = vec3(0.0);
    if (transfer.w > 1.5) {
        col = diskLight(vec3(transfer.x,0.0,transfer.y),transfer.z);
#if DEBUG_MODE == 3
        col = transfer.w < 2.5 ? vec3(0.8,0.28,0.08) : vec3(0.1,0.55,1.0);
#endif
    } else if (transfer.w > 0.5) {
        col = distantStars(transfer.xyz);
    }
    fragColor = vec4(linearToSrgb(tonemapAgX(EXPOSURE * col)) + ditherTPDF(fragCoord, iFrame), 1.0);
}
