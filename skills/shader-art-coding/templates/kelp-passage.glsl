// Moving through staggered curtains of kelp below a shoal and a gliding manta.
// @render time=3.0
// @variant DEBUG_MODE=3
#define DEBUG_MODE 0 // 3 fronds, wildlife, shafts
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
