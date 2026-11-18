// Two-ink relief print of potential flow around a circular cylinder. No simulated wake.
// @render time=2.0
// @variant DEBUG_MODE=2
// @variant QUALITY=1
#define DEBUG_MODE 0 // 2 velocity direction and speed
#define QUALITY 2    // 1 eight taps per side, 2 sixteen
const float RADIUS=0.37;
const vec2 CENTER=vec2(-0.37,0.06);

vec2 flow(vec2 p) {
    float r2=max(dot(p,p),RADIUS*RADIUS);
    float scale=RADIUS*RADIUS/(r2*r2);
    return vec2(1.0,0.0)-scale*vec2(p.x*p.x-p.y*p.y,2.0*p.x*p.y);
}
vec2 direction(vec2 p) {
    vec2 v=flow(p);
    return v/max(length(v),0.0001);
}
float paperNoise(vec2 p,float frequency) {
    return valueNoiseD(p*frequency).x;
}
float streaks(vec2 p,float px) {
#if QUALITY == 1
    const int TAPS=8;
#else
    const int TAPS=16;
#endif
    float h=0.095/float(TAPS), freq=min(170.0,0.45/px);
    float total=paperNoise(p,freq), weights=1.0;
    for(int signIndex=0;signIndex<2;signIndex++){
        vec2 s=p;
        float stepSize=signIndex==0?h:-h;
        for(int j=1;j<=TAPS;j++){
            vec2 mid=s+0.5*stepSize*direction(s);
            s+=stepSize*direction(mid);
            if(dot(s,s)<RADIUS*RADIUS) break;
            float w=0.5+0.5*cos(PI*float(j)/float(TAPS+1));
            total+=w*paperNoise(s,freq);
            weights+=w;
        }
    }
    return total/weights;
}
void mainImage(out vec4 fragColor,in vec2 fragCoord) {
    vec2 p=rot2(-0.095)*((2.0*fragCoord-iResolution.xy)/iResolution.y-CENTER);
    float px=2.0/iResolution.y, r2=max(dot(p,p),RADIUS*RADIUS);
    float psi=p.y*(1.0-RADIUS*RADIUS/r2);
    float potential=p.x*(1.0+RADIUS*RADIUS/r2);
    vec2 v=flow(p);
    vec3 paper=vec3(0.87,0.77,0.54), ink=vec3(0.018,0.092,0.11), vermilion=vec3(0.67,0.105,0.045);
    vec3 col=paper;
    float distanceToBody=length(p)-RADIUS;
    if(distanceToBody>-px){
        float lines=streaks(p,px);
        float grain=smoothstep(0.42,0.61,lines);
        float contour=filteredStripes(28.0*psi,28.0*px*max(length(v),0.01));
        float tracer=0.5+0.5*cos(9.0*potential-1.9*iTime+4.0*psi);
        float ribbon=1.0-smoothstep(0.018,0.050,abs(psi-0.065));
        col=mix(paper,ink,clamp(0.80*grain+0.13*contour,0.0,1.0));
        col=mix(col,vermilion,ribbon*(0.50+0.40*tracer));
    }
    float interior=1.0-smoothstep(-px,px,distanceToBody);
    vec3 lacquer=mix(ink,vermilion,0.76+0.19*p.y/RADIUS);
    col=mix(col,lacquer,interior);
    col*=0.96+0.07*hash12(fragCoord);
#if DEBUG_MODE == 2
    col=mix(vec3(0.5+0.22*v,0.28*length(v)),vec3(0.03),interior);
#endif
    fragColor=vec4(linearToSrgb(col),1.0);
}
