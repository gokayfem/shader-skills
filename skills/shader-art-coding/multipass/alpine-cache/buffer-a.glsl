// Buffer A. Bind Buffer A itself to channel 0 for feedback. Store in floating point.
// A WebGL host can draw this pass once at an independent resolution and retain its texture.
void mainImage(out vec4 fragColor, in vec2 fragCoord) {
    ivec2 size = ivec2(iResolution.xy);
    ivec2 pixel = ivec2(fragCoord);
    // Dimensions occupy spare channels. Rebuild if a host resets or resizes the buffer.
    if (iFrame > 0 && all(equal(texelFetch(iChannel0, ivec2(0), 0).gb, vec2(size)))) {
        fragColor = texelFetch(iChannel0, pixel, 0);
        return;
    }
    vec2 xz = mix(CACHE_LOW, CACHE_HIGH, vec2(pixel) / max(vec2(size - 1), vec2(1.0)));
    fragColor = vec4(alpineHeight(xz), vec2(size), 1.0);
}
