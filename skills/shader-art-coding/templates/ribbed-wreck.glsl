// A broken wooden hull embedded in a sand bank. Orbit the wreck or dive closer with the camera controls.
// @render time=3.2
// @variant DEBUG_MODE=4
#define DEBUG_MODE 0 // 4 primary trace cost
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
