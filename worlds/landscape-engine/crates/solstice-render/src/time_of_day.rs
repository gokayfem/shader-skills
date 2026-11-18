//! The time of day: where the sun and moon are, and how they light the scene.
//!
//! A [`TimeOfDay`] at a [`Place`] gives an [`Environment`]. By day the sun lights the scene; the
//! sky tables turn it gold near the horizon and keep the sky glowing after it sets. As twilight
//! fades the moon takes over, at a phase and position that follow the date, and stars appear.
//! Exposure is held to daylight, so night stays dark rather than being brightened to grey.
//!
//! The sun follows the standard formulas for solar declination and hour angle. The moon is a
//! simple model, enough for the right look rather than an almanac: it goes through its phases
//! every 29.53 days, is opposite the sun when full, and so rises about 50 minutes later each day.
//!
//! Directions are in scene space: +Y is up, and a [`Place`]'s `north` says which way north is.
//! At the default, north is -Z and east is +X, as for the weather's wind.

use crate::frame::Environment;
use glam::{Quat, Vec3};
use std::f32::consts::{FRAC_PI_2, TAU};

/// Days from new moon to new moon.
const LUNAR_MONTH: f32 = 29.530_59;
/// Day of the year of a new moon, to anchor the phases.
const A_NEW_MOON: f32 = 6.0;
/// The tilt of the Earth's axis, in degrees.
const OBLIQUITY: f32 = 23.44;
/// How finely the light's direction moves, in degrees. Every move redraws the sun's shadow maps,
/// so the light steps rather than glides: a twentieth of a degree is invisible but saves redrawing
/// every frame.
const LIGHT_STEP_DEGREES: f32 = 0.05;
/// Moonlight against sunlight. A full moon gives about 1/400 000 of the sun's light; games and
/// films show it far brighter so that night is readable, and this follows them.
const MOONLIGHT: f32 = 1.0 / 2500.0;
/// Radiance of the moonless night sky's own glow, airglow and starlight, per unit of daylight
/// sun: dark, but enough that shapes show against it. It shines by day too, lost in the sunlit
/// sky, so twilight fades down to it and never dips darker than night.
const NIGHT_GLOW: f32 = 1.2e-4;
/// Radiance of the moon's sunlit highlands, per unit of daylight sun. By day it is the true
/// value, sunlight off grey rock, about as bright as the blue sky, so the moon shows pale. By
/// night the eye adapts to the moon apart from the dark land, and sees its face; a single
/// exposure cannot, so the moon is dimmed to keep its seas from washing out.
const MOON_RADIANCE_DAY: f32 = 0.03;
const MOON_RADIANCE_NIGHT: f32 = 0.0024;
/// Radiance of the brightest stars, per unit of daylight sun.
const STARLIGHT: f32 = 2.0e-3;

/// Where on Earth the scene is, and which way it faces.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Place {
    /// Degrees north of the equator; negative for the south.
    pub latitude: f32,
    /// The scene's rotation of north about +Y, in radians: 0 puts north along -Z.
    pub north: f32,
}

impl Default for Place {
    /// Somewhere at the latitude of the Mediterranean.
    fn default() -> Self {
        Self {
            latitude: 38.0,
            north: 0.0,
        }
    }
}

/// A moment in the year.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TimeOfDay {
    /// Day of the year, from 1 on the first of January.
    pub day: f32,
    /// Local solar time in hours, 0 to 24: the sun is highest at 12.
    pub hours: f32,
}

impl Default for TimeOfDay {
    /// A summer's morning.
    fn default() -> Self {
        Self {
            day: 172.0,
            hours: 9.0,
        }
    }
}

/// Where the sun and moon are, and the moon's phase.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sky {
    /// Unit directions towards the sun and the moon.
    pub sun: Vec3,
    pub moon: Vec3,
    /// How much of the moon's face is lit: 0 new, 1 full.
    pub moon_lit: f32,
    /// The direction of the celestial pole, about which the stars turn.
    pub pole: Vec3,
    /// The direction of the March equinox: the point on the celestial equator where right
    /// ascension starts. With `pole` it places every star.
    pub equinox: Vec3,
}

impl Sky {
    /// The direction of a star at `right_ascension` and `declination`, in radians.
    pub fn celestial(&self, right_ascension: f32, declination: f32) -> Vec3 {
        let (sin_dec, cos_dec) = declination.sin_cos();
        let (sin_ra, cos_ra) = right_ascension.sin_cos();
        let towards_ra_6h = self.pole.cross(self.equinox);
        self.equinox * cos_dec * cos_ra + towards_ra_6h * cos_dec * sin_ra + self.pole * sin_dec
    }
}

/// A direction from altitude and azimuth, in radians: azimuth from north towards east.
fn horizontal(altitude: f32, azimuth: f32, place: &Place) -> Vec3 {
    let (sin_alt, cos_alt) = altitude.sin_cos();
    let (sin_az, cos_az) = azimuth.sin_cos();
    // North is -Z and east +X before turning the scene.
    let local = Vec3::new(cos_alt * sin_az, sin_alt, -cos_alt * cos_az);
    Quat::from_rotation_y(place.north) * local
}

/// Altitude and azimuth of a body at `declination` and `hour_angle` (radians) seen from
/// `latitude` (radians).
fn altitude_azimuth(declination: f32, hour_angle: f32, latitude: f32) -> (f32, f32) {
    let sin_alt =
        latitude.sin() * declination.sin() + latitude.cos() * declination.cos() * hour_angle.cos();
    let altitude = sin_alt.clamp(-1.0, 1.0).asin();
    // Azimuth from north, towards east: negative hour angles are in the morning, to the east.
    let y = -hour_angle.sin() * declination.cos();
    let x =
        declination.sin() * latitude.cos() - declination.cos() * latitude.sin() * hour_angle.cos();
    (altitude, y.atan2(x))
}

impl TimeOfDay {
    /// Moves the time on by `hours`, wrapping into the next day.
    pub fn advance(&mut self, hours: f32) {
        self.hours += hours;
        let days = (self.hours / 24.0).floor();
        self.hours -= days * 24.0;
        self.day = (self.day - 1.0 + days).rem_euclid(365.0) + 1.0;
    }

    /// Where the sun, moon and stars are at this time, seen from `place`.
    pub fn sky(&self, place: &Place) -> Sky {
        let latitude = place.latitude.to_radians();
        // The sun's declination through the year, highest at the June solstice.
        let year = TAU * (self.day + 10.0) / 365.0;
        let sun_declination = -OBLIQUITY.to_radians() * year.cos();
        let sun_hour_angle = TAU * (self.hours - 12.0) / 24.0;
        let (sun_alt, sun_az) = altitude_azimuth(sun_declination, sun_hour_angle, latitude);

        // The moon's age through its month, 0 at new moon and 0.5 at full.
        let age = ((self.day + self.hours / 24.0 - A_NEW_MOON) / LUNAR_MONTH).rem_euclid(1.0);
        let moon_lit = 0.5 - 0.5 * (TAU * age).cos();
        // It trails the sun around the sky by its age: opposite when full. Its declination is
        // near the sun's when new and opposite when full, as the full moon rides high in winter.
        let moon_hour_angle = sun_hour_angle - TAU * age;
        let moon_declination = sun_declination * (TAU * age).cos();
        let (moon_alt, moon_az) = altitude_azimuth(moon_declination, moon_hour_angle, latitude);

        // The stars: the sun's right ascension, from its longitude along the ecliptic (0 at the
        // March equinox, as its declination above assumes), gives the sidereal time, which is
        // the hour angle of the equinox.
        let sun_longitude = year - FRAC_PI_2;
        let sun_right_ascension =
            (OBLIQUITY.to_radians().cos() * sun_longitude.sin()).atan2(sun_longitude.cos());
        let sidereal_time = sun_hour_angle + sun_right_ascension;
        let (equinox_alt, equinox_az) = altitude_azimuth(0.0, sidereal_time, latitude);

        Sky {
            sun: horizontal(sun_alt, sun_az, place),
            moon: horizontal(moon_alt, moon_az, place),
            moon_lit,
            // The north celestial pole: above the northern horizon, or below the southern one.
            pole: horizontal(latitude, 0.0, place),
            equinox: horizontal(equinox_alt, equinox_az, place),
        }
    }

    /// The scene's lighting at this time: `daylight` is how the scene is lit by a high sun,
    /// whose direction is ignored.
    pub fn environment(&self, place: &Place, daylight: &Environment) -> Environment {
        let sky = self.sky(place);
        let sun_height = sky.sun.y.asin().to_degrees();
        let moon_height = sky.moon.y.asin().to_degrees();

        // The sun lights the scene until twilight fades, about 10 degrees below the horizon;
        // its light dims to nothing over the last few degrees, while the night sky's own glow
        // and the moon take over in near darkness, where the change cannot be seen.
        let sun_fade = smoothstep(-10.0, -5.0, sun_height);
        let night = 1.0 - smoothstep(-14.0, -6.0, sun_height);
        let use_sun = sun_height > -10.0;

        let mut environment = *daylight;
        environment.daylight_illuminance = daylight.sun_illuminance;
        environment.night_sky = NIGHT_GLOW * daylight.sun_illuminance;
        environment.stars = night * STARLIGHT * daylight.sun_illuminance;
        environment.star_pole = sky.pole;
        environment.star_equinox = sky.equinox;
        environment.sun_disc = Some(sky.sun);
        environment.moon_disc = Some(sky.moon);
        environment.moon_radiance = daylight.sun_illuminance
            * (MOON_RADIANCE_DAY + (MOON_RADIANCE_NIGHT - MOON_RADIANCE_DAY) * night);
        if use_sun {
            environment.sun_direction = sky.sun;
            environment.sun_illuminance = daylight.sun_illuminance * sun_fade;
        } else {
            // Moonlight, fading in after the sun has gone and as the moon rises.
            let rise = smoothstep(-4.0, 6.0, moon_height);
            environment.sun_direction = sky.moon;
            environment.sun_illuminance = daylight.sun_illuminance
                * MOONLIGHT
                * sky.moon_lit
                * rise
                * smoothstep(-10.0, -14.0, sun_height);
            // Moonlight is sunlight reflected off grey rock: the same colour, a touch warmer.
            environment.sun_color = daylight.sun_color * Vec3::new(1.0, 0.97, 0.92);
        }
        environment.sun_direction = stepped(environment.sun_direction);
        environment
    }
}

/// `direction`, moved to the nearest of a fine set of directions, so that a slowly moving light
/// changes only now and then.
fn stepped(direction: Vec3) -> Vec3 {
    let step = LIGHT_STEP_DEGREES.to_radians();
    let altitude = (direction.y.clamp(-1.0, 1.0).asin() / step).round() * step;
    let azimuth = (direction.x.atan2(-direction.z) / step).round() * step;
    let (sin_alt, cos_alt) = altitude.sin_cos();
    Vec3::new(cos_alt * azimuth.sin(), sin_alt, -cos_alt * azimuth.cos())
}

fn smoothstep(low: f32, high: f32, x: f32) -> f32 {
    let t = ((x - low) / (high - low)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn degrees_up(direction: Vec3) -> f32 {
        direction.y.asin().to_degrees()
    }

    #[test]
    fn the_sun_rises_in_the_east_peaks_at_noon_and_sets_in_the_west() {
        let place = Place::default();
        let at = |hours| TimeOfDay { day: 80.0, hours }.sky(&place).sun;
        // Near the equinox the sun rises due east and sets due west, at about six o'clock.
        let dawn = at(6.0);
        assert!(degrees_up(dawn).abs() < 2.0, "dawn at {}", degrees_up(dawn));
        assert!(dawn.x > 0.95, "rises in the east: {dawn}");
        let dusk = at(18.0);
        assert!(dusk.x < -0.95, "sets in the west: {dusk}");
        // At noon it is due south, as high as 90 degrees less the latitude.
        let noon = at(12.0);
        assert!(noon.z > 0.0 && noon.x.abs() < 1.0e-3, "noon sun {noon}");
        assert!(
            (degrees_up(noon) - (90.0 - 38.0)).abs() < 1.5,
            "{}",
            degrees_up(noon)
        );
        assert!(degrees_up(at(0.0)) < -40.0);
    }

    #[test]
    fn summer_sun_is_higher_and_the_south_has_it_the_other_way_round() {
        let noon = |day, latitude| {
            let place = Place {
                latitude,
                ..Default::default()
            };
            degrees_up(TimeOfDay { day, hours: 12.0 }.sky(&place).sun)
        };
        assert!((noon(172.0, 38.0) - (90.0 - 38.0 + 23.44)).abs() < 1.0);
        assert!((noon(355.0, 38.0) - (90.0 - 38.0 - 23.44)).abs() < 1.0);
        assert!(noon(355.0, -38.0) > noon(172.0, -38.0));
        // South of the tropics the noon sun is to the north.
        let place = Place {
            latitude: -38.0,
            ..Default::default()
        };
        assert!(
            TimeOfDay {
                day: 80.0,
                hours: 12.0
            }
            .sky(&place)
            .sun
            .z < 0.0
        );
    }

    #[test]
    fn north_can_point_anywhere() {
        let turned = Place {
            north: std::f32::consts::FRAC_PI_2,
            ..Default::default()
        };
        let sun = TimeOfDay {
            day: 80.0,
            hours: 6.0,
        }
        .sky(&turned)
        .sun;
        let plain = TimeOfDay {
            day: 80.0,
            hours: 6.0,
        }
        .sky(&Place::default())
        .sun;
        assert!(
            (Quat::from_rotation_y(std::f32::consts::FRAC_PI_2) * plain).distance(sun) < 1.0e-5
        );
    }

    #[test]
    fn the_full_moon_is_opposite_the_sun_and_the_new_moon_beside_it() {
        let place = Place::default();
        let full_day = A_NEW_MOON + LUNAR_MONTH / 2.0;
        let full = TimeOfDay {
            day: full_day,
            hours: 0.0,
        }
        .sky(&place);
        assert!(full.moon_lit > 0.99);
        assert!(
            full.moon.dot(full.sun) < -0.8,
            "full moon {} sun {}",
            full.moon,
            full.sun
        );
        assert!(degrees_up(full.moon) > 20.0, "full moon at midnight is up");
        let new = TimeOfDay {
            day: A_NEW_MOON,
            hours: 12.0,
        }
        .sky(&place);
        assert!(new.moon_lit < 0.01);
        assert!(new.moon.dot(new.sun) > 0.9);
    }

    #[test]
    fn time_wraps_into_the_next_day_and_year() {
        let mut time = TimeOfDay {
            day: 365.0,
            hours: 23.0,
        };
        time.advance(3.0);
        assert!((time.hours - 2.0).abs() < 1.0e-4 && (time.day - 1.0).abs() < 1.0e-4);
    }

    /// Whether the light comes from the moon rather than the sun.
    fn lit_by_moon(environment: &Environment) -> bool {
        let light = environment.sun_direction;
        let (sun, moon) = (
            environment.sun_disc.unwrap(),
            environment.moon_disc.unwrap(),
        );
        light.angle_between(moon) < light.angle_between(sun)
    }

    #[test]
    fn day_is_lit_by_the_sun_and_night_by_a_dim_moon_with_stars() {
        let place = Place::default();
        let daylight = Environment::default();
        let full_day = A_NEW_MOON + LUNAR_MONTH / 2.0;
        let noon = TimeOfDay {
            day: full_day,
            hours: 12.0,
        }
        .environment(&place, &daylight);
        assert_eq!(noon.sun_illuminance, daylight.sun_illuminance);
        assert!(!lit_by_moon(&noon) && noon.stars == 0.0);

        let midnight = TimeOfDay {
            day: full_day,
            hours: 0.0,
        }
        .environment(&place, &daylight);
        assert!(lit_by_moon(&midnight) && midnight.stars > 0.0 && midnight.night_sky > 0.0);
        assert!(midnight.sun_illuminance > 0.0);
        assert!(midnight.sun_illuminance < daylight.sun_illuminance / 1000.0);
        assert_eq!(midnight.daylight_illuminance, daylight.sun_illuminance);
    }

    #[test]
    fn light_never_jumps_through_dusk() {
        // Through sunset and into the night, the light dims smoothly and never flashes up.
        let place = Place::default();
        let daylight = Environment::default();
        let mut time = TimeOfDay {
            day: 172.0,
            hours: 17.0,
        };
        let mut previous = f32::MAX;
        let mut swapped = false;
        for _ in 0..(4 * 60) {
            let environment = time.environment(&place, &daylight);
            let light = environment.sun_illuminance;
            if lit_by_moon(&environment) && !swapped {
                swapped = true;
                // The moon takes over in near darkness.
                assert!(
                    light < daylight.sun_illuminance * 0.001,
                    "{light} at the swap"
                );
            }
            if !swapped {
                assert!(light <= previous + 1.0e-6, "brightened at {} h", time.hours);
            }
            previous = light;
            time.advance(1.0 / 60.0);
        }
        assert!(swapped, "never handed over to the moon");
    }

    #[test]
    fn the_light_moves_in_small_steps() {
        let place = Place::default();
        let daylight = Environment::default();
        let mut time = TimeOfDay::default();
        let mut changes = 0;
        let mut previous = time.environment(&place, &daylight).sun_direction;
        // A minute of day time, a frame at a time at a day per twenty minutes.
        for _ in 0..60 {
            time.advance(24.0 / (20.0 * 60.0 * 60.0));
            let direction = time.environment(&place, &daylight).sun_direction;
            if direction != previous {
                changes += 1;
                assert!(direction.angle_between(previous).to_degrees() < 0.2);
            }
            previous = direction;
        }
        assert!(changes < 30, "the light moved on {changes} of 60 frames");
    }

    #[test]
    fn the_sun_sits_at_its_own_place_among_the_stars() {
        // Its right ascension and declination, by the same model, put through the star frame,
        // must land on the sun: this checks the frame's handedness and its turning.
        let place = Place::default();
        for (day, hours) in [(20.0, 7.0), (100.0, 12.0), (172.0, 16.5), (300.0, 10.0)] {
            let sky = TimeOfDay { day, hours }.sky(&place);
            let year = TAU * (day + 10.0) / 365.0;
            let longitude = year - FRAC_PI_2;
            let right_ascension =
                (OBLIQUITY.to_radians().cos() * longitude.sin()).atan2(longitude.cos());
            let declination = -OBLIQUITY.to_radians() * year.cos();
            let star = sky.celestial(right_ascension, declination);
            assert!(star.angle_between(sky.sun).to_degrees() < 0.5, "day {day}");
        }
    }

    #[test]
    fn the_galactic_centre_is_low_in_the_south_at_midnight_in_june() {
        // As seen from 38 degrees north: Sagittarius, 23 degrees up, due south.
        let sky = TimeOfDay {
            day: 172.0,
            hours: 0.0,
        }
        .sky(&Place::default());
        let centre = sky.celestial(266.4_f32.to_radians(), (-28.94_f32).to_radians());
        assert!(
            (degrees_up(centre) - 23.0).abs() < 1.5,
            "{}",
            degrees_up(centre)
        );
        let towards_south = Vec3::new(centre.x, 0.0, centre.z).normalize();
        assert!(
            towards_south.angle_between(Vec3::Z).to_degrees() < 8.0,
            "{towards_south}"
        );
        // An hour later it has moved west, as the sky turns.
        let later = TimeOfDay {
            day: 172.0,
            hours: 1.0,
        }
        .sky(&Place::default());
        let moved = later.celestial(266.4_f32.to_radians(), (-28.94_f32).to_radians());
        assert!(moved.x < centre.x - 0.05);
        // And the pole star stands at the latitude's height over the north.
        let polaris = sky.celestial(0.0, 89.3_f32.to_radians());
        assert!((degrees_up(polaris) - 38.0).abs() < 1.0);
    }
}
