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
