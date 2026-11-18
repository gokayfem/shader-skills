// A broken wooden hull embedded in a sand bank. Orbit the wreck or dive closer with the camera controls.
// @render time=3.2
// @variant DEBUG_MODE=4
#define DEBUG_MODE 0 // 4 primary trace cost

// Library code below is bundled from shader-skills (MIT): https://github.com/gokayfem/shader-skills
// ---- shader-skills library: common ----
const float TAU = 6.28318530717959;
// 2D rotation matrix. `rot2(a) * v` rotates v counter-clockwise by a radians. (cos, sin) is renormalized because GLSL ES leaves trig precision undefined: on SwiftShader and some mobile GPUs a raw cos/sin pair is far enough from unit length that rotations stop preserving distances.
mat2 rot2(float a) {
    vec2 cs = normalize(vec2(cos(a), sin(a)));
    return mat2(cs.x, cs.y, -cs.y, cs.x);
}

// ---- shader-skills library: hash ----
// PCG-style 1D integer hash.
uint pcgHash(uint v) {
    uint state = v * 747796405u + 2891336453u;
    uint word = ((state >> ((state >> 28u) + 4u)) ^ state) * 277803737u;
    return (word >> 22u) ^ word;
}
// 3D integer hash, three decorrelated outputs.
uvec3 pcg3d(uvec3 v) {
    v = v * 1664525u + 1013904223u;
    v.x += v.y * v.z;
    v.y += v.z * v.x;
    v.z += v.x * v.y;
    v ^= v >> 16u;
    v.x += v.y * v.z;
    v.y += v.z * v.x;
    v.z += v.x * v.y;
    return v;
}
// Map a uint to [0,1) using the top 24 bits, so the result can never round up to 1.0.
float uintToUnit(uint h) { return float(h >> 8u) * (1.0 / 16777216.0); }
vec2 uintToUnit(uvec2 h) { return vec2(h >> 8u) * (1.0 / 16777216.0); }
vec3 uintToUnit(uvec3 h) { return vec3(h >> 8u) * (1.0 / 16777216.0); }
vec4 uintToUnit(uvec4 h) { return vec4(h >> 8u) * (1.0 / 16777216.0); }
// Random float in [0,1) for an integer-valued 2D input.
float hash12(vec2 p) {
    uvec2 q = uvec2(ivec2(floor(p)));
    return uintToUnit(pcgHash(q.x + pcgHash(q.y)));
}
// Random vec3 in [0,1)^3 for an integer-valued 3D input.
vec3 hash33(vec3 p) { return uintToUnit(pcg3d(uvec3(ivec3(floor(p))))); }

// ---- shader-skills library: noise ----
// Quintic fade 6t^5 - 15t^4 + 10t^3: C2-continuous interpolation weights for lattice noise.
vec2 quinticFade(vec2 f) { return f * f * f * (f * (f * 6.0 - 15.0) + 10.0); }
vec3 quinticFade(vec3 f) { return f * f * f * (f * (f * 6.0 - 15.0) + 10.0); }
// Derivative of quinticFade: 30 t^2 (t - 1)^2.
vec2 quinticFadeDeriv(vec2 f) { return 30.0 * f * f * (f * (f - 2.0) + 1.0); }
vec3 quinticFadeDeriv(vec3 f) { return 30.0 * f * f * (f * (f - 2.0) + 1.0); }
// Unit gradient for a 2D lattice point.
vec2 latticeGradient(vec2 cell) {
    float a = TAU * hash12(cell);
    return vec2(cos(a), sin(a));
}
// 2D gradient (Perlin-style) noise with analytic derivatives. Returns vec3(value, d/dx, d/dy). Gradient noise has less grid-aligned blockiness than value noise. Prefer it for fBm.
vec3 gradientNoiseD(vec2 p) {
    vec2 i = floor(p), f = fract(p);
    vec2 u = quinticFade(f), du = quinticFadeDeriv(f);
    vec2 ga = latticeGradient(i);
    vec2 gb = latticeGradient(i + vec2(1.0, 0.0));
    vec2 gc = latticeGradient(i + vec2(0.0, 1.0));
    vec2 gd = latticeGradient(i + vec2(1.0, 1.0));
    float va = dot(ga, f);
    float vb = dot(gb, f - vec2(1.0, 0.0));
    float vc = dot(gc, f - vec2(0.0, 1.0));
    float vd = dot(gd, f - vec2(1.0, 1.0));
    float k = va - vb - vc + vd;
    float value = va + u.x * (vb - va) + u.y * (vc - va) + u.x * u.y * k;
    vec2 deriv = ga + u.x * (gb - ga) + u.y * (gc - ga) + u.x * u.y * (ga - gb - gc + gd)
               + du * (vec2(vb - va, vc - va) + u.yx * k);
    return vec3(value, deriv);
}
// 2D gradient noise.
float gradientNoise(vec2 p) { return gradientNoiseD(p).x; }
// Dot product of a hashed 3D gradient with the offset to its lattice corner.
float latticeGradDot(vec3 cell, vec3 corner, vec3 f) {
    vec3 g = hash33(cell + corner) * 2.0 - 1.0;
    return dot(g, f - corner);
}
// 3D gradient noise.
float gradientNoise(vec3 p) {
    vec3 i = floor(p), f = fract(p);
    vec3 u = quinticFade(f);
    return mix(mix(mix(latticeGradDot(i, vec3(0, 0, 0), f), latticeGradDot(i, vec3(1, 0, 0), f), u.x),
                   mix(latticeGradDot(i, vec3(0, 1, 0), f), latticeGradDot(i, vec3(1, 1, 0), f), u.x), u.y),
               mix(mix(latticeGradDot(i, vec3(0, 0, 1), f), latticeGradDot(i, vec3(1, 0, 1), f), u.x),
                   mix(latticeGradDot(i, vec3(0, 1, 1), f), latticeGradDot(i, vec3(1, 1, 1), f), u.x), u.y), u.z);
}
// 3D gradient noise with analytic derivatives. Returns vec4(value, d/dx, d/dy, d/dz); value equals gradientNoise(p). The trilinear blend is expanded as a polynomial in the fade weights, so the gradient is the same blend of the corner gradients plus the chain-rule term of the fade.
vec4 gradientNoiseD(vec3 p) {
    vec3 i = floor(p), f = fract(p);
    vec3 u = quinticFade(f), du = quinticFadeDeriv(f);
    vec3 g000 = hash33(i) * 2.0 - 1.0;
    vec3 g100 = hash33(i + vec3(1, 0, 0)) * 2.0 - 1.0;
    vec3 g010 = hash33(i + vec3(0, 1, 0)) * 2.0 - 1.0;
    vec3 g110 = hash33(i + vec3(1, 1, 0)) * 2.0 - 1.0;
    vec3 g001 = hash33(i + vec3(0, 0, 1)) * 2.0 - 1.0;
    vec3 g101 = hash33(i + vec3(1, 0, 1)) * 2.0 - 1.0;
    vec3 g011 = hash33(i + vec3(0, 1, 1)) * 2.0 - 1.0;
    vec3 g111 = hash33(i + vec3(1, 1, 1)) * 2.0 - 1.0;
    float v000 = dot(g000, f);
    float v100 = dot(g100, f - vec3(1, 0, 0));
    float v010 = dot(g010, f - vec3(0, 1, 0));
    float v110 = dot(g110, f - vec3(1, 1, 0));
    float v001 = dot(g001, f - vec3(0, 0, 1));
    float v101 = dot(g101, f - vec3(1, 0, 1));
    float v011 = dot(g011, f - vec3(0, 1, 1));
    float v111 = dot(g111, f - vec3(1, 1, 1));
    // value = v000 + kx u.x + ky u.y + kz u.z + kxy u.x u.y + kyz u.y u.z + kzx u.z u.x + kxyz u.x u.y u.z
    float kx = v100 - v000, ky = v010 - v000, kz = v001 - v000;
    float kxy = v000 - v100 - v010 + v110;
    float kyz = v000 - v010 - v001 + v011;
    float kzx = v000 - v100 - v001 + v101;
    float kxyz = -v000 + v100 + v010 - v110 + v001 - v101 - v011 + v111;
    float value = v000 + kx * u.x + ky * u.y + kz * u.z
                + kxy * u.x * u.y + kyz * u.y * u.z + kzx * u.z * u.x + kxyz * u.x * u.y * u.z;
    vec3 grad = g000 + (g100 - g000) * u.x + (g010 - g000) * u.y + (g001 - g000) * u.z
              + (g000 - g100 - g010 + g110) * u.x * u.y
              + (g000 - g010 - g001 + g011) * u.y * u.z
              + (g000 - g100 - g001 + g101) * u.z * u.x
              + (-g000 + g100 + g010 - g110 + g001 - g101 - g011 + g111) * u.x * u.y * u.z
              + du * vec3(kx + kxy * u.y + kzx * u.z + kxyz * u.y * u.z,
                          ky + kxy * u.x + kyz * u.z + kxyz * u.x * u.z,
                          kz + kyz * u.y + kzx * u.x + kxyz * u.x * u.y);
    return vec4(value, grad);
}
const int FBM_MAX_OCTAVES = 12;
// Screen-filtered 2D fBm with the same spectrum as fbm(p, octaves). Pass the pixel width in input lattice cells. Each octave fades between 4 and 2 pixels per cell. Unresolved bands contribute their mean (zero).
float fbmLod(vec2 p, int octaves, float footprint) {
    int count = clamp(octaves, 0, FBM_MAX_OCTAVES);
    if (count == 0) return 0.0;
    float width = max(footprint, 0.0);
    float sum = 0.0, amplitude = 1.0;
    const mat2 rotation = mat2(0.8, 0.6, -0.6, 0.8);
    for (int i = 0; i < FBM_MAX_OCTAVES; i++) {
        if (i >= count || width >= 0.5) break;
        float keep = 1.0 - smoothstep(0.25, 0.5, width);
        sum += amplitude * keep * gradientNoise(p);
        p = 2.01 * (rotation * p) + vec2(17.13, -3.71);
        amplitude *= 0.5;
        width *= 2.01;
    }
    // Normalize by the requested spectrum so removing detail never boosts the remaining bands.
    return 1.4 * sum / (2.0 * (1.0 - exp2(-float(count))));
}

// ---- shader-skills library: sdf3d ----
// Axis-aligned box with half-extents b.
float sdBox(vec3 p, vec3 b) {
    vec3 q = abs(p) - b;
    return length(max(q, 0.0)) + min(max(q.x, max(q.y, q.z)), 0.0);
}
// Capsule: segment a-b swept by radius r.
float sdCapsule(vec3 p, vec3 a, vec3 b, float r) {
    vec3 pa = p - a, ba = b - a;
    float h = clamp(dot(pa, ba) / dot(ba, ba), 0.0, 1.0);
    return length(pa - ba * h) - r;
}

// ---- shader-skills library: sdf2d ----
// Axis-aligned rectangle with half-extents b.
float sdBox(vec2 p, vec2 b) {
    vec2 d = abs(p) - b;
    return length(max(d, 0.0)) + min(max(d.x, d.y), 0.0);
}

// ---- shader-skills library: camera ----
// Camera-to-world matrix looking from ro toward ta, with roll in radians. Columns are (right, up, forward).
mat3 cameraMatrix(vec3 ro, vec3 ta, float roll) {
    vec3 w = normalize(ta - ro);
    vec3 up = vec3(sin(roll), cos(roll), 0.0);
    vec3 u = normalize(cross(w, up));
    vec3 v = cross(u, w);
    return mat3(u, v, w);
}
// Primary ray direction for screen point uv. focal is about 1.5-2.5 for natural framing (see fovToFocal).
vec3 cameraRay(mat3 cam, vec2 uv, float focal) {
    return normalize(cam * vec3(uv, focal));
}

// ---- shader-skills library: water ----
// Beer-Lambert RGB transmittance exp(-extinction*pathLength). Negative extinction/path is clamped to zero. Use the refracted optical path, not vertical water depth. Attenuation alone does not add scattered light.
vec3 beerTransmittance(vec3 extinction, float pathLength) {
    return exp(-max(extinction,vec3(0.0))*max(pathLength,0.0));
}
// Procedural caustic filaments on a receiving surface. Footprint is measured in input pattern units.
float causticLace(vec2 p, float time, float footprint) {
    vec2 q=p+0.22*vec2(sin(1.7*p.y+time),cos(1.3*p.x-0.7*time));
    float f=sin(2.9*q.x+q.y)+0.80*sin(2.6*q.y-1.2*q.x);
    float width=clamp(2.0*footprint,0.015,0.30);
    float line=1.0-smoothstep(0.055,0.055+width,abs(f));
    return mix(line,0.085,smoothstep(0.08,0.32,footprint));
}

// ---- shader-skills library: color ----
// Exact sRGB encode (OETF): linear -> display [0,1]. Apply exactly once, after tonemapping. pow(c, 1/2.2) is visibly wrong near black (0.001 encodes to 0.013 here vs 0.043 with 2.2).
vec3 linearToSrgb(vec3 c) {
    c = max(c, 0.0);
    return mix(c * 12.92, 1.055 * pow(c, vec3(1.0 / 2.4)) - 0.055, step(0.0031308, c));
}

// ---- shader-skills library: tonemap ----
// Khronos PBR Neutral: leaves [0, ~0.76] almost untouched and compresses only highlights, preserving hue.
vec3 tonemapNeutral(vec3 c) {
    const float startCompression = 0.76;
    const float desaturation = 0.15;
    c = max(c, 0.0);
    float x = min(c.r, min(c.g, c.b));
    float offset = x < 0.08 ? x - 6.25 * x * x : 0.04;
    c -= offset;
    float peak = max(c.r, max(c.g, c.b));
    if (peak < startCompression) return c;
    const float d = 1.0 - startCompression;
    float newPeak = 1.0 - d * d / (peak + d - startCompression);
    c *= newPeak / peak;
    float g = 1.0 - 1.0 / (desaturation * (peak - newPeak) + 1.0);
    return mix(c, vec3(newPeak), g);
}

// ---- shader ----
const float CAMERA_ARC=0.63;
const float DIVE_DEPTH=0.45;
const float MURK=0.16;

float sandHeight(vec2 p){ return -0.60+0.12*sin(0.86*p.x-0.42*p.y)+0.042*sin(3.1*p.y+0.4*p.x); }
vec2 wreckMap(vec3 p){
    vec2 nearest=vec2((p.y-sandHeight(p.xz))*0.93,0.0);
    // Bounding box culls timber detail when a step is well clear of the wreck.
    vec3 q=p;q.xy=rot2(0.19)*q.xy;
    float bound=sdBox(q-vec3(0,0.32,0),vec3(1.25,1.3,2.75));
    if(bound>0.22)return vec2(min(bound,nearest.x),nearest.y);
    float taper=1.0-0.60*pow(clamp(abs(q.z)/2.5,0.0,1.0),2.0);
    vec2 hull=q.xy-vec2(0.0,0.98);
    float ring=abs(length(vec2(hull.x/taper,hull.y*0.74))-1.0)*min(taper,1.0)-0.054;
    float rib=abs(mod(q.z+0.18,0.36)-0.18)-0.055;
    float brokenTop=0.83+0.23*sin(4.2*floor(q.z/0.36+0.5)+sign(q.x));
    float frame=max(max(ring,rib),max(q.y-brokenTop,abs(q.z)-2.5));
    float plank=abs(mod(q.y+0.10,0.20)-0.10)-0.081;
    float shell=max(max(ring-0.016,plank),max(q.y-(q.x<0.0?0.12:0.42),abs(q.z)-2.42));
    float timber=min(frame,shell);
    float mast=sdCapsule(q,vec3(0.0,-0.25,-0.57),vec3(0.52,1.43,-0.93),0.075);
    float yard=sdCapsule(q,vec3(-0.42,0.96,-0.88),vec3(1.04,0.85,-0.66),0.045);
    float d=0.70*min(timber,min(mast,yard));
    return d<nearest.x?vec2(d,1.0):nearest;
}
void mainImage(out vec4 fragColor,in vec2 fragCoord){
    vec2 uv=(2.0*fragCoord-iResolution.xy)/iResolution.y;
    float a=CAMERA_ARC+0.05*sin(iTime*0.19);
    vec3 ro=vec3(4.5*cos(a),2.4-1.8*DIVE_DEPTH,4.5*sin(a));
    vec3 rd=cameraRay(cameraMatrix(ro,vec3(0,0.18,0),0.035),uv,1.65);
    vec3 water=mix(vec3(0.006,0.052,0.092),vec3(0.055,0.24,0.27),smoothstep(-0.6,0.5,rd.y));
    vec3 col=water;
    float t=0.02,work=0.0;
    for(int i=0;i<105;i++){
        vec3 p=ro+t*rd;
        vec2 h=wreckMap(p);work=float(i+1)/105.0;
        if(h.x<0.001+0.0004*t){
            vec2 e=vec2(0.002,0.0);
            vec3 n=normalize(vec3(wreckMap(p+e.xyy).x-wreckMap(p-e.xyy).x,
                                  wreckMap(p+e.yxy).x-wreckMap(p-e.yxy).x,
                                  wreckMap(p+e.yyx).x-wreckMap(p-e.yyx).x));
            float fp=2.0*t/(1.65*iResolution.y);
            float caustic=causticLace(2.2*p.xz+0.42*p.y,iTime*0.43,2.2*fp);
            vec3 base=vec3(0.31,0.39,0.30)*(0.85+0.15*sin(19.0*p.z+2.3*sin(2.0*p.x)));
            if(h.y>0.5){
                float grain=fbmLod(vec2(p.z*2.0,p.y*28.0+p.x*7.0),4,fp*28.0);
                base=mix(vec3(0.075,0.042,0.024),vec3(0.32,0.18,0.084),0.5+0.5*grain);
                base=mix(base,vec3(0.09,0.18,0.09),0.38*smoothstep(0.32,0.84,n.y));
            }
            vec3 lit=base*(0.22+max(dot(n,normalize(vec3(-0.3,0.84,-0.32))),0.0)*(1.05+1.5*caustic));
            col=mix(water,lit,beerTransmittance(MURK*vec3(1.45,0.90,0.74),t));
            break;
        }
        t+=max(h.x,0.001);
        if(t>18.0)break;
    }
    // Integrate the light shaft pattern along the visible segment; geometry caps its extent.
    float span=min(t,12.0),shaft=0.0;
    for(int j=0;j<8;j++){
        float s=(float(j)+0.5)*span/8.0;
        vec3 q=ro+s*rd;
        shaft+=exp(-0.27*s)*causticLace(0.85*q.xz+q.y*vec2(0.24,0.10),0.22*iTime,0.07)*span/8.0;
    }
    col+=shaft*vec3(0.035,0.065,0.058);
#if DEBUG_MODE == 4
    col=vec3(work,work*work,1.0-work);
#endif
    fragColor=vec4(linearToSrgb(tonemapNeutral(col)),1.0);
}
