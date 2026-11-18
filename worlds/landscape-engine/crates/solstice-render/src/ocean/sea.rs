//! The sea as the renderer runs it: the ocean easing from one kind to another, the waves
//! following the weather's wind as a real sea does, slowly, and the waves themselves.

use super::spectrum::{unresolved_slopes, SeaState, SLOPE_TABLE_SIZE};
use super::waves::OceanWaves;
use super::Ocean;
use crate::clouds::Weather;
use crate::gpu::Gpu;
use std::f32::consts::{PI, TAU};

/// Seconds for the sea to make up most of the way to a new wind. Real seas take hours to build;
/// this is long enough to read as a sea building and calming, short enough to see happen.
pub const SEA_RESPONSE_SECONDS: f32 = 60.0;

pub struct Sea {
    waves: OceanWaves,
    ocean: Ocean,
    target: Ocean,
    /// Seconds over which the ocean eases to `target`, and how far it has come.
    transition: f32,
    elapsed: f32,
    start: Ocean,
    /// The sea state the waves are drawn from, which lags the weather's wind.
    state: Option<SeaState>,
    /// Slopes too fine for the waves to draw, for the surface's roughness, the state they were
    /// worked out for, and how long ago.
    slopes: [f32; SLOPE_TABLE_SIZE],
    slopes_for: Option<SeaState>,
    since_slopes: f32,
}

impl Sea {
    pub fn new(gpu: &Gpu, ocean: Ocean, resolution: u32) -> Self {
        Self {
            waves: OceanWaves::new(gpu, resolution),
            ocean,
            target: ocean,
            transition: 0.0,
            elapsed: 0.0,
            start: ocean,
            state: None,
            slopes: [0.0; SLOPE_TABLE_SIZE],
            slopes_for: None,
            since_slopes: 0.0,
        }
    }

    /// Eases to `ocean` over `seconds`, or at once if `seconds` is zero.
    pub fn set_ocean(&mut self, ocean: Ocean, seconds: f32) {
        self.start = self.ocean;
        self.target = ocean;
        self.transition = seconds.max(0.0);
        self.elapsed = 0.0;
        if seconds <= 0.0 {
            self.ocean = ocean;
        }
    }

    /// The ocean as it is now, part way to its target.
    pub fn ocean(&self) -> Ocean {
        self.ocean
    }

    /// The sea state the waves are drawn from now.
    pub fn state(&self) -> Option<SeaState> {
        self.state
    }

    pub fn waves(&self) -> &OceanWaves {
        &self.waves
    }

    /// The mean square slope of the waves shorter than each wavenumber of the table in
    /// `spectrum::unresolved_slopes`.
    pub fn slopes(&self) -> &[f32; SLOPE_TABLE_SIZE] {
        &self.slopes
    }

    /// Moves the sea on by `seconds` under `weather`, with tiles of `resolution` texels.
    pub fn update(&mut self, gpu: &Gpu, weather: &Weather, seconds: f32, resolution: u32) {
        if self.waves.size() != resolution {
            self.waves = OceanWaves::new(gpu, resolution);
        }
        if self.transition > 0.0 && self.elapsed < self.transition {
            self.elapsed += seconds;
            let t = (self.elapsed / self.transition).clamp(0.0, 1.0);
            let eased = t * t * (3.0 - 2.0 * t);
            self.ocean = self.start.lerp(&self.target, eased);
        } else {
            self.ocean = self.target;
        }

        let wanted = SeaState::new(&self.ocean, weather);
        let state = match self.state {
            None => wanted,
            Some(previous) => follow_wind(previous, wanted, seconds),
        };
        self.state = Some(state);
        // The roughness table takes an integral, so while the sea eases it follows a few times a
        // second, as the waves' amplitudes do.
        self.since_slopes += seconds.max(0.0);
        if self.slopes_for != Some(state)
            && (self.slopes_for.is_none() || self.since_slopes >= 0.25)
        {
            self.slopes = unresolved_slopes(&state.params());
            self.slopes_for = Some(state);
            self.since_slopes = 0.0;
        }
        self.waves.update(gpu, &state, &self.ocean, seconds);
    }

    pub fn record(&mut self, encoder: &mut wgpu::CommandEncoder) {
        self.waves.record(encoder);
    }
}

/// The sea state `seconds` on from `previous`, towards `wanted`. The wind's speed and direction
/// come through slowly, turning the short way; everything else comes from the ocean, which eases
/// on its own.
pub fn follow_wind(previous: SeaState, wanted: SeaState, seconds: f32) -> SeaState {
    let follow = 1.0 - (-seconds.max(0.0) / SEA_RESPONSE_SECONDS).exp();
    let turn = (wanted.wind_angle - previous.wind_angle + PI).rem_euclid(TAU) - PI;
    SeaState {
        wind_speed: previous.wind_speed + (wanted.wind_speed - previous.wind_speed) * follow,
        wind_angle: previous.wind_angle + turn * follow,
        ..wanted
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sea_follows_the_wind_slowly() {
        let ocean = Ocean::default();
        let at = |speed: f32, from: f32| {
            SeaState::new(
                &ocean,
                &Weather {
                    wind_speed: speed,
                    wind_from: from,
                    ..Weather::default()
                },
            )
        };
        let (calm, gale) = (at(4.0, 350.0), at(30.0, 10.0));
        let mut state = calm;
        for _ in 0..60 {
            state = follow_wind(state, gale, 1.0);
        }
        // After a minute the sea has come most of the way, but not all.
        let share = (state.wind_speed - calm.wind_speed) / (gale.wind_speed - calm.wind_speed);
        assert!((0.55..0.75).contains(&share), "{share}");
        // It turned the short way, through north, not round through south.
        let turned = (state.wind_angle - calm.wind_angle + PI).rem_euclid(TAU) - PI;
        let full = (gale.wind_angle - calm.wind_angle + PI).rem_euclid(TAU) - PI;
        assert!(turned * full > 0.0 && turned.abs() < full.abs());
        // And it does not depend on the frame rate.
        let mut fine = calm;
        for _ in 0..600 {
            fine = follow_wind(fine, gale, 0.1);
        }
        assert!((fine.wind_speed - state.wind_speed).abs() < 1.0e-3);
    }
}
