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
