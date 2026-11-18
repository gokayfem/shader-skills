// Coastal observatory with a weather front, animated tide, moon terminator and layered night sky.
// @render time=1.0
// @variant DEBUG_MODE=3
#define DEBUG_MODE 0 // 3 cloud coverage, moon illumination, land silhouette
const float HOUR=19.7;
const float OVERCAST=0.32;
const float MOON_PHASE=0.18;

float landHeight(float x){
    return -0.44+0.14*sin(2.1*x+0.7)+0.06*sin(7.2*x)+0.035*fbm(vec2(x*7.0,2.4),4);
}
void mainImage(out vec4 fragColor,in vec2 fragCoord){
    vec2 p=(2.0*fragCoord-iResolution.xy)/iResolution.y;
    float px=2.0/iResolution.y;
    float angle=TAU*(HOUR-6.0)/24.0;
    float elevation=sin(angle);
    float daylight=smoothstep(-0.24,0.16,elevation);
    float dusk=exp(-8.0*abs(elevation+0.10));
    vec3 sky=mix(vec3(0.007,0.012,0.045),vec3(0.09,0.37,0.65),daylight);
    vec3 horizon=mix(vec3(0.055,0.09,0.15),vec3(0.63,0.79,0.84),daylight)+dusk*vec3(1.05,0.31,0.095);
    vec3 col=mix(horizon,sky,smoothstep(-0.35,1.05,p.y));
    vec2 sun=vec2(-0.65*cos(angle),-0.22+1.23*elevation);
    col+=vec3(1.7,1.05,0.53)*(exp(-9.0*length(p-sun))*0.8+1.0-smoothstep(0.061,0.061+px,length(p-sun)));
    vec2 star=p*85.0;
    vec3 h=hash32(floor(star));
    float galaxy=exp(-7.0*abs(p.y-0.33*p.x-0.46))*smoothstep(-0.3,0.6,fbm(p*5.0,4));
    col+=(1.0-daylight)*(galaxy*vec3(0.16,0.092,0.15)+step(0.955,h.z)*exp(-39.0*dot2(fract(star)-h.xy))*0.9);
    vec2 moon=(p-vec2(-0.62,0.44))/0.13;
    float disk=1.0-smoothstep(1.0-px/0.13,1.0+px/0.13,length(moon));
    float phase=TAU*MOON_PHASE;
    float lunar=lunarDiskLight(moon,vec3(sin(phase),0.0,-cos(phase)));
    col=mix(col,mix(sky,vec3(0.95,0.93,0.79),lunar)*(0.94+0.06*gradientNoise(moon*15.0)),disk*(1.0-0.8*daylight));
    vec2 cloudUV=vec2(1.35*p.x+0.028*iTime,4.2*p.y);
    float density=fbm(cloudUV,5);
    float cloud=smoothstep(0.40-0.82*OVERCAST,0.70-0.82*OVERCAST,density);
    col=mix(col,mix(vec3(0.029,0.047,0.10),vec3(0.74,0.78,0.80),daylight)+dusk*vec3(0.38,0.12,0.08),cloud*0.88);
    float land=landHeight(p.x), shore=1.0-smoothstep(land-px,land+px,p.y);
    col=mix(col,vec3(0.019,0.038,0.046)*(0.6+daylight),shore);
    float ground=landHeight(0.62);
    vec2 q=p-vec2(0.62,ground);
    float tower=max(abs(q.x)-(0.058-0.026*clamp(q.y/0.33,0.0,1.0)),abs(q.y-0.15)-0.18);
    float lantern=max(abs(q.x)-0.047,abs(q.y-0.35)-0.03);
    float roof=max(abs(q.x)*0.8+q.y-0.423,0.378-q.y);
    col=mix(col,vec3(0.064,0.065,0.066),1.0-smoothstep(-px,px,min(tower,roof)));
    col=mix(col,vec3(2.8,1.2,0.34),1.0-smoothstep(-px,px,lantern));
    col+=vec3(0.8,0.24,0.055)*exp(-23.0*length(q-vec2(0,0.35)))*(1.0-daylight);
    float tide=-0.70+0.025*sin(iTime*0.3);
    if(p.y<tide){
        float wave=sin(72.0*p.y+5.0*sin(3.0*p.x+iTime)+0.6*iTime);
        col=mix(vec3(0.012,0.036,0.058),horizon,0.07+0.045*wave);
        col+=vec3(0.58,0.24,0.09)*exp(-7.0*abs(p.x-0.62+0.045*wave))*pow(0.5+0.5*wave,6.0);
    }
#if DEBUG_MODE == 3
    col=vec3(cloud,lunar*disk,shore);
#endif
    fragColor=vec4(linearToSrgb(tonemapNeutral(col)),1.0);
}
