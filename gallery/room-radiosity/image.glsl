#define ROOM_RADIOSITY 1
// A warm ceiling panel lights a plaster sphere in a blue and terracotta room.
// @render time=2.0
// @variant DEBUG_MODE=6
#define DEBUG_MODE 0 // 6 direct, left and right bounce contributions

// Library code below is bundled from shader-skills (MIT): https://github.com/gokayfem/shader-skills
// ---- shader-skills library: common ----
const float PI  = 3.14159265358979;
const float TAU = 6.28318530717959;

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

// ---- shader-skills library: shading ----
// Approximate diffuse form factor from a small, oriented patch to a surface point. Both normals point out of their surfaces. Area is in squared world units. The finite area softens near-field error.
float diffusePatchWeight(vec3 point, vec3 normal, vec3 center, vec3 patchNormal, float area) {
    vec3 delta = center - point;
    float distance2 = dot(delta, delta);
    vec3 direction = delta * inversesqrt(max(distance2, 1e-10));
    float coupling = max(dot(normal, direction), 0.0) * max(dot(patchNormal, -direction), 0.0);
    return coupling * max(area, 0.0) / max(PI * distance2 + max(area, 0.0), 1e-10);
}

// ---- shader-skills library: color ----
// Exact sRGB encode (OETF): linear -> display [0,1]. Apply exactly once, after tonemapping. pow(c, 1/2.2) is visibly wrong near black (0.001 encodes to 0.013 here vs 0.043 with 2.2).
vec3 linearToSrgb(vec3 c) {
    c = max(c, 0.0);
    return mix(c * 12.92, 1.055 * pow(c, vec3(1.0 / 2.4)) - 0.055, step(0.0031308, c));
}

// ---- shader-skills library: tonemap ----
// AgX: log encoding plus a sigmoid inside an inset/outset gamut. Mid-grey 0.18 maps to about 0.5 display. Minimal implementation after Troy Sobotka's AgX and Benjamin Wrensch's polynomial fit (both MIT).
vec3 tonemapAgX(vec3 c) {
    const mat3 inset = mat3(0.842479062253094, 0.0423282422610123, 0.0423756549057051,
                            0.0784335999999992, 0.878468636469772, 0.0784336,
                            0.0792237451477643, 0.0791661274605434, 0.879142973793104);
    const mat3 outset = mat3(1.19687900512017, -0.0528968517574562, -0.0529716355144438,
                             -0.0980208811401368, 1.15190312990417, -0.0980434501171241,
                             -0.0990297440797205, -0.0989611768448433, 1.15107367264116);
    const float minEv = -12.47393, maxEv = 4.026069;
    c = inset * max(c, 0.0);
    c = clamp((log2(max(c, 1e-10)) - minEv) / (maxEv - minEv), 0.0, 1.0);
    vec3 x2 = c * c, x4 = x2 * x2;
    c = 15.5 * x4 * x2 - 40.14 * x4 * c + 31.96 * x4 - 6.868 * x2 * c + 0.4298 * x2 + 0.1191 * c - 0.00232;
    c = outset * c;
    return pow(max(c, 0.0), vec3(2.2));
}

// ---- shader ----
const float BOUNCE=1.0;
const float PANEL_X=0.15;
const vec3 ORB=vec3(-0.23,-0.39,-0.29);
const float RADIUS=0.60;

bool clearSegment(vec3 a,vec3 b){
    vec3 delta=b-a;
    float t=clamp(dot(ORB-a,delta)/max(dot(delta,delta),1e-7),0.0,1.0);
    return length(a+t*delta-ORB)>RADIUS-0.002;
}
vec3 panelLight(vec3 p,vec3 n){
    vec3 sum=vec3(0.0);
    for(int i=0;i<4;i++){
        vec3 source=vec3(PANEL_X+0.25*(float(i%2)-0.5),1.29,-0.50+0.5*(float(i/2)-0.5));
        if(clearSegment(p+0.003*n,source))
            sum+=vec3(11.0,8.8,6.7)*diffusePatchWeight(p,n,source,vec3(0,-1,0),0.125);
    }
    return sum;
}
vec3 wallPigment(float side){
    return side<0.0 ? vec3(0.82,0.135,0.044) : vec3(0.035,0.40,0.62);
}
vec3 wallBounce(vec3 p,vec3 n,float side){
    vec3 sum=vec3(0.0);
    vec3 inward=vec3(-side,0,0);
    for(int j=0;j<3;j++) for(int i=0;i<3;i++){
        vec3 surfacePoint=vec3(side*1.4,-1.0+(float(i)+0.5)*2.3/3.0,-1.6+(float(j)+0.5)*3.1/3.0);
        if(clearSegment(p+0.003*n,surfacePoint))
            sum+=panelLight(surfacePoint,inward)*diffusePatchWeight(p,n,surfacePoint,inward,2.3*3.1/9.0);
    }
    return wallPigment(side)*sum;
}
// Five inward-facing room surfaces, 4x4 diffuse patches per surface.
// The sphere participates in visibility and receives bounced light.
void roomPatch(ivec2 index,out vec3 p,out vec3 n,out vec3 albedo,out float area){
    int face=index.x/4;
    vec2 uv=(vec2(index.x%4,index.y)+0.5)/4.0;
    vec3 v=vec3(mix(-1.4,1.4,uv.x),mix(-1.0,1.3,uv.y),mix(-1.6,1.5,uv.x));
    if(face<2){
        float side=face==0?-1.0:1.0;
        p=vec3(1.4*side,v.y,v.z);n=vec3(-side,0,0);
        albedo=wallPigment(side);area=2.3*3.1/16.0;
    }else if(face<4){
        float side=face==2?-1.0:1.0;
        p=vec3(v.x,side<0.0?-1.0:1.3,mix(-1.6,1.5,uv.y));n=vec3(0,-side,0);
        albedo=vec3(0.72);area=2.8*3.1/16.0;
    }else{
        p=vec3(v.x,v.y,-1.6);n=vec3(0,0,1);
        albedo=vec3(0.73,0.70,0.64);area=2.8*2.3/16.0;
    }
}
vec3 gatherRoom(vec3 p,vec3 n) {
    vec3 incoming=vec3(0.0);
    float totalWeight=0.0;
    for(int y=0;y<4;y++) for(int x=0;x<20;x++){
        ivec2 id=ivec2(x,y);
        vec3 center,normal,albedo;float area;
        roomPatch(id,center,normal,albedo,area);
        float weight=diffusePatchWeight(p,n,center,normal,area);
        if(weight>0.0 && clearSegment(p+0.003*n,center)){
            incoming+=weight*texelFetch(iChannel0,id,0).rgb;
            totalWeight+=weight;
        }
    }
    return incoming/max(1.0,totalWeight);
}
void mainImage(out vec4 fragColor,in vec2 fragCoord){
    vec2 uv=(2.0*fragCoord-iResolution.xy)/iResolution.y;
    vec3 ro=vec3(0.05,0.08,2.72);
    vec3 rd=cameraRay(cameraMatrix(ro,vec3(0,-0.12,-0.44),0.0),uv,1.55);
    // The camera looks into a five-sided room. Find the nearest forward plane.
    float t=(-1.6-ro.z)/rd.z;
    vec3 n=vec3(0,0,1),base=vec3(0.73,0.70,0.64);
    if(abs(rd.x)>1e-6){
        float side=sign(rd.x), wall=(side*1.4-ro.x)/rd.x;
        if(wall<t){ t=wall; n=vec3(-side,0,0); base=wallPigment(side); }
    }
    if(abs(rd.y)>1e-6){
        float plane=rd.y<0.0 ? -1.0 : 1.3;
        float wall=(plane-ro.y)/rd.y;
        if(wall<t){ t=wall; n=vec3(0,-sign(rd.y),0); base=vec3(0.72); }
    }
    vec2 sphere=iSphere(ro,rd,ORB,RADIUS);
    if(sphere.x>0.0 && sphere.x<t){
        t=sphere.x;n=normalize(ro+t*rd-ORB);base=vec3(0.84,0.81,0.73);
    }
    vec3 p=ro+t*rd;
    vec3 direct=panelLight(p,n);
#ifdef ROOM_RADIOSITY
    vec3 indirect=gatherRoom(p,n);
#else
    vec3 left=wallBounce(p,n,-1.0),right=wallBounce(p,n,1.0);
    vec3 indirect=left+right;
#endif
    base*=0.97+0.035*gradientNoise(p*34.0);
    vec3 col=base*(vec3(0.027)+direct+BOUNCE*indirect);
    if(n.y < -0.99 && abs(p.x-PANEL_X)<0.25 && abs(p.z+0.5)<0.5) col=vec3(4.0,3.4,2.4);
#if DEBUG_MODE == 6
#ifdef ROOM_RADIOSITY
    col=4.0*indirect;
#else
    col=vec3(length(direct)*0.35,length(left)*2.0,length(right)*2.0);
#endif
#endif
    fragColor=vec4(linearToSrgb(tonemapAgX(col*1.7)),1.0);
}
