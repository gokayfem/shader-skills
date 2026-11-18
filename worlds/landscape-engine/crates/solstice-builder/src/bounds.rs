//! Bounding spheres.

use glam::Vec3;

/// A bounding sphere.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Sphere {
    pub center: Vec3,
    pub radius: f32,
}

impl Sphere {
    /// Bounds a set of points using Ritter's approximation, seeded by the box
    /// centre. Near-optimal in practice and linear in the point count.
    pub fn from_points(points: impl Iterator<Item = Vec3> + Clone) -> Self {
        let (mut min, mut max) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
        for p in points.clone() {
            min = min.min(p);
            max = max.max(p);
        }
        if min.x > max.x {
            return Self::default();
        }
        let mut sphere = Self {
            center: (min + max) * 0.5,
            radius: 0.0,
        };
        for p in points {
            sphere = sphere.grown_to(p);
        }
        sphere
    }

    fn grown_to(self, point: Vec3) -> Self {
        let offset = point - self.center;
        let distance = offset.length();
        if distance <= self.radius {
            return self;
        }
        let radius = (self.radius + distance) * 0.5;
        Self {
            center: self.center + offset * ((radius - self.radius) / distance),
            radius,
        }
    }

    /// The smallest sphere containing both spheres, padded so that the
    /// containment still holds exactly after floating-point rounding.
    pub fn merged(self, other: Self) -> Self {
        let offset = other.center - self.center;
        let distance = offset.length();
        if distance + other.radius <= self.radius {
            return self;
        }
        if distance + self.radius <= other.radius {
            return other;
        }
        let radius = (distance + self.radius + other.radius) * 0.5;
        Self {
            center: self.center + offset * ((radius - self.radius) / distance),
            radius: radius * (1.0 + 1.0e-5),
        }
    }

    /// Merges any number of spheres.
    pub fn merged_all(spheres: impl IntoIterator<Item = Self>) -> Self {
        let mut iter = spheres.into_iter();
        let first = iter.next().unwrap_or_default();
        iter.fold(first, Self::merged)
    }

    /// True if `other` lies inside this sphere, allowing for rounding.
    pub fn contains(self, other: Self) -> bool {
        self.center.distance(other.center) + other.radius <= self.radius * (1.0 + 1.0e-3) + 1.0e-5
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_points_contains_every_point() {
        let points = [
            Vec3::new(1.0, 2.0, 3.0),
            Vec3::new(-4.0, 0.5, 2.0),
            Vec3::new(0.0, -7.0, 1.0),
            Vec3::new(3.0, 3.0, -3.0),
        ];
        let sphere = Sphere::from_points(points.iter().copied());
        for p in points {
            assert!(sphere.center.distance(p) <= sphere.radius + 1.0e-4);
        }
    }

    #[test]
    fn merged_contains_both_inputs() {
        let a = Sphere {
            center: Vec3::ZERO,
            radius: 1.0,
        };
        let b = Sphere {
            center: Vec3::X * 5.0,
            radius: 2.0,
        };
        let merged = a.merged(b);
        assert!(merged.contains(a));
        assert!(merged.contains(b));
    }

    #[test]
    fn merged_with_contained_sphere_is_unchanged() {
        let outer = Sphere {
            center: Vec3::ZERO,
            radius: 10.0,
        };
        let inner = Sphere {
            center: Vec3::X,
            radius: 1.0,
        };
        assert_eq!(outer.merged(inner), outer);
        assert_eq!(inner.merged(outer), outer);
    }
}
