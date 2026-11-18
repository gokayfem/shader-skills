//! color: transfer functions, perceptual color (Oklab / OkLCh), palettes, luminance, blackbody light colors.
//! Light, shade and mix in LINEAR color. Picked hex colors, cosine palettes and Shadertoy textures are sRGB-encoded,
//! so decode them with srgbToLinear before lighting. Encode once at the very end with linearToSrgb.

/// Exact sRGB decode (EOTF): encoded [0,1] -> linear.
/// @use decoding picked colors, palettes used as albedo, and texture samples
vec3 srgbToLinear(vec3 c) {
    c = max(c, 0.0);
    return mix(c / 12.92, pow((c + 0.055) / 1.055, vec3(2.4)), step(0.04045, c));
}

/// Exact sRGB encode (OETF): linear -> display [0,1]. Apply exactly once, after tonemapping.
/// pow(c, 1/2.2) is visibly wrong near black (0.001 encodes to 0.013 here vs 0.043 with 2.2).
/// @use final output
vec3 linearToSrgb(vec3 c) {
    c = max(c, 0.0);
    return mix(c * 12.92, 1.055 * pow(c, vec3(1.0 / 2.4)) - 0.055, step(0.0031308, c));
}

/// Relative luminance of linear Rec.709 / sRGB color.
/// @use desaturation, brightness masks, luminance-based effects
float luminance(vec3 c) { return dot(c, vec3(0.2126, 0.7152, 0.0722)); }

/// Scale saturation around luminance: 0 = grey, 1 = unchanged, > 1 = more saturated.
/// @use grading, fading distant objects, stylization
vec3 saturation(vec3 c, float s) { return mix(vec3(luminance(c)), c, s); }

/// Linear sRGB -> Oklab (L in [0,1], a/b around [-0.4, 0.4]). Matrices from Bjorn Ottosson (public domain / MIT).
/// @use perceptual gradients, even-lightness palettes, hue and chroma edits
/// @see https://bottosson.github.io/posts/oklab/
vec3 linearSrgbToOklab(vec3 c) {
    vec3 lms = c * mat3(0.4122214708, 0.5363325363, 0.0514459929,
                        0.2119034982, 0.6806995451, 0.1073969566,
                        0.0883024619, 0.2817188376, 0.6299787005);
    lms = sign(lms) * pow(abs(lms), vec3(1.0 / 3.0));
    return lms * mat3(0.2104542553, 0.7936177850, -0.0040720468,
                      1.9779984951, -2.4285922050, 0.4505937099,
                      0.0259040371, 0.7827717662, -0.8086757660);
}

/// Oklab -> linear sRGB. The result can fall outside [0,1] for saturated colors, so clamp after tonemapping.
/// @use converting perceptual edits back for lighting or output
vec3 oklabToLinearSrgb(vec3 lab) {
    vec3 lms = lab * mat3(1.0, 0.3963377774, 0.2158037573,
                          1.0, -0.1055613458, -0.0638541728,
                          1.0, -0.0894841775, -1.2914855480);
    lms = lms * lms * lms;
    return lms * mat3(4.0767416621, -3.3077115913, 0.2309699292,
                      -1.2684380046, 2.6097574011, -0.3413193965,
                      -0.0041960863, -0.7034186147, 1.7076147010);
}

/// Mix two linear colors in Oklab. Avoids the muddy mid-tones of RGB lerps and the hue detours of HSV.
/// @use gradients, sky ramps, color transitions over time, blending materials
vec3 mixOklab(vec3 a, vec3 b, float t) {
    return oklabToLinearSrgb(mix(linearSrgbToOklab(a), linearSrgbToOklab(b), t));
}

/// OkLCh (lightness, chroma, hue in radians) -> linear sRGB. Hue sweeps keep lightness constant.
/// L about 0.4-0.9, C about 0.05-0.2 stays mostly in gamut.
/// @use harmonious palettes, rainbow ramps with even brightness, per-instance hue variation
vec3 oklchToLinearSrgb(vec3 lch) {
    return oklabToLinearSrgb(vec3(lch.x, lch.y * cos(lch.z), lch.y * sin(lch.z)));
}

/// Cosine palette a + b * cos(2 pi (c t + d)). Output is display-like (sRGB-ish), so decode it for albedo.
/// Integer c values loop seamlessly over t in [0,1].
/// @use quick gradients, heat maps, psychedelic color cycling, per-id colors
/// @see https://iquilezles.org/articles/palettes/
vec3 cosinePalette(float t, vec3 a, vec3 b, vec3 c, vec3 d) {
    return a + b * cos(TAU * (c * t + d));
}

/// Approximate linear sRGB color of a blackbody at temperature kelvin (1000-15000 K), normalized to max channel 1.
/// Uses the Krystek (1985) Planckian-locus fit -> XYZ -> sRGB.
/// @use fire, embers, lava, stars, incandescent lamps, sunset light, molten metal
vec3 blackbody(float kelvin) {
    float T = clamp(kelvin, 1000.0, 15000.0);
    float u = (0.860117757 + 1.54118254e-4 * T + 1.28641212e-7 * T * T) / (1.0 + 8.42420235e-4 * T + 7.08145163e-7 * T * T);
    float v = (0.317398726 + 4.22806245e-5 * T + 4.20481691e-8 * T * T) / (1.0 - 2.89741816e-5 * T + 1.61456053e-7 * T * T);
    float x = 3.0 * u / (2.0 * u - 8.0 * v + 4.0);
    float y = 2.0 * v / (2.0 * u - 8.0 * v + 4.0);
    vec3 xyz = vec3(x / y, 1.0, (1.0 - x - y) / y);
    vec3 rgb = xyz * mat3(3.2406, -1.5372, -0.4986,
                          -0.9689, 1.8758, 0.0415,
                          0.0557, -0.2040, 1.0570);
    rgb = max(rgb, 0.0);
    return rgb / max(max(rgb.r, rgb.g), max(rgb.b, 1e-6));
}

/// First-order peak wavelength for a periodic stack in air, using effective index n and layer period in nm.
/// @use structural blue on butterfly wings, angle-dependent multilayer coatings
/// @see Kinoshita et al., Forma 17 (2002), 103-121. Effective optical period only.
float braggPeakNm(float periodNm, float n, float cosIncidence) {
    float c = clamp(abs(cosIncidence), 0.0, 1.0);
    return 2.0 * periodNm * sqrt(max(n * n - 1.0 + c * c, 0.0));
}
