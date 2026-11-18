//! water: finite-depth wave dispersion, analytic wave derivatives, exact dielectric Fresnel and RGB attenuation.
//! Units must agree: wavelength/depth/path in meters, gravity in m/s^2, extinction in 1/m. No textures or scene state.

/// Angular frequency sqrt(g*k*tanh(k*depth)) for finite-depth gravity waves. Nonpositive inputs are clamped.
/// @use shallow coasts, ocean swell, lake waves with depth-dependent speed
/// @range nonnegative
/// @see MIT Marine Hydrodynamics lecture 20: https://ocw.mit.edu/courses/2-20-marine-hydrodynamics-13-021-spring-2005/5d48a5937971d973fd8ca90c051a83f8_lecture20.pdf
float waterDispersion(float k, float depth, float gravity) {
    k = max(k,0.0);
    float e = exp(-2.0*k*max(depth,0.0));
    return sqrt(max(gravity,0.0)*k*(1.0-e)/(1.0+e));
}

/// One sinusoidal wave: vec4(height, dh/dx, dh/dz, Laplacian). Direction is normalized internally; zero uses +X.
/// wavelength > 0, amplitude in meters. footprint filters waves below 2-6 pixels; derivatives hold it constant.
/// @use analytic water normals, layered swell/chop, curvature foam, finite-depth lake and coastal waves
/// @see Mark Finch, GPU Gems chapter 1, "Effective Water Simulation from Physical Models"
vec4 waterWaveD(vec2 xz, float time, vec2 direction, float wavelength, float amplitude, float depth, float footprint) {
    float dl = length(direction);
    vec2 dir = dl>0.000001 ? direction/dl : vec2(1.0,0.0);
    float lengthM = max(wavelength,0.0001);
    float k = TAU/lengthM;
    float keep = smoothstep(2.0,6.0,lengthM/max(footprint,0.000001));
    float phase = k*dot(dir,xz)-waterDispersion(k,depth,9.81)*time;
    float h = amplitude*keep*sin(phase);
    return vec4(h, amplitude*keep*k*cos(phase)*dir, -k*k*h);
}

/// Exact unpolarized dielectric Fresnel for a positive incident cosine and positive refractive indices.
/// Returns 1 for total internal reflection. Caller chooses etaIncident/etaTransmitted for the ray's side.
/// @use air-water interfaces (1.0,1.333), underwater views (1.333,1.0), glass
/// @range [0,1]
/// @see https://www.pbr-book.org/4ed/Reflection_Models/Specular_Reflection_and_Transmission
float fresnelDielectric(float cosIncident, float etaIncident, float etaTransmitted) {
    float ci = clamp(abs(cosIncident),0.0,1.0);
    float ei = max(etaIncident,0.000001), et = max(etaTransmitted,0.000001);
    if (abs(ei-et)<0.0000001) return 0.0;
    float sinT2 = (ei*ei)/(et*et)*max(0.0,1.0-ci*ci);
    if (sinT2>=1.0) return 1.0;
    float ct = sqrt(max(0.0,1.0-sinT2));
    float rs = (ei*ci-et*ct)/max(ei*ci+et*ct,0.000001);
    float rp = (et*ci-ei*ct)/max(et*ci+ei*ct,0.000001);
    return clamp(0.5*(rs*rs+rp*rp),0.0,1.0);
}

/// Beer-Lambert RGB transmittance exp(-extinction*pathLength). Negative extinction/path is clamped to zero.
/// Use the refracted optical path, not vertical water depth. Attenuation alone does not add scattered light.
/// @use depth-colored water, underwater visibility, tinted glass, homogeneous fog
/// @range [0,1]
/// @see https://www.pbr-book.org/4ed/Volume_Scattering/Transmittance
vec3 beerTransmittance(vec3 extinction, float pathLength) {
    return exp(-max(extinction,vec3(0.0))*max(pathLength,0.0));
}

/// Displace a rooted blade along the horizontal current, with zero offset and slope at its base.
/// heightFraction is zero at the root and one at the tip. Amplitude uses world units; phase uses radians.
/// @use kelp forests, sea grass, wind moving through reeds
/// @see Original two-frequency bending profile.
float currentBend(float heightFraction, float amplitude, float phase) {
    float h = clamp(heightFraction, 0.0, 1.0);
    return amplitude * h * h * (0.72 * sin(phase - 2.4*h) + 0.28 * sin(1.7*phase + 3.1*h));
}

/// Procedural caustic filaments on a receiving surface. Footprint is measured in input pattern units.
/// @use rippled sand, submerged timbers, shallow pools, shafts meeting the floor
/// @see Original crossed sine-field pattern. Unresolved bands fade to their average coverage.
float causticLace(vec2 p, float time, float footprint) {
    vec2 q=p+0.22*vec2(sin(1.7*p.y+time),cos(1.3*p.x-0.7*time));
    float f=sin(2.9*q.x+q.y)+0.80*sin(2.6*q.y-1.2*q.x);
    float width=clamp(2.0*footprint,0.015,0.30);
    float line=1.0-smoothstep(0.055,0.055+width,abs(f));
    return mix(line,0.085,smoothstep(0.08,0.32,footprint));
}
