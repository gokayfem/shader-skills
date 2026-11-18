// Alpine firs drawn as bounded clusters. Subpixel branches collapse into a crown silhouette.
// @render time=2.0
// @variant DEBUG_MODE=4
#define DEBUG_MODE 0 // 4 warm = explicit boughs, blue = crown proxy
const float ERROR_PX=1.5;
const float WIND=0.55;

vec2 fir(vec2 p,float px,float errorPx,float seed){
    float y=p.y;
    p.x-=currentBend(y,0.105*WIND,iTime*0.62+seed);
    float crown=max(abs(p.x)-0.32*(1.0-y),max(-y,y-1.0));
    float trunk=max(abs(p.x)-0.029,abs(y+0.035)-0.10);
    float detail=smoothstep(ERROR_PX,2.0*ERROR_PX,errorPx);
    float d=crown;
    if(detail>0.0 && y<1.04 && y>-0.04 && abs(p.x)<0.42){
        float bough=1.0;
        for(int k=0;k<6;k++){
            float low=0.14*float(k);
            float h=0.31-0.017*float(k);
            float w=0.32*(1.0-low);
            float u=(y-low)/h;
            float skirt=low+0.07*(1.0-abs(p.x)/max(w,0.01));
            bough=min(bough,max(abs(p.x)-w*(1.0-u),max(skirt-y,y-low-h)));
        }
        d=mix(crown,bough,detail);
    }
    return vec2(1.0-smoothstep(-px,px,min(d,trunk)),detail);
}
void mainImage(out vec4 fragColor,in vec2 fragCoord){
    vec2 uv=(2.0*fragCoord-iResolution.xy)/iResolution.y;
    vec3 sky=mix(vec3(1.15,0.47,0.22),vec3(0.14,0.28,0.50),smoothstep(-0.4,0.85,uv.y));
    vec2 solar=uv-vec2(-0.58,0.22);
    vec3 col=sky+vec3(1.2,0.72,0.3)*(exp(-4.5*length(solar))+1.0-smoothstep(0.125,0.131,length(solar)));
    float px=2.0/iResolution.y;
    for(int layer=0;layer<7;layer++){
        float nearLevel=float(layer)/6.0;
        float z=mix(55.0,2.8,pow(nearLevel,0.56));
        float scale=1.5/z;
        float x=uv.x/scale+float(layer)*16.31+iTime*0.065;
        float id=floor(x/0.58);
        float ground=-0.05-0.77*nearLevel+0.13*sin(uv.x*(2.0+nearLevel)+float(layer)*1.78)
                    +0.055*gradientNoise(vec2(uv.x*4.0,float(layer)));
        float forest=1.0-smoothstep(ground-px,ground+px,uv.y),refined=0.0;
        for(int j=-1;j<=1;j++){
            float tree=id+float(j);
            vec3 seed=hash32(vec2(tree,float(layer)));
            float height=0.65+0.71*seed.y;
            vec2 local=vec2(x-(tree+0.24+0.40*seed.x)*0.58,(uv.y-ground)/scale)/height;
            vec2 treeMask=fir(local,px/(scale*height),projectedErrorPx(0.042*height,z,1.5,iResolution.y),6.28*seed.z);
            forest=max(forest,treeMask.x);
            refined=max(refined,treeMask.x*treeMask.y);
        }
        vec3 woodland=mix(vec3(0.34,0.27,0.34),vec3(0.012,0.042,0.053),pow(nearLevel,0.75));
        woodland+=vec3(0.10,0.055,0.045)*(1.0-nearLevel)*exp(-2.4*abs(uv.x+0.58));
#if DEBUG_MODE == 4
        woodland=mix(vec3(0.026,0.17,0.46),vec3(0.97,0.35,0.055),refined);
#endif
        col=mix(col,woodland,forest);
    }
    fragColor=vec4(linearToSrgb(tonemapNeutral(col)),1.0);
}
