// Two-ink relief print of potential flow around a circular cylinder. No simulated wake.
// @render time=2.0
// @variant DEBUG_MODE=2
// @variant QUALITY=1
#define DEBUG_MODE 0 // 2 velocity direction and speed
#define QUALITY 2    // 1 eight taps per side, 2 sixteen

// Library code below is bundled from shader-skills (MIT): https://github.com/gokayfem/shader-skills
// ---- shader-skills library: common ----
const float PI  = 3.14159265358979;
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

// ---- shader-skills library: noise ----
// Quintic fade 6t^5 - 15t^4 + 10t^3: C2-continuous interpolation weights for lattice noise.
vec2 quinticFade(vec2 f) { return f * f * f * (f * (f * 6.0 - 15.0) + 10.0); }
vec3 quinticFade(vec3 f) { return f * f * f * (f * (f * 6.0 - 15.0) + 10.0); }
// Derivative of quinticFade: 30 t^2 (t - 1)^2.
vec2 quinticFadeDeriv(vec2 f) { return 30.0 * f * f * (f * (f - 2.0) + 1.0); }
vec3 quinticFadeDeriv(vec3 f) { return 30.0 * f * f * (f * (f - 2.0) + 1.0); }
// 2D value noise with analytic derivatives. Returns vec3(value, d/dx, d/dy).
vec3 valueNoiseD(vec2 p) {
    vec2 i = floor(p), f = fract(p);
    vec2 u = quinticFade(f), du = quinticFadeDeriv(f);
    float a = hash12(i);
    float b = hash12(i + vec2(1.0, 0.0));
    float c = hash12(i + vec2(0.0, 1.0));
    float d = hash12(i + vec2(1.0, 1.0));
    float k1 = b - a, k2 = c - a, k3 = a - b - c + d;
    return vec3(a + k1 * u.x + k2 * u.y + k3 * u.x * u.y,
                du * vec2(k1 + k3 * u.y, k2 + k3 * u.x));
}

// ---- shader-skills library: pattern ----
// Box-filtered stripes along x with period 1 and duty cycle 0.5. Returns 0..1.
float filteredStripes(float x, float w) {
    w = max(w, 1e-4);
    float a = x + 0.5 * w, b = x - 0.5 * w;
    float ia = floor(a) * 0.5 + max(fract(a) - 0.5, 0.0);
    float ib = floor(b) * 0.5 + max(fract(b) - 0.5, 0.0);
    return (ia - ib) / w;
}

// ---- shader-skills library: color ----
// Exact sRGB encode (OETF): linear -> display [0,1]. Apply exactly once, after tonemapping. pow(c, 1/2.2) is visibly wrong near black (0.001 encodes to 0.013 here vs 0.043 with 2.2).
vec3 linearToSrgb(vec3 c) {
    c = max(c, 0.0);
    return mix(c * 12.92, 1.055 * pow(c, vec3(1.0 / 2.4)) - 0.055, step(0.0031308, c));
}

// ---- shader ----
const float RADIUS=0.37;
const vec2 CENTER=vec2(-0.37,0.06);

vec2 flow(vec2 p) {
    float r2=max(dot(p,p),RADIUS*RADIUS);
    float scale=RADIUS*RADIUS/(r2*r2);
    return vec2(1.0,0.0)-scale*vec2(p.x*p.x-p.y*p.y,2.0*p.x*p.y);
}
vec2 direction(vec2 p) {
    vec2 v=flow(p);
    return v/max(length(v),0.0001);
}
float paperNoise(vec2 p,float frequency) {
    return valueNoiseD(p*frequency).x;
}
float streaks(vec2 p,float px) {
#if QUALITY == 1
    const int TAPS=8;
#else
    const int TAPS=16;
#endif
    float h=0.095/float(TAPS), freq=min(170.0,0.45/px);
    float total=paperNoise(p,freq), weights=1.0;
    for(int signIndex=0;signIndex<2;signIndex++){
        vec2 s=p;
        float stepSize=signIndex==0?h:-h;
        for(int j=1;j<=TAPS;j++){
            vec2 mid=s+0.5*stepSize*direction(s);
            s+=stepSize*direction(mid);
            if(dot(s,s)<RADIUS*RADIUS) break;
            float w=0.5+0.5*cos(PI*float(j)/float(TAPS+1));
            total+=w*paperNoise(s,freq);
            weights+=w;
        }
    }
    return total/weights;
}
void mainImage(out vec4 fragColor,in vec2 fragCoord) {
    vec2 p=rot2(-0.095)*((2.0*fragCoord-iResolution.xy)/iResolution.y-CENTER);
    float px=2.0/iResolution.y, r2=max(dot(p,p),RADIUS*RADIUS);
    float psi=p.y*(1.0-RADIUS*RADIUS/r2);
    float potential=p.x*(1.0+RADIUS*RADIUS/r2);
    vec2 v=flow(p);
    vec3 paper=vec3(0.87,0.77,0.54), ink=vec3(0.018,0.092,0.11), vermilion=vec3(0.67,0.105,0.045);
    vec3 col=paper;
    float distanceToBody=length(p)-RADIUS;
    if(distanceToBody>-px){
        float lines=streaks(p,px);
        float grain=smoothstep(0.42,0.61,lines);
        float contour=filteredStripes(28.0*psi,28.0*px*max(length(v),0.01));
        float tracer=0.5+0.5*cos(9.0*potential-1.9*iTime+4.0*psi);
        float ribbon=1.0-smoothstep(0.018,0.050,abs(psi-0.065));
        col=mix(paper,ink,clamp(0.80*grain+0.13*contour,0.0,1.0));
        col=mix(col,vermilion,ribbon*(0.50+0.40*tracer));
    }
    float interior=1.0-smoothstep(-px,px,distanceToBody);
    vec3 lacquer=mix(ink,vermilion,0.76+0.19*p.y/RADIUS);
    col=mix(col,lacquer,interior);
    col*=0.96+0.07*hash12(fragCoord);
#if DEBUG_MODE == 2
    col=mix(vec3(0.5+0.22*v,0.28*length(v)),vec3(0.03),interior);
#endif
    fragColor=vec4(linearToSrgb(col),1.0);
}
