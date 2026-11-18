//! The sea's spectrum: how much wave there is at each wavelength and direction, and the waves
//! drawn from it.
//!
//! The wind sea follows Elfouhaily, Chapron, Katsaros and Vandemark (1997), the unified spectrum
//! behind most work on how the sea reflects light. Its long waves grow with the wind and the
//! fetch, through the wave age, and match measured seas: a fully developed sea is as high as
//! Pierson and Moskowitz found. Its short waves, the ripples that set how rough the water looks,
//! grow with the wind's friction on the surface, so a light breeze leaves the sea almost glassy
//! and a gale whips it rough. Waves spread around the wind's direction after Donelan and Banner,
//! most narrowly near the peak. A swell from distant storms adds a narrow band of long waves from
//! its own direction, in JONSWAP's shape.
//!
//! The waves are drawn on three square tiles of different sizes, the *cascades*, each carrying a
//! band of wavelengths the others don't, so together they cover from hundreds of metres down to
//! centimetres without repeating visibly. Each wave gets a random amplitude and phase from a hash
//! of its own wavenumber, so the same sea comes out whatever the tiles' resolution.
//!
//! Everything here is mirrored by `shaders/ocean/common.wgsl`, and tests hold the two together.

use super::Ocean;
use crate::clouds::Weather;
use std::f32::consts::{PI, TAU};

pub const GRAVITY: f32 = 9.81;

/// Width of each cascade's tile, in metres. Their ratios are not whole numbers, so their repeats
/// never line up. The first is large enough to carry a long swell.
pub const CASCADE_SIZES: [f32; 3] = [1000.0, 128.0, 17.0];

/// The longest wave a cascade carries is this fraction of its tile; longer ones belong to the
/// cascade before. Enough texels across each wave to draw it smoothly.
const LONGEST_WAVE_PER_TILE: f32 = 1.0 / 6.0;

/// Every wave's frequency is rounded to a multiple of one over this many seconds, so the whole
/// sea repeats exactly after it. That keeps each wave's phase exact however long the sea runs,
/// where a plain clock would lose precision; the rounding itself is far too small to see.
pub const REPEAT_SECONDS: f32 = 1000.0;

/// Elfouhaily's short-wave constants: the wavenumber of the slowest ripples, where gravity and
/// surface tension balance, about 1.7 cm long, and their phase speed.
const RIPPLE_WAVENUMBER: f32 = 370.0;
const RIPPLE_SPEED: f32 = 0.23;

/// A swell's spectrum is narrower than a wind sea's, and its direction steadier.
const SWELL_PEAK_ENHANCEMENT: f32 = 6.0;
const SWELL_SPREAD: f32 = 8.0;

/// Elfouhaily's dimensionless fetch, g·F/U², at which a sea is close to fully developed.
const DEVELOPED_FETCH: f32 = 22_000.0;

/// The wavenumbers at which cascade 0 hands over to 1, and 1 to 2.
pub fn band_limits() -> [f32; 2] {
    [
        TAU / (CASCADE_SIZES[1] * LONGEST_WAVE_PER_TILE),
        TAU / (CASCADE_SIZES[2] * LONGEST_WAVE_PER_TILE),
    ]
}

/// Whether waves of wavenumber `k` belong to `cascade`.
pub fn in_band(k: f32, cascade: usize) -> bool {
    let [first, second] = band_limits();
    match cascade {
        0 => k > 0.0 && k < first,
        1 => k >= first && k < second,
        _ => k >= second,
    }
}

/// The state of the sea that waves are drawn from: the wind over it and the swell through it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SeaState {
    /// Wind 10 m above the water, in metres per second.
    pub wind_speed: f32,
    /// Direction the wind blows towards, as an angle in the scene's ground plane: `atan2(z, x)`.
    pub wind_angle: f32,
    /// Distance the wind blows across open water, in metres.
    pub fetch: f32,
    pub swell_height: f32,
    pub swell_period: f32,
    /// Direction the swell travels towards, as `wind_angle`.
    pub swell_angle: f32,
}

/// The numbers the spectrum is evaluated from, worked out once for a sea state. The GPU
/// receives exactly these.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpectrumParams {
    /// Wavenumber at the wind sea's peak, in radians per metre.
    pub peak_wavenumber: f32,
    /// Inverse wave age: the wind's speed over the peak waves'. Near 0.84 a sea is fully
    /// developed; young seas, on short fetches, run up to 5.
    pub inverse_age: f32,
    pub wind_angle: f32,
    /// Elfouhaily's long- and short-wave levels, α_p and α_m.
    pub long_level: f32,
    pub short_level: f32,
    /// JONSWAP-like sharpening of the peak, γ, and its width, σ.
    pub peak_enhancement: f32,
    pub peak_width: f32,
    /// Phase speed of the peak waves, in metres per second.
    pub peak_speed: f32,
    pub swell_scale: f32,
    /// Angular frequency at the swell's peak, in radians per second.
    pub swell_peak: f32,
    pub swell_angle: f32,
}

impl SeaState {
    /// The sea raised by `weather`'s wind on `ocean`.
    pub fn new(ocean: &Ocean, weather: &Weather) -> Self {
        // The weather's wind is at cloud height, about twice that near the sea.
        let [east, south] = weather.wind_velocity();
        let swell_towards = (ocean.swell.from + 180.0).to_radians();
        Self {
            wind_speed: east.hypot(south) * 0.5,
            wind_angle: south.atan2(east),
            fetch: ocean.fetch * 1000.0,
            swell_height: ocean.swell.height,
            swell_period: ocean.swell.period,
            // Compass bearings run clockwise from north (-Z): east is +X, south is +Z.
            swell_angle: (-swell_towards.cos()).atan2(swell_towards.sin()),
        }
    }

    pub fn params(&self) -> SpectrumParams {
        let wind = self.wind_speed.max(0.3);
        // Elfouhaily's eq. 37: the wave age follows the fetch, from a young sea to a fully
        // developed one.
        let fetch = GRAVITY * self.fetch.max(1.0) / (wind * wind);
        let inverse_age = (0.84 * (fetch / DEVELOPED_FETCH).powf(0.4).tanh().powf(-0.75)).min(5.0);
        let peak_wavenumber = GRAVITY * (inverse_age / wind) * (inverse_age / wind);
        let peak_speed = (GRAVITY / peak_wavenumber).sqrt();
        // The wind's friction velocity, from the sea's own roughness (eqs. 60 and 66).
        let roughness = 3.7e-5 * wind * wind / GRAVITY * (wind / peak_speed).powf(0.9);
        let friction = 0.41 * wind / (10.0 / roughness).ln();
        let ratio = friction / RIPPLE_SPEED;
        let short_level = if ratio < 1.0 {
            0.01 * (1.0 + ratio.ln())
        } else {
            0.01 * (1.0 + 3.0 * ratio.ln())
        };
        let swell_peak = TAU / self.swell_period.max(1.0);
        SpectrumParams {
            peak_wavenumber,
            inverse_age,
            wind_angle: self.wind_angle,
            long_level: 0.006 * inverse_age.powf(0.55),
            short_level: short_level.max(0.0),
            peak_enhancement: if inverse_age < 1.0 {
                1.7
            } else {
                1.7 + 6.0 * inverse_age.log10()
            },
            peak_width: 0.08 * (1.0 + 4.0 / inverse_age.powi(3)),
            peak_speed,
            // The swell carries the energy of its significant height: Hs = 4 √m0.
            swell_scale: (self.swell_height / 4.0).powi(2)
                / shape_integral(swell_peak, SWELL_PEAK_ENHANCEMENT),
            swell_peak,
            swell_angle: self.swell_angle,
        }
    }

    /// Significant wave height of the wind sea, in metres: four times the square root of the
    /// variance its spectrum holds.
    pub fn wind_sea_height(&self) -> f32 {
        4.0 * omnidirectional_integral(&self.params(), |k, curvature| curvature / (k * k * k))
            .sqrt()
    }
}

/// Elfouhaily's curvature spectrum of the wind sea at wavenumber `k`, `B = k³·S(k)`: the long
/// waves around the peak (eq. 31), and the short waves the wind's friction raises (eq. 40).
///
/// The short waves are also cut off below the peak by Pierson and Moskowitz's factor, as in
/// Bruneton, Neyret and Holzschuch's implementation (2010). As published (eq. 41) they carry no
/// such factor, and would add ripples longer than the sea's own peak waves; the factor is
/// negligible everywhere else.
pub fn wind_curvature(k: f32, params: &SpectrumParams) -> f32 {
    if k <= 0.0 {
        return 0.0;
    }
    let speed = (GRAVITY / k).sqrt();
    let root = (k / params.peak_wavenumber).sqrt();
    let below_peak = params.peak_wavenumber / k;
    // Pierson and Moskowitz's cut-off below the peak.
    let cutoff = (-1.25 * (below_peak * below_peak).min(80.0)).exp();
    let sharpening =
        (-(root - 1.0) * (root - 1.0) / (2.0 * params.peak_width * params.peak_width)).exp();
    let peak = cutoff
        * params.peak_enhancement.powf(sharpening)
        * (-params.inverse_age / 10.0_f32.sqrt() * (root - 1.0)).exp();
    let long = 0.5 * params.long_level * params.peak_speed / speed * peak;
    let ripples = (-0.25 * (k / RIPPLE_WAVENUMBER - 1.0) * (k / RIPPLE_WAVENUMBER - 1.0)).exp();
    let short = 0.5 * params.short_level * RIPPLE_SPEED / speed * ripples * cutoff;
    long + short
}

/// The integral over wavenumber, from far below the peak to far above, of `f(k, B(k))`.
fn omnidirectional_integral(params: &SpectrumParams, f: impl Fn(f32, f32) -> f32) -> f32 {
    let (low, high, steps) = (1.0e-4_f64, 2000.0_f64, 20_000);
    let ratio = high / low;
    let mut sum = 0.0_f64;
    for i in 0..steps {
        // Logarithmic in k, which the spectrum's narrow peak needs.
        let k = low * ratio.powf((i as f64 + 0.5) / steps as f64);
        let dk = k * ratio.ln() / steps as f64;
        sum += f(k as f32, wind_curvature(k as f32, params)) as f64 * dk;
    }
    sum as f32
}

/// Entries in [`unresolved_slopes`]' table, one per step of wavenumber.
pub const SLOPE_TABLE_SIZE: usize = 32;
/// The table runs from 10^`SLOPE_TABLE_FIRST` to 10^(`SLOPE_TABLE_FIRST` + 31·`SLOPE_TABLE_STEP`)
/// radians per metre: from waves 600 m long to ripples 6 mm long.
pub const SLOPE_TABLE_FIRST: f32 = -2.0;
pub const SLOPE_TABLE_STEP: f32 = 5.0 / 31.0;
/// No ripple is shorter than about 3 mm; surface tension smooths anything finer.
const SHORTEST_RIPPLE: f32 = 2000.0;

/// For each wavenumber k in the table, the mean square slope of every wave shorter than k:
/// what a pixel too large to show those waves sees only as roughness. After Bruneton, Neyret
/// and Holzschuch (2010): the surface's slopes are carried by the waves drawn, or by roughness,
/// never lost, so distant water is as rough as it should be and the sun's glitter widens
/// towards the horizon.
pub fn unresolved_slopes(params: &SpectrumParams) -> [f32; SLOPE_TABLE_SIZE] {
    let mut table = [0.0; SLOPE_TABLE_SIZE];
    // From the shortest ripples down, adding up as the wavenumber falls.
    let steps = 4000;
    let top = (SHORTEST_RIPPLE as f64).log10();
    let bottom = SLOPE_TABLE_FIRST as f64;
    let step = (top - bottom) / steps as f64;
    let mut sum = 0.0_f64;
    let mut entry = SLOPE_TABLE_SIZE as i64 - 1;
    for i in 0..=steps {
        let log_k = top - i as f64 * step;
        while entry >= 0 && log_k <= bottom + entry as f64 * SLOPE_TABLE_STEP as f64 {
            table[entry as usize] = sum as f32;
            entry -= 1;
        }
        let k = 10f64.powf(log_k);
        // Mean square slope: ∫ B(k)/k dk, here over log k, so dk = k ln(10) d(log k).
        let dk = k * std::f64::consts::LN_10 * step;
        sum += wind_curvature(k as f32, params) as f64 / k * dk;
    }
    while entry >= 0 {
        table[entry as usize] = sum as f32;
        entry -= 1;
    }
    table
}

/// JONSWAP's shape over angular frequency, without its scale: a steep rise below the peak, a
/// sharper peak than Pierson and Moskowitz's by `enhancement`, and a fall as ω⁻⁵ above. For
/// the swell.
pub fn jonswap_shape(omega: f32, peak: f32, enhancement: f32) -> f32 {
    if omega <= 0.0 {
        return 0.0;
    }
    let width = if omega <= peak { 0.07 } else { 0.09 };
    let r = (-(omega - peak) * (omega - peak) / (2.0 * width * width * peak * peak)).exp();
    let ratio = peak / omega;
    let ratio2 = ratio * ratio;
    enhancement.powf(r) * (-1.25 * ratio2 * ratio2).exp() / omega.powi(5)
}

/// The integral of [`jonswap_shape`] over all frequencies, for setting a spectrum's energy.
fn shape_integral(peak: f32, enhancement: f32) -> f32 {
    // Almost all of it lies within a factor of a few of the peak; integrate finely there.
    let (low, high, steps) = (0.3 * peak as f64, 12.0 * peak as f64, 4000);
    let step = (high - low) / steps as f64;
    let sum: f64 = (0..steps)
        .map(|i| jonswap_shape((low + (i as f64 + 0.5) * step) as f32, peak, enhancement) as f64)
        .sum();
    (sum * step) as f32
}

/// How tightly waves at `ratio` times the peak frequency gather around the wind's direction,
/// after Donelan and Banner: most tightly at the peak, spreading wider above and below it.
pub fn banner_spread(ratio: f32) -> f32 {
    let beta = if ratio < 0.95 {
        2.61 * ratio.powf(1.3)
    } else if ratio < 1.6 {
        2.28 * ratio.powf(-1.3)
    } else {
        10.0_f32.powf(-0.4 + 0.8393 * (-0.567 * (ratio * ratio).ln()).exp())
    };
    beta.max(0.01)
}

/// Donelan and Banner's spreading over direction, `angle` from the mean in `-π..π`, for
/// tightness `beta`. It integrates to one over the circle.
pub fn spread(beta: f32, angle: f32) -> f32 {
    let sech = 1.0 / (beta * angle).cosh();
    beta / (2.0 * (beta * PI).tanh()) * sech * sech
}

fn wrap_angle(angle: f32) -> f32 {
    (angle + PI).rem_euclid(TAU) - PI
}

/// The spectrum over wavenumber: the variance of the sea's height per unit area of
/// wavenumber, at `(kx, kz)`, in m⁴. Deep water: ω² = gk.
pub fn spectrum(kx: f32, kz: f32, params: &SpectrumParams) -> f32 {
    let k = kx.hypot(kz);
    if k < 1.0e-6 {
        return 0.0;
    }
    let omega = (GRAVITY * k).sqrt();
    let angle = kz.atan2(kx);
    let peak_omega = (GRAVITY * params.peak_wavenumber).sqrt();
    // S(k, θ) = B(k) / k⁴ · D(θ), over the plane of wavenumbers.
    let wind = wind_curvature(k, params) / (k * k * k * k)
        * spread(
            banner_spread(omega / peak_omega),
            wrap_angle(angle - params.wind_angle),
        );
    let domega_dk = GRAVITY / (2.0 * omega);
    let swell = params.swell_scale
        * jonswap_shape(omega, params.swell_peak, SWELL_PEAK_ENHANCEMENT)
        * spread(SWELL_SPREAD, wrap_angle(angle - params.swell_angle))
        * domega_dk
        / k;
    wind + swell
}

/// The wavenumber of wave `(nx, nz)` on `cascade`'s tile, in radians per metre.
pub fn wavenumber(cascade: usize, nx: i32, nz: i32) -> [f32; 2] {
    let dk = TAU / CASCADE_SIZES[cascade];
    [nx as f32 * dk, nz as f32 * dk]
}

/// The PCG hash, exactly as `pcg` in `common/math.wgsl`.
pub fn pcg(v: u32) -> u32 {
    let state = v.wrapping_mul(747_796_405).wrapping_add(2_891_336_453);
    let word = ((state >> ((state >> 28) + 4)) ^ state).wrapping_mul(277_803_737);
    (word >> 22) ^ word
}

fn hash_to_unit(v: u32) -> f32 {
    (pcg(v) >> 8) as f32 * (1.0 / 16_777_216.0)
}

/// Two independent standard normal numbers for wave `(nx, nz)` of `cascade`, by Box and Muller.
pub fn gaussian_pair(cascade: usize, nx: i32, nz: i32, seed: u32) -> [f32; 2] {
    let hash = pcg(nx as u32 ^ pcg(nz as u32 ^ pcg(cascade as u32 ^ seed)));
    let u1 = hash_to_unit(hash);
    let u2 = hash_to_unit(hash ^ 0x68bc_21eb);
    let radius = (-2.0 * (1.0 - u1).ln()).sqrt();
    let angle = TAU * u2;
    [radius * angle.cos(), radius * angle.sin()]
}

/// The complex amplitude of wave `(nx, nz)` on `cascade`'s tile of `size` texels at time zero.
/// Its expected squared size is half the spectrum's variance in that wave's cell, the other half
/// being carried by its mirror image, so that the sea's variance is the spectrum's integral.
pub fn initial_amplitude(
    cascade: usize,
    size: u32,
    nx: i32,
    nz: i32,
    params: &SpectrumParams,
    seed: u32,
) -> [f32; 2] {
    let half = size as i32 / 2;
    // The Nyquist row and column have no mirror on the tile, so they carry nothing; nor does
    // the mean level.
    if nx == -half || nz == -half {
        return [0.0, 0.0];
    }
    let [kx, kz] = wavenumber(cascade, nx, nz);
    if !in_band(kx.hypot(kz), cascade) {
        return [0.0, 0.0];
    }
    let dk = TAU / CASCADE_SIZES[cascade];
    let amplitude = (spectrum(kx, kz, params) * dk * dk / 4.0).sqrt();
    let [a, b] = gaussian_pair(cascade, nx, nz, seed);
    [a * amplitude, b * amplitude]
}

/// How many steps of one over [`REPEAT_SECONDS`] hertz a wave of wavenumber `k` turns through
/// per second: its angular frequency, rounded so that the sea repeats exactly.
pub fn frequency_steps(k: f32) -> f32 {
    // floor(x + 0.5) rather than round, which rounds halves differently in WGSL.
    ((GRAVITY * k).sqrt() * REPEAT_SECONDS / TAU + 0.5).floor()
}

/// The variance of the sea's height that waves on `cascade`'s tile of `size` texels are expected
/// to have: the spectrum summed over the cells it carries.
pub fn expected_variance(cascade: usize, size: u32, params: &SpectrumParams) -> f32 {
    let half = size as i32 / 2;
    let dk = TAU / CASCADE_SIZES[cascade];
    let mut sum = 0.0_f64;
    for nz in -half + 1..half {
        for nx in -half + 1..half {
            let [kx, kz] = wavenumber(cascade, nx, nz);
            if in_band(kx.hypot(kz), cascade) {
                sum += (spectrum(kx, kz, params) * dk * dk) as f64;
            }
        }
    }
    sum as f32
}

/// The eight fields the GPU makes for `cascade`, summed directly over every wave, at `(x, z)`
/// metres and `tau` through the repeat. `amplitude` gives each wave's starting amplitude, as
/// read back from the GPU. In the GPU's order: horizontal displacement x, height, horizontal
/// displacement z, ∂Dx/∂z; slopes ∂h/∂x and ∂h/∂z, ∂Dx/∂x, ∂Dz/∂z. For tests.
///
/// The horizontal displacement D = i·(k/|k|)·h moves points towards each crest, so that adding it
/// sharpens crests and widens troughs, as real waves are shaped; its divergence is then negative
/// at crests, where the surface folds and foam forms.
pub fn direct_sum(
    cascade: usize,
    size: u32,
    amplitude: &dyn Fn(i32, i32) -> [f32; 2],
    tau: f32,
    x: f32,
    z: f32,
) -> [f32; 8] {
    let half = size as i32 / 2;
    let mut fields = [0.0_f64; 8];
    for nz in -half..half {
        for nx in -half..half {
            let [kx, kz] = wavenumber(cascade, nx, nz);
            let k = kx.hypot(kz);
            if k == 0.0 {
                continue;
            }
            let phase = TAU as f64 * (frequency_steps(k) as f64 * tau as f64).fract();
            let (sin_t, cos_t) = phase.sin_cos();
            let [ar, ai] = amplitude(nx, nz).map(|v| v as f64);
            let [mr, mi] = amplitude(-nx, -nz).map(|v| v as f64);
            // h = a·e^{iωt} + conj(m)·e^{-iωt}
            let hr = ar * cos_t - ai * sin_t + mr * cos_t - mi * sin_t;
            let hi = ar * sin_t + ai * cos_t - mr * sin_t - mi * cos_t;
            let (kx, kz, k) = (kx as f64, kz as f64, k as f64);
            let (sin_p, cos_p) = (kx * x as f64 + kz * z as f64).sin_cos();
            // Real part of (c · h · e^{ik·x}) for a complex factor c.
            let real = |cr: f64, ci: f64| {
                let (pr, pi) = (cr * hr - ci * hi, cr * hi + ci * hr);
                pr * cos_p - pi * sin_p
            };
            fields[0] += real(0.0, kx / k);
            fields[1] += real(1.0, 0.0);
            fields[2] += real(0.0, kz / k);
            fields[3] += real(-kx * kz / k, 0.0);
            fields[4] += real(0.0, kx);
            fields[5] += real(0.0, kz);
            fields[6] += real(-kx * kx / k, 0.0);
            fields[7] += real(-kz * kz / k, 0.0);
        }
    }
    fields.map(|v| v as f32)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sea(wind: f32, fetch_km: f32) -> SeaState {
        SeaState {
            wind_speed: wind,
            wind_angle: 0.3,
            fetch: fetch_km * 1000.0,
            swell_height: 0.0,
            swell_period: 10.0,
            swell_angle: 0.0,
        }
    }

    /// The spectrum's variance, integrated over wavenumber directly.
    fn continuous_variance(params: &SpectrumParams) -> f32 {
        omnidirectional_integral(params, |k, curvature| curvature / (k * k * k))
            + params.swell_scale * shape_integral(params.swell_peak, SWELL_PEAK_ENHANCEMENT)
    }

    /// Mean square slope of the waves up to wavenumber `limit`.
    fn mean_square_slope(state: &SeaState, limit: f32) -> f32 {
        omnidirectional_integral(&state.params(), |k, curvature| {
            if k <= limit {
                curvature / k
            } else {
                0.0
            }
        })
    }

    #[test]
    fn a_fully_developed_sea_is_as_high_as_pierson_and_moskowitz_say() {
        // Komen, Hasselmann and Hasselmann (1984) restate Pierson and Moskowitz's fully developed
        // sea at 10 m: dimensionless energy g²E/U10⁴ = 3.64e-3, so Hs = 4√E = 0.0246 U10².
        for wind in [5.0, 10.0, 15.0, 20.0] {
            let height = sea(wind, 50_000.0).wind_sea_height();
            let expected = 0.0246 * wind * wind;
            assert!(
                (height / expected - 1.0).abs() < 0.1,
                "{wind} m/s: {height} m"
            );
        }
    }

    #[test]
    fn short_fetches_raise_smaller_waves_as_measured() {
        // Against JONSWAP's growth law, Hs = 0.0016 √(gF/U²) U²/g, within the scatter of the
        // measurements behind both.
        for (wind, fetch) in [(5.0, 30.0), (9.0, 10.0), (9.0, 60.0), (16.0, 300.0)] {
            let height = sea(wind, fetch).wind_sea_height();
            let growth =
                0.0016 * (GRAVITY * fetch * 1000.0 / (wind * wind)).sqrt() * wind * wind / GRAVITY;
            assert!(
                (height / growth - 1.0).abs() < 0.25,
                "{wind} m/s over {fetch} km: {height} m against {growth} m"
            );
        }
        assert!(sea(10.0, 3.0).wind_sea_height() < sea(10.0, 30.0).wind_sea_height());
        // And the peak moves to longer waves as the fetch grows.
        assert!(
            sea(10.0, 100.0).params().peak_wavenumber < sea(10.0, 10.0).params().peak_wavenumber
        );
    }

    #[test]
    fn a_breeze_raises_fewer_ripples_than_a_gale() {
        // The ripples the wind's friction raises grow many times over from a light breeze to a
        // gale, while the long waves' own tail of ripples barely changes (Phillips' saturation).
        let ripples_alone = |wind: f32, k: f32| {
            let params = sea(wind, 100.0).params();
            wind_curvature(
                k,
                &SpectrumParams {
                    long_level: 0.0,
                    ..params
                },
            )
        };
        assert!(ripples_alone(16.0, 40.0) > ripples_alone(3.0, 40.0) * 20.0);
        // So centimetre waves are rougher in a gale.
        let total = |wind: f32| wind_curvature(40.0, &sea(wind, 100.0).params());
        assert!(
            total(16.0) > total(3.0) * 1.5,
            "{} against {}",
            total(16.0),
            total(3.0)
        );
        // The slopes the waves draw stay within what Cox and Munk measured from sun glitter,
        // 0.003 + 0.00512 U: the finer ripples they also saw become the surface's roughness.
        let mut previous = 0.0;
        for wind in [3.0, 6.0, 10.0, 16.0] {
            let drawn = mean_square_slope(&sea(wind, 200.0), 47.0);
            let measured = 0.003 + 0.00512 * wind;
            assert!(
                drawn < measured,
                "{wind} m/s: {drawn} drawn, {measured} measured"
            );
            assert!(
                drawn > measured * 0.25,
                "{wind} m/s: {drawn} drawn, {measured} measured"
            );
            assert!(
                drawn > previous,
                "{wind} m/s is no rougher than a lighter wind"
            );
            previous = drawn;
        }
    }

    #[test]
    fn unresolved_slopes_are_the_slopes_of_shorter_waves() {
        for wind in [3.0, 9.0, 16.0] {
            let state = sea(wind, 100.0);
            let params = state.params();
            let table = unresolved_slopes(&params);
            // Never rising with wavenumber: shorter cut-offs leave fewer waves.
            assert!(
                table.windows(2).all(|w| w[0] >= w[1] - 1.0e-7),
                "{wind}: {table:?}"
            );
            // At the start, nearly every wave is shorter: the whole sea's mean square slope.
            let whole = mean_square_slope(&state, SHORTEST_RIPPLE);
            assert!(
                (table[0] / whole - 1.0).abs() < 0.03,
                "{wind}: {} against {whole}",
                table[0]
            );
            // At the cascades' finest drawn wave, what is left is what they cannot draw.
            let k = 10f32.powf(SLOPE_TABLE_FIRST + 20.0 * SLOPE_TABLE_STEP);
            let expected = whole - mean_square_slope(&state, k);
            assert!(
                (table[20] / expected - 1.0).abs() < 0.05,
                "{wind}: {} against {expected}",
                table[20]
            );
        }
        // A gale's ripples make it rougher than a breeze, at every scale too fine to draw.
        let breeze = unresolved_slopes(&sea(3.0, 100.0).params());
        let gale = unresolved_slopes(&sea(16.0, 100.0).params());
        assert!(
            gale[25] > breeze[25] * 3.0,
            "{} against {}",
            gale[25],
            breeze[25]
        );
    }

    #[test]
    fn spreading_integrates_to_one() {
        for beta in [0.05, 0.5, 2.0, banner_spread(1.0), SWELL_SPREAD] {
            let steps = 20_000;
            let sum: f32 = (0..steps)
                .map(|i| spread(beta, -PI + (i as f32 + 0.5) / steps as f32 * TAU))
                .sum::<f32>()
                * TAU
                / steps as f32;
            assert!((sum - 1.0).abs() < 2.0e-3, "beta {beta}: {sum}");
        }
    }

    #[test]
    fn the_spectrum_holds_the_energy_its_height_says() {
        // Integrating the 2D spectrum over a fine polar grid gives back its variance, and the
        // swell carries exactly the energy of its height.
        let mut state = sea(9.0, 40.0);
        state.swell_height = 0.8;
        state.swell_angle = -1.2;
        let params = state.params();
        let (steps_k, steps_a) = (6000, 720);
        let (k_low, k_high) = (1.0e-3_f64, 40.0_f64);
        let mut sum = 0.0_f64;
        for i in 0..steps_k {
            let t = (i as f64 + 0.5) / steps_k as f64;
            let k = k_low * (k_high / k_low).powf(t);
            let dk = k * (k_high / k_low).ln() / steps_k as f64;
            for j in 0..steps_a {
                let angle = -std::f64::consts::PI
                    + (j as f64 + 0.5) / steps_a as f64 * std::f64::consts::TAU;
                let s =
                    spectrum((k * angle.cos()) as f32, (k * angle.sin()) as f32, &params) as f64;
                sum += s * k * dk * std::f64::consts::TAU / steps_a as f64;
            }
        }
        let expected = continuous_variance(&params) as f64;
        assert!(
            (sum / expected - 1.0).abs() < 0.02,
            "{sum} against {expected}"
        );
        let wind_part = (state.wind_sea_height() / 4.0).powi(2) as f64;
        let swell_part = (0.8_f64 / 4.0).powi(2);
        assert!((expected / (wind_part + swell_part) - 1.0).abs() < 0.01);
    }

    #[test]
    fn the_cascades_together_carry_the_whole_spectrum() {
        // Neither counted twice nor missed: the three tiles' cells add up to the spectrum's
        // variance, for calm and rough seas, with and without swell, at both resolutions.
        for (wind, fetch, swell) in [(4.0, 20.0, 0.0), (9.0, 60.0, 0.6), (16.0, 500.0, 1.5)] {
            let mut state = sea(wind, fetch);
            state.swell_height = swell;
            state.swell_period = 11.0;
            let params = state.params();
            let expected = continuous_variance(&params);
            for size in [128, 256] {
                let carried: f32 = (0..3).map(|c| expected_variance(c, size, &params)).sum();
                assert!(
                    (carried / expected - 1.0).abs() < 0.03,
                    "{wind} m/s, {fetch} km, {size}²: {carried} of {expected}"
                );
            }
        }
    }

    #[test]
    fn waves_follow_the_wind() {
        let state = sea(10.0, 50.0);
        let params = state.params();
        let k = params.peak_wavenumber;
        let at = |turn: f32| {
            spectrum(
                k * (state.wind_angle + turn).cos(),
                k * (state.wind_angle + turn).sin(),
                &params,
            )
        };
        assert!(at(0.0) > at(1.4) * 10.0 && at(1.4) > at(PI));
    }

    #[test]
    fn gaussian_pairs_are_standard_normal() {
        // That the hash itself matches the shader's is checked on the GPU, wave by wave.
        let samples: Vec<f32> = (0..20_000)
            .flat_map(|i| gaussian_pair(0, i, -i, 7))
            .collect();
        let mean = samples.iter().sum::<f32>() / samples.len() as f32;
        let variance =
            samples.iter().map(|v| (v - mean) * (v - mean)).sum::<f32>() / samples.len() as f32;
        assert!(
            mean.abs() < 0.02 && (variance - 1.0).abs() < 0.03,
            "{mean} {variance}"
        );
    }
}
