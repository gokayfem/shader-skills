// Alpine valley: designed lowland corridor, multiscale ridges, snow accumulation, strata and aerial depth.
// A heightfield cannot make overhangs. The controlled valley is not a drainage/erosion simulation.
// @render time=0.0
// @variant DEBUG_MODE=1
// @variant DEBUG_MODE=2
// @variant DEBUG_MODE=3
// @variant DEBUG_MODE=4
#define DEBUG_MODE 0 // 0 beauty | 1 elevation | 2 normal | 3 grass/rock/snow | 4 steps
const vec3 ALPINE_SUN=normalize(vec3(-0.65,0.42,-0.25));
const float SNOW_ALTITUDE=1.9;
const float FAR_DISTANCE=70.0;
const int ALPINE_TRACE_STEPS=320;

float alpineValleyMask(float x) { return 1.0-exp(-0.085*x*x); }

// The unit-gradient corner dots are bounded by sqrt(2); quintic interpolation is convex.
// Thus |fbm| < 2, |erodedFbm| < 2*sqrt(2), and ridgedFbm <= 1. With the height
// coefficients below, 0.31 + 23*valley bounds the full terrain before evaluating noise.
float alpineCeiling(float x) { return 0.31 + 23.0 * alpineValleyMask(x); }

float alpineHeight(vec2 p) {
    float valley=alpineValleyMask(p.x);
    float peaks=0.65+1.8*erodedFbm(p*vec2(0.30,0.25)+vec2(2.0,7.0),6)
                +0.35*ridgedFbm(p*0.2,4);
    float broad=0.5+0.5*fbm(p*0.12,4);
    float detail=0.06*fbm(p*1.8,3);
    return 0.18+(1.3+3.1*peaks+1.0*broad)*valley+detail;
}

vec2 map(vec3 p) {return vec2((p.y-alpineHeight(p.xz))*0.12,1.0);}

// Coarse-to-fine heightfield intersection. Distance-proportional sampling avoids the asymptotic
// slowdown of a small globally scaled gap near the horizon; crossings are refined by bisection.
vec3 traceAlpine(vec3 ro,vec3 rd) {
    float t=0.02,previous=0.0;
    for(int i=0;i<ALPINE_TRACE_STEPS;i++) {
        vec3 p=ro+rd*t;
        if (rd.y >= 0.0 && p.y > 23.31) break;
        if (p.y <= alpineCeiling(p.x) && p.y < alpineHeight(p.xz)) {
            float lo=previous,hi=t;
            for(int j=0;j<10;j++) {
                float mid=0.5*(lo+hi);
                vec3 q=ro+rd*mid;
                if(q.y>alpineHeight(q.xz))lo=mid;else hi=mid;
            }
            return vec3(0.5*(lo+hi),1.0,float(i));
        }
        previous=t;
        t+=0.04+0.015*t;
        if(t>FAR_DISTANCE)break;
    }
    return vec3(-1.0,0.0,float(ALPINE_TRACE_STEPS));
}

vec3 alpineSky(vec3 rd) {
    vec3 c=simpleSky(rd,ALPINE_SUN);
    // A broad cloud mask in sky-direction space; it is not a cloud volume.
    float cloud=smoothstep(0.1,0.5,fbm(rd.xz/max(rd.y+0.45,0.15)*1.2,4));
    return mix(c,vec3(0.86,0.88,0.91),cloud*0.45*smoothstep(0.05,0.4,rd.y));
}

vec3 alpineRender(vec2 coord) {
    vec2 uv=(2.0*coord-iResolution.xy)/iResolution.y;
    vec3 ro=vec3(0.15,2.1,10.0-0.25*iTime);
    ro.y=max(ro.y,alpineHeight(ro.xz)+0.7);
    vec3 rd=cameraRay(cameraMatrix(ro,vec3(0.0,1.9,-15.0),0.0),uv,1.8);
    vec3 hit=traceAlpine(ro,rd);
#if DEBUG_MODE == 4
    return debugHeat(hit.z/float(ALPINE_TRACE_STEPS));
#endif
    if(hit.x<0.0)return alpineSky(rd);
    vec3 p=ro+rd*hit.x;
    float footprint=hit.x*2.0/(iResolution.y*1.8);
    vec3 n=calcNormal(p,max(0.002,footprint*0.5));
    float snow=smoothstep(SNOW_ALTITUDE-0.3,SNOW_ALTITUDE+0.4,p.y+0.22*fbm(p.xz*2.0,3))
              *smoothstep(0.35,0.75,n.y);
    float grass=smoothstep(0.6,0.9,n.y)*(1.0-smoothstep(1.4,2.5,p.y));
    float detailKeep=1.0-smoothstep(0.02,0.2,footprint);
    float grain=0.5+0.5*fbm(p.xz*18.0,3)*detailKeep;
    float strata=0.5+0.5*sin(p.y*35.0+fbm(p.xz*3.0,3)*2.0);
    vec3 rock=mix(vec3(0.16,0.15,0.14),vec3(0.32,0.29,0.25),grain)*(0.85+0.15*strata*detailKeep);
    vec3 albedo=mix(rock,mix(vec3(0.075,0.12,0.035),vec3(0.16,0.20,0.065),grain),grass);
    albedo=mix(albedo,vec3(0.78,0.84,0.9),snow);
    float shadow=softShadow(p+n*0.03,ALPINE_SUN,0.03,15.0,12.0);
    vec3 col=shadePBR(n,-rd,ALPINE_SUN,albedo,0.8,0.0)*vec3(3.4,3.0,2.5)*shadow;
    col+=albedo*hemisphereLight(n,vec3(0.4,0.58,0.85),vec3(0.16,0.13,0.095));
#if DEBUG_MODE == 1
    return vec3(p.y/6.0);
#elif DEBUG_MODE == 2
    return n*0.5+0.5;
#elif DEBUG_MODE == 3
    return vec3(grass,1.0-grass,snow);
#endif
    vec3 haze=alpineSky(normalize(vec3(rd.x,0.04,rd.z)));
    float fog=heightFog(ro,rd,hit.x,0.011,0.18);
    return mix(col,haze,fog);
}

void mainImage(out vec4 fragColor,in vec2 fragCoord) {
    vec3 col=alpineRender(fragCoord);
#if DEBUG_MODE == 0
    col=linearToSrgb(tonemapAgX(col*1.15))+ditherTPDF(fragCoord,iFrame);
#endif
    fragColor=vec4(col,1.0);
}
