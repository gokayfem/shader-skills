// Transparent sea gooseberry study. Two visible interfaces and eight longitudinal ciliary rows.
// @render time=2.5
// @variant DEBUG_MODE=3
#define DEBUG_MODE 0 // 3 front and back rows and optical thickness

// Library code below is bundled from shader-skills (MIT): https://github.com/gokayfem/shader-skills
// ---- shader-skills library: common ----
const float PI  = 3.14159265358979;
const float TAU = 6.28318530717959;
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

// ---- shader-skills library: intersect ----
// Ray vs sphere centered at c with radius r. Returns vec2(tNear, tFar), or vec2(-1) on a miss. Uses the precision-friendly form that measures from the closest point on the ray.
vec2 iSphere(vec3 ro, vec3 rd, vec3 c, float r) {
    vec3 oc = ro - c;
    float b = dot(oc, rd);
    vec3 qc = oc - b * rd;
    float h = r * r - dot(qc, qc);
    if (h < 0.0) return vec2(-1.0);
    h = sqrt(h);
    return vec2(-b - h, -b + h);
}
// Ray interval for an axis-aligned ellipsoid at the origin with positive radii. Returns (-1,-1) on a miss.
vec2 iEllipsoid(vec3 ro, vec3 rd, vec3 radii) {
    vec3 v = rd / radii;
    float speed = length(v);
    vec2 t = iSphere(ro / radii, v / speed, vec3(0.0), 1.0);
    if (t.x == -1.0 && t.y == -1.0) return vec2(-1.0);
    return t / speed;
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
const vec3 RADII=vec3(0.56,0.77,0.50);
const float KEY_LIGHT=1.0;
const float BEAT_SPEED=2.6;
const float PLATES=32.0;

vec3 waterField(vec2 p) {
    vec3 water=mix(vec3(0.002,0.010,0.028),vec3(0.011,0.086,0.095),exp(-1.7*length(p-vec2(-0.7,0.65))));
    for(int layer=0;layer<3;layer++){
        vec2 q=p*(8.0+float(layer)*6.0)+vec2(float(layer)*4.3,0.035*iTime);
        vec2 id=floor(q);
        vec3 h=hash32(id);
        float d=length(fract(q)-mix(vec2(0.1),vec2(0.9),h.xy));
        water+=vec3(0.10,0.24,0.25)*step(0.80,h.z)*exp(-d*d*(180.0+200.0*h.z));
    }
    return water;
}

vec4 combSurface(vec3 unitPos,float px) {
    float lat=asin(clamp(unitPos.y,-1.0,1.0));
    float lon=atan(unitPos.z,unitPos.x);
    float tracks=lon*8.0/TAU+0.15*sin(2.0*lat);
    float crossRow=abs(fract(tracks+0.5)-0.5)*max(0.12,cos(lat));
    float row=1.0-smoothstep(0.022,0.055+px,crossRow);
    float beat=lat*8.0-iTime*BEAT_SPEED+0.55*cos(lon);
    float phase=lat*PLATES/PI+0.18*sin(beat);
    float plate=0.5+0.5*cos(TAU*phase);
    plate=mix(0.5,plate,1.0-smoothstep(0.22,0.5,px*PLATES/max(0.12,cos(lat))));
    float gate=1.0-smoothstep(1.03,1.38,abs(lat));
    vec3 spectrum=0.5+0.5*cos(beat+vec3(0.0,2.1,4.2)+1.3*sin(lon));
    vec3 n=normalize(unitPos/RADII);
    float lamp=0.16+0.84*max(dot(n,normalize(vec3(-0.55,0.65,0.65))),0.0);
    float strength=KEY_LIGHT*gate*row*plate*lamp;
    return vec4((0.09+spectrum*spectrum)*strength*2.5,strength);
}

void mainImage(out vec4 fragColor,in vec2 fragCoord){
    vec2 uv=(2.0*fragCoord-iResolution.xy)/iResolution.y;
    float px=2.0/iResolution.y;
    vec2 q=rot2(-0.35)*(uv-vec2(0.03,0.12+0.035*sin(iTime*0.5)));
    vec3 col=waterField(uv);
    // Two trailing tentacles with fine lateral branches. Draw behind the transparent body.
    for(int i=0;i<2;i++){
        float side=(i==0)?-1.0:1.0;
        float along=max(-q.y-0.24,0.0);
        float x=side*(0.22+0.18*along)+0.12*sin(5.0*along+iTime*0.6+side)*along;
        float dist=abs(q.x-x);
        float stem=1.0-smoothstep(0.002,0.003+px,dist);
        float twig=abs(fract(along*27.0+(q.x-x)*side*16.0)-0.5);
        float fringe=(1.0-smoothstep(0.03,0.15,twig))*(1.0-smoothstep(0.0,0.07,dist));
        col+=vec3(0.12,0.34,0.34)*(stem+0.42*fringe)*smoothstep(0.0,0.07,along);
    }
    vec3 ro=vec3(q,2.0), rd=vec3(0,0,-1);
    ro.xz=rot2(0.21)*ro.xz;
    rd.xz=rot2(0.21)*rd.xz;
    float angle=0.38+0.14*sin(0.4*iTime);
    ro.yz=rot2(angle)*ro.yz;
    rd.yz=rot2(angle)*rd.yz;
    vec2 hit=iEllipsoid(ro,rd,RADII);
    if(hit.y>0.0){
        vec3 front=(ro+rd*hit.x)/RADII, back=(ro+rd*hit.y)/RADII;
        vec3 n=normalize(front/RADII);
        float path=hit.y-hit.x;
        float rim=pow(1.0-max(dot(n,-rd),0.0),2.2);
        vec4 nearRows=combSurface(front,px), farRows=combSurface(back,px);
        vec3 transmission=exp(-path*vec3(0.16,0.10,0.085));
        vec3 refracted=waterField(uv-0.014*n.xy*path);
        col=mix(col,refracted,0.67)*transmission+farRows.rgb*transmission*0.38+nearRows.rgb;
        float gut=exp(-55.0*abs(q.x-0.035*sin(q.y*7.0)))*(1.0-smoothstep(0.12,0.58,abs(q.y)));
        col+=vec3(0.23,0.095,0.055)*gut+vec3(0.012,0.082,0.081)*path;
        col+=vec3(0.20,0.57,0.53)*rim;
#if DEBUG_MODE == 3
        col=vec3(nearRows.a,farRows.a,path*0.6);
#endif
    }
    fragColor=vec4(linearToSrgb(tonemapNeutral(col)),1.0);
}
