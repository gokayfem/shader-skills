// Image channel 0 = current Buffer A, linear, clamp. DEBUG_MODE 3 displays the two concentrations.
#define DEBUG_MODE 0
void mainImage(out vec4 fragColor,in vec2 fragCoord){
    vec2 uv=fragCoord/iResolution.xy;
    vec2 texel=1.0/vec2(textureSize(iChannel0,0));
    vec2 state=texture(iChannel0,uv).rg;
    float gx=texture(iChannel0,uv+vec2(texel.x,0)).g-texture(iChannel0,uv-vec2(texel.x,0)).g;
    float gy=texture(iChannel0,uv+vec2(0,texel.y)).g-texture(iChannel0,uv-vec2(0,texel.y)).g;
    vec3 n=normalize(vec3(-5.0*gx,-5.0*gy,1.0));
    float b=state.y;
    vec3 ink=mix(vec3(0.008,0.033,0.045),vec3(0.013,0.36,0.34),smoothstep(0.03,0.17,b));
    ink=mix(ink,vec3(0.98,0.49,0.16),smoothstep(0.17,0.30,b));
    ink=mix(ink,vec3(0.93,0.79,0.48),smoothstep(0.31,0.40,b));
    vec3 lamp=normalize(vec3(-0.65,0.55,0.48));
    vec3 color=ink*(0.38+1.15*max(dot(n,lamp),0.0))
              +0.45*pow(max(dot(n,normalize(lamp+vec3(0,0,1))),0.0),24.0);
#if DEBUG_MODE == 3
    color=vec3(state,0.0);
#endif
    fragColor=vec4(linearToSrgb(tonemapNeutral(color)),1.0);
}
