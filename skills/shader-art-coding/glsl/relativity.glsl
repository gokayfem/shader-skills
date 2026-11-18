//! relativity: null rays in a Schwarzschild orbital plane, in units of the Schwarzschild radius.

/// Derivative of (u, du/dphi), where u = Schwarzschild radius / ray radius. phi is the orbital angle in radians.
/// @use light bending around a spherical nonrotating black hole, photon sphere diagnostics
/// @see Bruneton, Real-time High-Quality Rendering of Non-Rotating Black Holes, section 3.
vec2 nullOrbitRate(vec2 state) {
    return vec2(state.y, 1.5 * state.x * state.x - state.x);
}

/// Advance a null geodesic by a fixed orbital angle with fourth-order Runge-Kutta.
/// @use thin accretion disk crossings, black hole capture cones, critical orbit tests
/// @see https://arxiv.org/abs/2010.08735 (geodesic equation; this integrator uses no lookup tables)
vec2 nullOrbitStep(vec2 state, float angle) {
    vec2 a = nullOrbitRate(state);
    vec2 b = nullOrbitRate(state + 0.5 * angle * a);
    vec2 c = nullOrbitRate(state + 0.5 * angle * b);
    vec2 d = nullOrbitRate(state + angle * c);
    return state + (angle / 6.0) * (a + 2.0 * b + 2.0 * c + d);
}
