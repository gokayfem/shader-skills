// Morpho in a forest clearing. Wing outline, venation, melanin margin and scale reflectance are independent.
// @render time=2.0
// @variant DEBUG_MODE=3
#define DEBUG_MODE 0 // 3 scale return, pigment margin and vein masks
const float SCALE_DENSITY = 105.0;
const float LAYER_PERIOD_NM = 172.0;
const float WING_BEAT = 0.62;

// Track minimum squared distance and even-odd crossings along a cubic outline.
void wingArc(vec2 p, vec2 a, vec2 b, vec2 c, vec2 d, inout float dist2, inout float side) {
    vec2 prev = a;
    for (int i=1; i<=9; ++i) {
        float t = float(i)/9.0;
        vec2 q = mix(mix(mix(a,b,t),mix(b,c,t),t),mix(mix(b,c,t),mix(c,d,t),t),t);
        vec2 edge = q-prev;
        vec2 diff = p-prev;
        vec2 delta = diff-edge*clamp(dot(diff,edge)/max(dot(edge,edge),1e-9),0.0,1.0);
        dist2 = min(dist2,dot(delta,delta));
        if ((q.y>p.y)!=(prev.y>p.y)) {
            if (p.x < prev.x+edge.x*diff.y/edge.y) side = -side;
        }
        prev=q;
    }
}

float wingOutline(vec2 p, bool fore) {
    vec2 root = vec2(0.025,0.06);
    float d=10.0, side=1.0;
    if (fore) {
        vec2 tip=vec2(1.30,0.67), corner=vec2(1.14,-0.08);
        wingArc(p,root,vec2(0.28,0.40),vec2(0.95,0.82),tip,d,side);
        wingArc(p,tip,vec2(1.42,0.56),vec2(1.33,0.12),corner,d,side);
        wingArc(p,corner,vec2(0.84,-0.27),vec2(0.31,-0.17),root,d,side);
    } else {
        vec2 outer=vec2(1.02,-0.22), tail=vec2(0.47,-0.76);
        wingArc(p,root,vec2(0.34,0.11),vec2(0.94,0.08),outer,d,side);
        wingArc(p,outer,vec2(1.04,-0.46),vec2(0.81,-0.73),tail,d,side);
        wingArc(p,tail,vec2(0.21,-0.78),vec2(0.065,-0.38),root,d,side);
    }
    return side*sqrt(d);
}

vec3 wingColor(vec2 p,float outline,bool fore,float tilt,float px) {
    vec2 fromBase=p-vec2(0.015,0.07);
    float r=length(fromBase);
    float a=atan(fromBase.y,fromBase.x)+(fore?0.0:0.15);
    float phase=(a+0.15*r+0.025*sin(6.0*r))* (fore?7.4:6.0);
    float halfWidth=mix(0.008,0.002,smoothstep(0.0,1.1,r));
    float veinDist=abs(fract(phase+0.5)-0.5)*max(r,0.02)/(fore?7.4:6.0);
    float vein=1.0-smoothstep(halfWidth,halfWidth+px,veinDist);
    // Secondary branches split from the radial veins near the leading edge.
    float branch=abs(fract(phase+0.5+0.56*smoothstep(0.51,1.26,r))-0.5)*max(r,0.02)/7.4;
    vein=max(vein,0.68*(1.0-smoothstep(0.002,0.002+px,branch))*smoothstep(0.55,0.82,r));
    float margin=1.0-smoothstep(0.021,0.105,-outline+0.017*sin(a*38.0));
    vec2 tile=vec2(r*0.85,a*max(r,0.06))*SCALE_DENSITY;
    tile.x+=0.5*mod(floor(tile.y),2.0);
    float seed=hash12(floor(tile));
    float scales=(seed-0.5)*(1.0-smoothstep(0.25,1.0,px*SCALE_DENSITY));
    float bend=tilt+0.30*sin(3.0*p.x+2.0*p.y)+0.095*sin(phase*TAU)+0.18*scales;
    float nm=braggPeakNm(LAYER_PERIOD_NM,1.48,cos(bend));
    float cyan=smoothstep(450.0,501.0,nm);
    vec3 reflected=mix(vec3(0.018,0.08,0.83),vec3(0.012,0.72,0.87),cyan);
    float flash=exp(-3.2*bend*bend)*(0.70+0.5*scales);
    vec3 pigment=vec3(0.023,0.011,0.009)*(1.0+0.5*scales);
    vec3 color=mix(pigment+reflected*(0.19+1.75*flash),pigment,margin);
    float spots=exp(-130.0*abs(-outline-0.042))*pow(0.5+0.5*cos(phase*TAU),8.0);
    color+=vec3(0.55,0.49,0.36)*spots*smoothstep(0.65,1.14,r);
#if DEBUG_MODE == 3
    return vec3(flash,margin,vein);
#endif
    return mix(color,pigment,vein*0.88);
}

void mainImage(out vec4 fragColor,in vec2 fragCoord) {
    vec2 uv=(2.0*fragCoord-iResolution.xy)/iResolution.y;
    float px=2.0/iResolution.y;
    float blur=fbm(uv*2.3,3);
    vec3 col=mix(vec3(0.008,0.029,0.024),vec3(0.13,0.22,0.105),smoothstep(-0.45,0.45,blur));
    for(int i=0;i<18;i++){
        vec3 h=hash32(vec2(float(i),17.0));
        float d=length(uv-(h.xy-0.5)*vec2(4.0,2.5));
        col+=vec3(0.12,0.14,0.065)*(1.0-smoothstep(0.035+0.09*h.z,0.13+0.09*h.z,d));
    }
    vec2 q=rot2(-0.13)*(uv+vec2(0.03,-0.04));
    float beat=0.42+0.40*sin(WING_BEAT*iTime);
    vec2 p=vec2(abs(q.x)/cos(0.51*beat),q.y);
    float hind=wingOutline(p,false), fore=wingOutline(p,true);
    if (hind<px) col=mix(col,wingColor(p,hind,false,beat+0.28,px),1.0-smoothstep(-px,px,hind));
    if (fore<px) col=mix(col,wingColor(p,fore,true,beat+sign(q.x)*0.20,px),1.0-smoothstep(-px,px,fore));
    float antenna=sdQuadraticBezier(vec2(abs(q.x),q.y),vec2(0.012,0.21),vec2(0.085,0.41),vec2(0.19,0.46));
    float body=min(sdEllipse(q-vec2(0.0,-0.04),vec2(0.044,0.27)),length(q-vec2(0,0.20))-0.054);
    col=mix(col,vec3(0.024,0.014,0.009)*(1.0+0.5*sin(q.y*150.0)),1.0-smoothstep(-px,px,min(antenna-0.004,body)));
    fragColor=vec4(linearToSrgb(tonemapNeutral(col)),1.0);
}
