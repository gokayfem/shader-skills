// Moving through staggered curtains of kelp below a shoal and a gliding manta.
// @render time=3.0
// @variant DEBUG_MODE=3
#define DEBUG_MODE 0 // 3 fronds, wildlife, shafts

// Library code below is bundled from shader-skills (MIT): https://github.com/gokayfem/shader-skills
// ---- shader-skills library: common ----
const float PI  = 3.14159265358979;
// Squared length.
float dot2(vec2 v) { return dot(v, v); }
float dot2(vec3 v) { return dot(v, v); }
// 2D rotation matrix. `rot2(a) * v` rotates v counter-clockwise by a radians. (cos, sin) is renormalized because GLSL ES leaves trig precision undefined: on SwiftShader and some mobile GPUs a raw cos/sin pair is far enough from unit length that rotations stop preserving distances.
mat2 rot2(float a) {
    vec2 cs = normalize(vec2(cos(a), sin(a)));
    return mat2(cs.x, cs.y, -cs.y, cs.x);
}

// ---- shader-skills library: hash ----
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
// Random vec3 in [0,1)^3 for an integer-valued 2D input.
vec3 hash32(vec2 p) {
    ivec2 q = ivec2(floor(p));
    return uintToUnit(pcg3d(uvec3(q.x, q.y, 0x9E37u)));
}

// ---- shader-skills library: sdf3d ----
// Unsigned distance to a 3D quadratic Bezier curve A-B-C; subtract a radius for a tube. Solves the closest-point cubic analytically. Falls back to a segment when the curve is straight.
float sdQuadraticBezier(vec3 pos, vec3 A, vec3 B, vec3 C) {
    vec3 a = B - A;
    vec3 b = A - 2.0 * B + C;
    vec3 c = a * 2.0;
    vec3 d = A - pos;
    float bb = dot(b, b);
    if (bb < 1e-10) {
        vec3 pa = pos - A, ca = C - A;
        return length(pa - ca * clamp(dot(pa, ca) / dot(ca, ca), 0.0, 1.0));
    }
    float kk = 1.0 / bb;
    float kx = kk * dot(a, b);
    float ky = kk * (2.0 * dot(a, a) + dot(d, b)) / 3.0;
    float kz = kk * dot(d, a);
    float pp = ky - kx * kx;
    float q = kx * (2.0 * kx * kx - 3.0 * ky) + kz;
    float h = q * q + 4.0 * pp * pp * pp;
    float res;
    if (h >= 0.0) {
        h = sqrt(h);
        vec2 x = (vec2(h, -h) - q) / 2.0;
        vec2 uv = sign(x) * pow(abs(x), vec2(1.0 / 3.0));
        float t = clamp(uv.x + uv.y - kx, 0.0, 1.0);
        res = dot2(d + (c + b * t) * t);
    } else {
        float z = sqrt(-pp);
        float v = acos(clamp(q / (pp * z * 2.0), -1.0, 1.0)) / 3.0;
        float m = cos(v);
        float n = sin(v) * 1.7320508;
        vec2 t = clamp(vec2(m + m, -n - m) * z - kx, 0.0, 1.0);
        res = min(dot2(d + (c + b * t.x) * t.x), dot2(d + (c + b * t.y) * t.y));
    }
    return sqrt(res);
}

// ---- shader-skills library: sdf2d ----
// Unsigned distance to segment a-b. Subtract a width to get a stroke.
float sdSegment(vec2 p, vec2 a, vec2 b) {
    vec2 pa = p - a, ba = b - a;
    float h = clamp(dot(pa, ba) / dot(ba, ba), 0.0, 1.0);
    return length(pa - ba * h);
}
// Unsigned distance to a quadratic Bezier curve A-B-C. Solves the closest-point cubic analytically. Falls back to a segment when the curve degenerates to a line. Float error in the cubic solve is about 1e-4.
float sdQuadraticBezier(vec2 pos, vec2 A, vec2 B, vec2 C) {
    vec2 a = B - A;
    vec2 b = A - 2.0 * B + C;
    vec2 c = a * 2.0;
    vec2 d = A - pos;
    float bb = dot(b, b);
    if (bb < 1e-10) return sdSegment(pos, A, C);
    float kk = 1.0 / bb;
    float kx = kk * dot(a, b);
    float ky = kk * (2.0 * dot(a, a) + dot(d, b)) / 3.0;
    float kz = kk * dot(d, a);
    float pp = ky - kx * kx;
    float q = kx * (2.0 * kx * kx - 3.0 * ky) + kz;
    float h = q * q + 4.0 * pp * pp * pp;
    float res;
    if (h >= 0.0) {
        h = sqrt(h);
        vec2 x = (vec2(h, -h) - q) / 2.0;
        vec2 uv = sign(x) * pow(abs(x), vec2(1.0 / 3.0));
        float t = clamp(uv.x + uv.y - kx, 0.0, 1.0);
        res = dot2(d + (c + b * t) * t);
    } else {
        float z = sqrt(-pp);
        float v = acos(clamp(q / (pp * z * 2.0), -1.0, 1.0)) / 3.0;
        float m = cos(v);
        float n = sin(v) * 1.7320508;
        vec2 t = clamp(vec2(m + m, -n - m) * z - kx, 0.0, 1.0);
        res = min(dot2(d + (c + b * t.x) * t.x), dot2(d + (c + b * t.y) * t.y));
    }
    return sqrt(res);
}

// ---- shader-skills library: water ----
// Displace a rooted blade along the horizontal current, with zero offset and slope at its base. heightFraction is zero at the root and one at the tip. Amplitude uses world units; phase uses radians.
float currentBend(float heightFraction, float amplitude, float phase) {
    float h = clamp(heightFraction, 0.0, 1.0);
    return amplitude * h * h * (0.72 * sin(phase - 2.4*h) + 0.28 * sin(1.7*phase + 3.1*h));
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
const float CURRENT = 0.7;
const float SHAFTS = 0.8;

float kelpCurtain(vec2 p, float layer, float px) {
    float cellWidth = 0.47;
    float cover = 0.0;
    for (int k=-1; k<=1; ++k) {
        float id = floor(p.x/cellWidth)+float(k);
        vec3 h = hash32(vec2(id,layer));
        float root = (id+0.22+0.55*h.x)*cellWidth;
        float height = 0.46+1.55*h.y;
        float y = p.y+1.0;
        float t = clamp(y/height,0.0,1.0);
        float x = p.x-root-currentBend(t,0.28*CURRENT,iTime*0.65+5.0*h.z);
        float stem = abs(x)-0.009*(1.2-0.7*t);
        float row = t*12.0;
        float side = mod(floor(row),2.0)*2.0-1.0;
        float span = (0.08+0.07*sin(PI*t))*(0.9-0.5*t);
        float along = side*x/span;
        float leaf = abs(fract(row)-0.16-0.65*along-0.10*sin(along*PI))-0.20*sin(PI*clamp(along,0.0,1.0));
        float d = min(stem, max(max(-along,along-1.0)*span,leaf*height/12.0));
        cover=max(cover,1.0-smoothstep(-px,px,max(d,max(-y,y-height))));
    }
    return cover;
}

float mantaShape(vec2 p,float px) {
    float x=abs(p.x);
    float y=p.y-0.15*x*sin(iTime*1.15)-0.22*x*x;
    float wing=max(x-1.0,abs(y-0.08*x)-0.32*pow(max(1.0-x,0.0),1.55));
    float tail=sdQuadraticBezier(p,vec2(0,-0.18),vec2(0.07,-0.40),vec2(0.19,-0.69))-0.012;
    float fins=sdSegment(vec2(x,p.y),vec2(0.07,0.21),vec2(0.12,0.36))-0.022;
    return 1.0-smoothstep(-px,px,min(min(wing,tail),fins));
}

void mainImage(out vec4 fragColor,in vec2 fragCoord) {
    vec2 p=(2.0*fragCoord-iResolution.xy)/iResolution.y;
    float px=2.0/iResolution.y;
    float rays=pow(0.5+0.5*sin(9.5*p.x-3.3*p.y+1.15*sin(4.2*p.x+0.21*iTime)),8.0);
    vec3 water=mix(vec3(0.005,0.042,0.065),vec3(0.055,0.50,0.52),smoothstep(-1.3,1.2,p.y));
    vec3 col=water+SHAFTS*vec3(0.21,0.34,0.22)*rays*(0.32+0.34*p.y);
    float mask=0.0, animals=0.0;
    for(int i=0;i<4;i++){
        float z=float(i);
        vec2 q=vec2(p.x*(1.95-0.30*z)+z*3.5+iTime*(0.025+0.014*z),p.y+0.10*z);
        float gap=smoothstep(0.15,1.05,abs(p.x+0.10+0.10*sin(z)));
        q.y+=(1.0-gap)*(0.75+0.20*z);
        float blades=kelpCurtain(q,z,px*(1.95-0.30*z));
        vec3 frond=mix(vec3(0.032,0.27,0.25),vec3(0.011,0.090,0.052),z/3.0);
        col=mix(col,frond*(1.0+0.35*rays+0.15*sin(q.y*17.0)),blades);
        mask=max(mask,blades);
    }
    vec2 hero=rot2(-0.16)*(p-vec2(-0.15+0.18*sin(iTime*0.25),0.42))/0.58;
    float manta=mantaShape(hero,px/0.58);
    col=mix(col,vec3(0.021,0.10,0.13)*(1.0+0.55*exp(-6.0*abs(hero.x))),manta);
    for(int j=0;j<18;j++){
        vec3 h=hash32(vec2(float(j),31.0));
        vec2 q=(p-vec2(0.43+0.86*(h.x-0.5)+0.15*sin(iTime*0.47),0.04+0.39*(h.y-0.5)+0.024*sin(iTime*2.0+8.0*h.x)))/(0.027+0.029*h.z);
        float fish=min(length(q*vec2(1.05,3.1))-1.0,max(abs(q.y)-0.55*max(-q.x-0.5,0.0),abs(q.x+0.98)-0.36));
        animals=max(animals,1.0-smoothstep(-px*20.0,px*20.0,fish));
    }
    col=mix(col,vec3(0.075,0.26,0.25),animals);
    vec2 silt=p*22.0+vec2(0.12*iTime,-0.26*iTime);
    vec3 mote=hash32(floor(silt));
    col+=vec3(0.28,0.43,0.32)*step(0.78,mote.z)*exp(-150.0*dot2(fract(silt)-mote.xy));
#if DEBUG_MODE == 3
    col=vec3(mask,max(animals,manta),rays);
#endif
    fragColor=vec4(linearToSrgb(tonemapNeutral(col)),1.0);
}
