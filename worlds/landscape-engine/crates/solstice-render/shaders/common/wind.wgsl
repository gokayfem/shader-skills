// The wind near the ground: the weather's mean wind, with gusts and lulls carried downwind
// across the scene and its direction wandering from place to place (docs/weather-effects.md).
// Twin of `WindSnapshot::velocity_at` in wind.rs, which tests compare against.

#include "common/noise.wgsl"

// The wind at one moment. Twin of `WindSnapshot` in wind.rs.
struct WindSnapshot {
    // The mean wind 10 m up, in m/s as (east, south): the scene's +X and +Z.
    mean: vec2f,
    // How far the air has carried everything since the wind began, in metres. Gusts are fixed
    // in the air, so they are found at `p - offset` (Taylor's frozen turbulence).
    offset: vec2f,
    time: f32,
    // 0..1, from the weather.
    gusts: f32,
    turbulence: f32,
    _pad: f32,
}

// A snapshot packed into two vectors, as the water's uniforms carry it: the mean wind, gusts and
// turbulence; then the air's offset and the time.
fn unpack_wind(a: vec4f, b: vec4f) -> WindSnapshot {
    return WindSnapshot(a.xy, b.xy, b.z, a.z, a.w, 0.0);
}

// Sizes of the gusts, in metres: broad gusts and lulls, and the rolls within them that run
// over a field of grass as waves.
const WIND_GUST_SIZE: f32 = 42.0;
const WIND_ROLL_SIZE: f32 = 11.0;
const WIND_VEER_SIZE: f32 = 50.0;
// How fast each changes shape as it is carried, in noise cells a second.
const WIND_GUST_CHANGE: f32 = 0.045;
const WIND_ROLL_CHANGE: f32 = 0.2;
const WIND_VEER_CHANGE: f32 = 0.07;
// How strongly gusts and rolls change the wind's speed at `gusts` 1, and how far, in radians,
// its direction wanders at `turbulence` 1.
const WIND_GUST_STRENGTH: f32 = 1.4;
const WIND_ROLL_STRENGTH: f32 = 0.6;
const WIND_VEER_ANGLE: f32 = 0.7;
// The least a lull leaves of the mean wind.
const WIND_LULL: f32 = 0.1;
const WIND_SEED_GUST: u32 = 0x5bd1e995u;
const WIND_SEED_ROLL: u32 = 0x27d4eb2fu;
const WIND_SEED_VEER: u32 = 0x165667b1u;

// The wind 10 m up at `p`, in m/s as (east, south).
fn wind_velocity(s: WindSnapshot, p: vec2f) -> vec2f {
    let speed = length(s.mean);
    if speed < 1.0e-4 {
        return vec2f(0.0);
    }
    let q = p - s.offset;
    let gust = gradient_noise(vec3f(q / WIND_GUST_SIZE, s.time * WIND_GUST_CHANGE), WIND_SEED_GUST).x;
    let roll = gradient_noise(vec3f(q / WIND_ROLL_SIZE, s.time * WIND_ROLL_CHANGE), WIND_SEED_ROLL).x;
    let veer = gradient_noise(vec3f(q / WIND_VEER_SIZE, s.time * WIND_VEER_CHANGE), WIND_SEED_VEER).x;
    let factor = max(WIND_LULL, 1.0 + s.gusts * (WIND_GUST_STRENGTH * gust + WIND_ROLL_STRENGTH * roll));
    let angle = s.turbulence * WIND_VEER_ANGLE * veer;
    let c = cos(angle);
    let sn = sin(angle);
    let along = s.mean / speed;
    return vec2f(along.x * c - along.y * sn, along.x * sn + along.y * c) * speed * factor;
}
