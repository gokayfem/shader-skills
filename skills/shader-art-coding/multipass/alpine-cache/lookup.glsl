// Image. Bind Buffer A to channel 0, linear filtering, clamp, no mipmaps.
// Exact evaluation outside the baked region keeps long camera runs continuous.
float alpineHeight(vec2 p) {
    vec2 uv = (p - CACHE_LOW) / (CACHE_HIGH - CACHE_LOW);
    if (any(lessThan(uv, vec2(0.0))) || any(greaterThan(uv, vec2(1.0)))) return alpineHeightExact(p);
    vec2 size = vec2(textureSize(iChannel0, 0));
    return textureLod(iChannel0, (uv * (size - 1.0) + 0.5) / size, 0.0).r;
}
