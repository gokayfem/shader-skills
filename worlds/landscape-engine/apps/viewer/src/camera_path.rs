//! A smooth camera path through keyframes, for repeatable flyovers.
//!
//! The text format has one keyframe per line: `time  x y z  target_x target_y target_z`.

use glam::Vec3;
use solstice_render::Camera;

#[derive(Clone, Copy, Debug)]
struct Key {
    time: f32,
    position: Vec3,
    target: Vec3,
}

/// Keyframes, interpolated through with Catmull-Rom splines.
#[derive(Clone, Debug)]
pub struct CameraPath {
    keys: Vec<Key>,
}

impl CameraPath {
    /// Parses the text format. Blank lines and everything after a `#` are ignored.
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut keys: Vec<Key> = Vec::new();
        for (index, line) in text.lines().enumerate() {
            let line = line.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            let values = line
                .split_whitespace()
                .map(|word| {
                    word.parse::<f32>()
                        .map_err(|_| format!("line {}: '{word}' is not a number", index + 1))
                })
                .collect::<Result<Vec<f32>, String>>()?;
            let [time, x, y, z, tx, ty, tz] = values[..] else {
                return Err(format!(
                    "line {}: expected 7 numbers, found {}",
                    index + 1,
                    values.len()
                ));
            };
            if keys.last().is_some_and(|last| time <= last.time) {
                return Err(format!("line {}: times must increase", index + 1));
            }
            keys.push(Key {
                time,
                position: Vec3::new(x, y, z),
                target: Vec3::new(tx, ty, tz),
            });
        }
        if keys.len() < 2 {
            return Err("a path needs at least two keyframes".to_string());
        }
        Ok(Self { keys })
    }

    pub fn duration(&self) -> f32 {
        self.keys[self.keys.len() - 1].time
    }

    /// The camera at `time`, which is held at either end of the path.
    pub fn sample(&self, time: f32) -> Camera {
        let last = self.keys.len() - 1;
        let time = time.clamp(self.keys[0].time, self.keys[last].time);
        let segment = self
            .keys
            .partition_point(|key| key.time <= time)
            .clamp(1, last)
            - 1;
        let at = |index: isize| self.keys[index.clamp(0, last as isize) as usize];
        let i = segment as isize;
        let (a, b, c, d) = (at(i - 1), at(i), at(i + 1), at(i + 2));
        let t = (time - b.time) / (c.time - b.time);

        let position = catmull_rom(a.position, b.position, c.position, d.position, t);
        let target = catmull_rom(a.target, b.target, c.target, d.target, t);
        let forward = (target - position).normalize_or(Vec3::NEG_Z);
        Camera {
            position,
            yaw: (-forward.x).atan2(-forward.z),
            pitch: forward.y.clamp(-1.0, 1.0).asin(),
            ..Default::default()
        }
    }
}

fn catmull_rom(a: Vec3, b: Vec3, c: Vec3, d: Vec3, t: f32) -> Vec3 {
    let (t2, t3) = (t * t, t * t * t);
    0.5 * (2.0 * b
        + (c - a) * t
        + (2.0 * a - 5.0 * b + 4.0 * c - d) * t2
        + (3.0 * b - a - 3.0 * c + d) * t3)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PATH: &str = "
        # time, position, target
        0   0 0 0    0 0 -10
        10  10 0 0   10 0 -10
        20  10 0 10  0 0 0
    ";

    #[test]
    fn passes_through_every_keyframe() {
        let path = CameraPath::parse(PATH).unwrap();
        assert!(path.sample(0.0).position.distance(Vec3::ZERO) < 1.0e-4);
        assert!(
            path.sample(10.0)
                .position
                .distance(Vec3::new(10.0, 0.0, 0.0))
                < 1.0e-4
        );
        assert!(
            path.sample(20.0)
                .position
                .distance(Vec3::new(10.0, 0.0, 10.0))
                < 1.0e-4
        );
    }

    #[test]
    fn looks_at_its_target() {
        let camera = CameraPath::parse(PATH).unwrap().sample(10.0);
        assert!(camera.forward().distance(Vec3::NEG_Z) < 1.0e-3);
    }

    #[test]
    fn moves_without_jumps() {
        let path = CameraPath::parse(PATH).unwrap();
        let mut previous = path.sample(0.0).position;
        for frame in 1..=1200 {
            let position = path.sample(frame as f32 / 60.0).position;
            assert!(position.distance(previous) < 0.1, "jump at frame {frame}");
            previous = position;
        }
    }

    #[test]
    fn holds_at_the_ends() {
        let path = CameraPath::parse(PATH).unwrap();
        assert_eq!(path.sample(-5.0).position, path.sample(0.0).position);
        assert_eq!(path.sample(99.0).position, path.sample(20.0).position);
    }

    #[test]
    fn bad_input_is_reported() {
        assert!(CameraPath::parse("0 1 2")
            .unwrap_err()
            .contains("7 numbers"));
        assert!(CameraPath::parse("0 0 0 0 0 0 x")
            .unwrap_err()
            .contains("not a number"));
        assert!(CameraPath::parse("0 0 0 0 0 0 0")
            .unwrap_err()
            .contains("two keyframes"));
        let backwards = "5 0 0 0 0 0 0\n1 0 0 0 0 0 0";
        assert!(CameraPath::parse(backwards)
            .unwrap_err()
            .contains("increase"));
    }
}
