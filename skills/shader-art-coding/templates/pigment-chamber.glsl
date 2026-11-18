// A warm ceiling panel lights a plaster sphere in a blue and terracotta room.
// @render time=2.0
// @variant DEBUG_MODE=6
#define DEBUG_MODE 0 // 6 direct, left and right bounce contributions
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
