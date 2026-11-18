// A macro camera among open barrel sponges. The rims, bores and silty shelf share one distance field.
// @render time=1.0
// @variant DEBUG_MODE=2
#define DEBUG_MODE 0 // 2 normals
const float LANTERN = 1.2;
float seabed(vec2 p) { return -0.50+0.09*sin(p.x*2.0)+0.07*sin(1.8*p.y+p.x); }
vec2 shelfField(vec3 p){
    vec2 best=vec2((p.y-seabed(p.xz))*0.9,0.0);
    const float spacing=1.55;
    vec2 id=floor(p.xz/spacing);
    vec3 h=hash32(id);
    vec2 root=(id+0.42+0.16*h.xy)*spacing;
    float y=p.y-seabed(root);
    float height=0.43+1.18*h.z, t=clamp(y/height,0.0,1.0);
    vec2 q=p.xz-root-vec2(0.13,-0.055)*t*t;
    float angle=atan(q.y,q.x+1e-7);
    float radius=(0.16+0.15*h.y)*(0.62+0.42*t)+0.088*sin(PI*t);
    radius+=0.018*sin(7.0*angle+2.2*t)*smoothstep(0.08,0.24,length(q));
    float rim=height+0.048*sin(3.0*angle+4.0*h.x)+0.023*sin(7.0*angle);
    float radial=length(q);
    float outer=max(radial-radius,max(-y,y-rim));
    float inner=max(radial-(radius-0.043),0.12-y);
    float d=0.56*max(outer,-inner);
    if(d<best.x) best=vec2(d,1.0+h.x);
    // Every barrel fits at least 0.07 units inside its tile. A positive guard also lets rays cross tile seams.
    vec2 edge=0.5*spacing-abs(mod(p.xz,spacing)-0.5*spacing);
    best.x=min(best.x,min(edge.x,edge.y)+0.07);
    return best;
}
void mainImage(out vec4 fragColor,in vec2 fragCoord) {
    vec2 uv=(2.0*fragCoord-iResolution.xy)/iResolution.y;
    vec3 ro=vec3(1.12,1.10,3.23+0.12*sin(0.32*iTime));
    vec3 rd=cameraRay(cameraMatrix(ro,vec3(0.65,0.42,0.0),0.04),uv,1.65);
    vec3 water=mix(vec3(0.012,0.022,0.065),vec3(0.034,0.17,0.19),smoothstep(-0.38,0.5,rd.y));
    vec3 col=water,n=vec3(0.0);
    float t=0.03;
    for(int i=0;i<85;i++){
        vec3 p=ro+t*rd;
        vec2 hit=shelfField(p);
        if(hit.x<0.0015+0.0003*t){
            vec2 e=vec2(0.002,0.0);
            n=normalize(vec3(shelfField(p+e.xyy).x-shelfField(p-e.xyy).x,
                             shelfField(p+e.yxy).x-shelfField(p-e.yxy).x,
                             shelfField(p+e.yyx).x-shelfField(p-e.yyx).x));
            vec3 light=normalize(vec3(-0.55,0.75,0.32));
            float grain=0.5+0.5*gradientNoise(p*14.0);
            vec3 base=mix(vec3(0.05,0.07,0.12),vec3(0.14,0.11,0.16),grain);
            if(hit.y>0.5){
                vec2 root=(floor(p.xz/1.55)+0.42+0.16*hash32(floor(p.xz/1.55)).xy)*1.55;
                vec2 surface=vec2(atan(p.z-root.y,p.x-root.x+1e-7)*3.82,p.y*13.0);
                surface.x+=0.5*mod(floor(surface.y),2.0);
                vec2 pore=fract(surface)-0.5;
                float pores=1.0-smoothstep(0.10,0.28,length(pore*vec2(1.0,0.72)));
                base=mix(vec3(0.19,0.065,0.26),vec3(0.63,0.19,0.21),hit.y-1.0)*(0.6+0.4*grain)*(1.0-0.4*pores);
                base+=LANTERN*vec3(0.62,0.10,0.25)*pow(max(n.y,0.0),6.0);
            }
            float caustic=pow(0.5+0.5*sin(9.0*p.x+2.3*sin(5.0*p.z+iTime*0.4)),8.0);
            col=base*(0.25+1.25*max(dot(n,light),0.0))*(1.0+0.35*caustic);
            col+=vec3(0.025,0.13,0.15)*pow(1.0-max(dot(-rd,n),0.0),3.0);
            col=mix(water,col,beerTransmittance(vec3(0.19,0.13,0.11),t));
            break;
        }
        t+=max(hit.x,0.001);
        if(t>10.0)break;
    }
#if DEBUG_MODE == 2
    col=0.5+0.5*n;
#endif
    fragColor=vec4(linearToSrgb(tonemapNeutral(col)),1.0);
}
