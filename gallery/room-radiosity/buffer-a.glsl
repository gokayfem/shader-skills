// A warm ceiling panel lights a plaster sphere in a blue and terracotta room.
// @render time=2.0
// @variant DEBUG_MODE=6
#define DEBUG_MODE 0 // 6 direct, left and right bounce contributions

// Library code below is bundled from shader-skills (MIT): https://github.com/gokayfem/shader-skills
// ---- shader-skills library: common ----
const float PI  = 3.14159265358979;

// ---- shader-skills library: shading ----
// Approximate diffuse form factor from a small, oriented patch to a surface point. Both normals point out of their surfaces. Area is in squared world units. The finite area softens near-field error.
float diffusePatchWeight(vec3 point, vec3 normal, vec3 center, vec3 patchNormal, float area) {
    vec3 delta = center - point;
    float distance2 = dot(delta, delta);
    vec3 direction = delta * inversesqrt(max(distance2, 1e-10));
    float coupling = max(dot(normal, direction), 0.0) * max(dot(patchNormal, -direction), 0.0);
    return coupling * max(area, 0.0) / max(PI * distance2 + max(area, 0.0), 1e-10);
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
// Initialize from the emitter, then apply one Jacobi radiosity iteration per feedback step.
void mainImage(out vec4 fragColor,in vec2 fragCoord){
    ivec2 id=ivec2(fragCoord);
    fragColor=vec4(0,0,0,1);
    if(id.x>=20 || id.y>=4) return;
    vec3 p,n,albedo;float area;
    roomPatch(id,p,n,albedo,area);
    vec3 incoming=panelLight(p,n);
    if(iFrame>0) incoming+=gatherRoom(p,n);
    fragColor=vec4(albedo*incoming,1.0);
}
