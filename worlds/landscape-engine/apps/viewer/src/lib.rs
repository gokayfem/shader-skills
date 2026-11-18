//! Shared parts of the viewer. The native window and the browser page are thin shells
//! around [`FlyCamera`] and the renderer.

pub mod bob;
pub mod camera_path;
#[cfg(feature = "gameplay")]
pub mod play;
#[cfg(feature = "gameplay")]
pub mod shared;
#[cfg(target_arch = "wasm32")]
mod web;

use glam::Vec3;
use solstice_asset::scene::Viewpoint;
use solstice_render::{Camera, Environment, Place, TimeOfDay};

/// Keyboard and mouse state, in a form both front ends can fill in.
#[derive(Clone, Copy, Debug, Default)]
pub struct CameraInput {
    pub forward: f32,
    pub right: f32,
    pub up: f32,
    pub look_yaw: f32,
    pub look_pitch: f32,
    pub fast: bool,
}

/// A free-flying first-person camera.
#[derive(Clone, Copy, Debug)]
pub struct FlyCamera {
    pub camera: Camera,
    pub speed: f32,
    velocity: Vec3,
}

impl FlyCamera {
    pub fn new(camera: Camera) -> Self {
        Self {
            camera,
            speed: 8.0,
            velocity: Vec3::ZERO,
        }
    }

    /// Applies input accumulated over `delta` seconds.
    pub fn update(&mut self, input: &CameraInput, delta: f32) {
        self.camera.yaw += input.look_yaw;
        self.camera.pitch = (self.camera.pitch + input.look_pitch).clamp(-1.5, 1.5);
        let speed = self.speed * if input.fast { 5.0 } else { 1.0 };
        let wanted = (self.camera.forward() * input.forward
            + self.camera.right() * input.right
            + Vec3::Y * input.up)
            * speed;
        // Ease towards the wanted velocity, independent of frame rate. Instant starts and
        // stops read as stutter, whatever the frame rate. Looking is left direct, since any
        // smoothing there would be felt as input lag.
        const RESPONSE_PER_SECOND: f32 = 10.0;
        self.velocity += (wanted - self.velocity) * (1.0 - (-delta * RESPONSE_PER_SECOND).exp());
        self.camera.position += self.velocity * delta;
    }
}

/// Where a scene opens: its own viewpoint, if it has one.
pub fn initial_camera(viewpoint: Option<Viewpoint>) -> Camera {
    let mut camera = Camera::default();
    if let Some(viewpoint) = viewpoint {
        camera.position = Vec3::from_array(viewpoint.position);
        camera.yaw = viewpoint.yaw;
        camera.pitch = viewpoint.pitch;
    }
    camera
}

/// Late-afternoon sun, low enough for long shadows.
pub fn default_environment() -> Environment {
    Environment {
        sun_direction: Vec3::new(0.62, 0.42, 0.5).normalize(),
        sun_illuminance: 14.0,
        // Above the physical ratio: like a sky light in a game engine, this holds shadow
        // detail that a real camera would need several stops of range to capture.
        sky_intensity: 3.0,
        gi_intensity: 0.85,
        ..Default::default()
    }
}

/// Real seconds a whole day takes to pass, by default: twenty minutes, slow enough to watch
/// the light change, fast enough to see a day go by.
pub const DEFAULT_DAY_SECONDS: f32 = 20.0 * 60.0;

/// The time of day, passing at a chosen speed, and the light it gives.
#[derive(Clone, Copy, Debug)]
pub struct DayClock {
    pub time: TimeOfDay,
    pub place: Place,
    /// Real seconds a whole day takes, or 0 to hold the time still.
    pub day_seconds: f32,
    /// How the scene is lit by a high sun; the clock moves the sun and moon around it.
    pub daylight: Environment,
}

impl DayClock {
    pub fn new(time: TimeOfDay, day_seconds: f32) -> Self {
        Self {
            time,
            place: Place::default(),
            day_seconds,
            daylight: default_environment(),
        }
    }

    /// Moves the time on by `real_seconds` of real time, and returns the light it gives.
    pub fn tick(&mut self, real_seconds: f32) -> Environment {
        if self.day_seconds > 0.0 {
            self.time.advance(real_seconds * 24.0 / self.day_seconds);
        }
        self.environment()
    }

    /// The light at the current time.
    pub fn environment(&self) -> Environment {
        self.time.environment(&self.place, &self.daylight)
    }
}

/// Reads a time of day written `HH:MM`, or plain hours.
pub fn parse_time(text: &str) -> Option<f32> {
    let hours = match text.split_once(':') {
        Some((h, m)) => h.parse::<f32>().ok()? + m.parse::<f32>().ok()? / 60.0,
        None => text.parse::<f32>().ok()?,
    };
    (0.0..=24.0).contains(&hours).then_some(hours)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fly(frames: u32, delta: f32, forward: f32, camera: &mut FlyCamera) {
        let input = CameraInput {
            forward,
            ..Default::default()
        };
        for _ in 0..frames {
            camera.update(&input, delta);
        }
    }

    #[test]
    fn reaches_full_speed_and_coasts_to_a_stop() {
        let mut camera = FlyCamera::new(Camera::default());
        fly(1, 1.0 / 60.0, 1.0, &mut camera);
        assert!(
            camera.velocity.length() < camera.speed * 0.3,
            "starts gradually"
        );
        fly(120, 1.0 / 60.0, 1.0, &mut camera);
        assert!(
            (camera.velocity.length() - camera.speed).abs() < 0.01,
            "reaches full speed"
        );
        fly(120, 1.0 / 60.0, 0.0, &mut camera);
        assert!(camera.velocity.length() < 0.01, "stops");
    }

    #[test]
    fn travel_does_not_depend_on_the_frame_rate() {
        let mut fast = FlyCamera::new(Camera::default());
        let mut slow = FlyCamera::new(Camera::default());
        fly(144, 1.0 / 144.0, 1.0, &mut fast);
        fly(30, 1.0 / 30.0, 1.0, &mut slow);
        let difference = fast.camera.position.distance(slow.camera.position);
        assert!(
            difference < 0.5,
            "travelled {difference} m apart in one second"
        );
    }

    #[test]
    fn times_read_as_hours() {
        assert_eq!(parse_time("18:30"), Some(18.5));
        assert_eq!(parse_time("6"), Some(6.0));
        assert_eq!(parse_time("25:00"), None);
        assert_eq!(parse_time("dusk"), None);
    }

    #[test]
    fn a_clock_passes_a_day_in_its_day_length() {
        let mut clock = DayClock::new(
            TimeOfDay {
                day: 100.0,
                hours: 6.0,
            },
            60.0,
        );
        for _ in 0..60 {
            clock.tick(1.0);
        }
        assert!((clock.time.hours - 6.0).abs() < 1.0e-2 && (clock.time.day - 101.0).abs() < 1.0e-3);
        let mut still = DayClock::new(
            TimeOfDay {
                day: 100.0,
                hours: 6.0,
            },
            0.0,
        );
        still.tick(100.0);
        assert_eq!(still.time.hours, 6.0);
    }
}
