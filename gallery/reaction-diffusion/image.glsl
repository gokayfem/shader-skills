// Image channel 0 = current Buffer A, linear, clamp. DEBUG_MODE 3 displays the two concentrations.
#define DEBUG_MODE 0

// Library code below is bundled from shader-skills (MIT): https://github.com/gokayfem/shader-skills
// ---- shader-skills library: color ----
vec3 linearToSrgb(vec3 c) {
    c = max(c, 0.0);
    return mix(c * 12.92, 1.055 * pow(c, vec3(1.0 / 2.4)) - 0.055, step(0.0031308, c));
}

// ---- shader-skills library: tonemap ----
vec3 tonemapNeutral(vec3 c) {
    const float startCompression = 0.76;
    const float desaturation = 0.15;
    c = max(c, 0.0);
    float x = min(c.r, min(c.g, c.b));
    float offset = x < 0.08 ? x - 6.25 * x * x : 0.04;
    c -= offset;
    float peak = max(c.r, max(c.g, c.b));
    if (peak < startCompression) return c;
    const float d = 1.0 - startCompression;
    float newPeak = 1.0 - d * d / (peak + d - startCompression);
    c *= newPeak / peak;
    float g = 1.0 - 1.0 / (desaturation * (peak - newPeak) + 1.0);
    return mix(c, vec3(newPeak), g);
}

// ---- shader ----
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
