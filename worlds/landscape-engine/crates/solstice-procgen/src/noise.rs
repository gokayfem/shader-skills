//! Gradient noise for procedural shapes and textures.

use glam::{IVec3, Vec3};

fn hash(cell: IVec3, seed: u32) -> u32 {
    let mut h = seed
        .wrapping_mul(0x9E37_79B1)
        .wrapping_add((cell.x as u32).wrapping_mul(0x85EB_CA77))
        .wrapping_add((cell.y as u32).wrapping_mul(0xC2B2_AE3D))
        .wrapping_add((cell.z as u32).wrapping_mul(0x27D4_EB2F));
    h ^= h >> 15;
    h = h.wrapping_mul(0x2C1B_3C6D);
    h ^= h >> 12;
    h = h.wrapping_mul(0x297A_2D39);
    h ^ (h >> 15)
}

fn gradient(h: u32) -> Vec3 {
    const GRADIENTS: [Vec3; 12] = [
        Vec3::new(1.0, 1.0, 0.0),
        Vec3::new(-1.0, 1.0, 0.0),
        Vec3::new(1.0, -1.0, 0.0),
        Vec3::new(-1.0, -1.0, 0.0),
        Vec3::new(1.0, 0.0, 1.0),
        Vec3::new(-1.0, 0.0, 1.0),
        Vec3::new(1.0, 0.0, -1.0),
        Vec3::new(-1.0, 0.0, -1.0),
        Vec3::new(0.0, 1.0, 1.0),
        Vec3::new(0.0, -1.0, 1.0),
        Vec3::new(0.0, 1.0, -1.0),
        Vec3::new(0.0, -1.0, -1.0),
    ];
    GRADIENTS[(h % 12) as usize]
}

/// Perlin noise. `period`, when set, makes the result tile on that integer lattice.
pub fn perlin(p: Vec3, seed: u32, period: Option<i32>) -> f32 {
    perlin_periods(p, seed, period.map(IVec3::splat))
}

/// Perlin noise that tiles with its own integer period along each axis.
fn perlin_periods(p: Vec3, seed: u32, period: Option<IVec3>) -> f32 {
    let base = p.floor();
    let f = p - base;
    let cell = base.as_ivec3();
    let wrap = |c: IVec3| match period {
        Some(n) => IVec3::new(
            c.x.rem_euclid(n.x),
            c.y.rem_euclid(n.y),
            c.z.rem_euclid(n.z),
        ),
        None => c,
    };
    let corner = |offset: IVec3| {
        let g = gradient(hash(wrap(cell + offset), seed));
        g.dot(f - offset.as_vec3())
    };
    let fade = f * f * f * (f * (f * 6.0 - Vec3::splat(15.0)) + Vec3::splat(10.0));

    let lerp = |a: f32, b: f32, t: f32| a + (b - a) * t;
    let x00 = lerp(
        corner(IVec3::new(0, 0, 0)),
        corner(IVec3::new(1, 0, 0)),
        fade.x,
    );
    let x10 = lerp(
        corner(IVec3::new(0, 1, 0)),
        corner(IVec3::new(1, 1, 0)),
        fade.x,
    );
    let x01 = lerp(
        corner(IVec3::new(0, 0, 1)),
        corner(IVec3::new(1, 0, 1)),
        fade.x,
    );
    let x11 = lerp(
        corner(IVec3::new(0, 1, 1)),
        corner(IVec3::new(1, 1, 1)),
        fade.x,
    );
    lerp(lerp(x00, x10, fade.y), lerp(x01, x11, fade.y), fade.z)
}

/// Fractal sum of noise, in roughly `-1..1`.
pub fn fbm(p: Vec3, octaves: u32, seed: u32) -> f32 {
    let (mut sum, mut amplitude, mut frequency, mut total) = (0.0, 0.5, 1.0, 0.0);
    for octave in 0..octaves {
        sum += amplitude * perlin(p * frequency, seed.wrapping_add(octave), None);
        total += amplitude;
        amplitude *= 0.5;
        frequency *= 2.03;
    }
    sum / total * 1.6
}

/// Ridged fractal noise in `0..1`, with sharp creases like eroded stone.
pub fn ridged(p: Vec3, octaves: u32, seed: u32) -> f32 {
    let (mut sum, mut amplitude, mut frequency, mut total) = (0.0, 0.5, 1.0, 0.0);
    for octave in 0..octaves {
        let n = 1.0 - (perlin(p * frequency, seed.wrapping_add(octave), None) * 1.8).abs();
        sum += amplitude * n * n;
        total += amplitude;
        amplitude *= 0.5;
        frequency *= 2.1;
    }
    sum / total
}

/// Fractal noise on a torus: tiles over `0..1` in X and Y, for seamless textures.
pub fn tiling_fbm(u: f32, v: f32, base_period: i32, octaves: u32, seed: u32) -> f32 {
    let (mut sum, mut amplitude, mut period, mut total) = (0.0, 0.5, base_period, 0.0);
    for octave in 0..octaves {
        let p = Vec3::new(u * period as f32, v * period as f32, 0.37);
        sum += amplitude * perlin(p, seed.wrapping_add(octave), Some(period));
        total += amplitude;
        amplitude *= 0.5;
        period *= 2;
    }
    sum / total * 1.6
}

/// Fractal noise on a torus with `across` features over `0..1` in X and `down` in Y, so its
/// features are drawn out along whichever has fewer. Tiles over `0..1` in X and Y.
pub fn tiling_fbm_xy(u: f32, v: f32, across: i32, down: i32, octaves: u32, seed: u32) -> f32 {
    let (mut sum, mut amplitude, mut scale, mut total) = (0.0, 0.5, 1, 0.0);
    for octave in 0..octaves {
        let period = IVec3::new(across * scale, down * scale, 1);
        let p = Vec3::new(u * period.x as f32, v * period.y as f32, 0.37);
        sum += amplitude * perlin_periods(p, seed.wrapping_add(octave), Some(period));
        total += amplitude;
        amplitude *= 0.5;
        scale *= 2;
    }
    sum / total * 1.6
}

/// Ridged fractal noise on a torus, in `0..1`: tiles over `0..1` in X and Y.
pub fn tiling_ridged(u: f32, v: f32, base_period: i32, octaves: u32, seed: u32) -> f32 {
    let (mut sum, mut amplitude, mut period, mut total) = (0.0, 0.5, base_period, 0.0);
    for octave in 0..octaves {
        let p = Vec3::new(u * period as f32, v * period as f32, 0.37);
        let n = 1.0 - (perlin(p, seed.wrapping_add(octave), Some(period)) * 1.8).abs();
        sum += amplitude * n * n;
        total += amplitude;
        amplitude *= 0.5;
        period *= 2;
    }
    sum / total
}

/// Billowing fractal noise on a torus, in `0..1`: puffy lumps with sharp creases between them, the
/// inverse of ridged noise. Tiles over `0..1` in X and Y.
pub fn tiling_billow(u: f32, v: f32, base_period: i32, octaves: u32, seed: u32) -> f32 {
    let (mut sum, mut amplitude, mut period, mut total) = (0.0, 0.5, base_period, 0.0);
    for octave in 0..octaves {
        let p = Vec3::new(u * period as f32, v * period as f32, 0.37);
        sum += amplitude * (perlin(p, seed.wrapping_add(octave), Some(period)) * 1.8).abs();
        total += amplitude;
        amplitude *= 0.5;
        period *= 2;
    }
    sum / total
}

/// A deterministic float in `0..1` for a lattice cell, one per `channel`.
pub fn lattice_random(x: i32, y: i32, seed: u32, channel: u32) -> f32 {
    (hash(IVec3::new(x, y, channel as i32), seed) >> 8) as f32 / 16_777_216.0
}

/// A deterministic float in `0..1` from an integer stream.
pub fn random(index: u32, stream: u32) -> f32 {
    (hash(IVec3::new(index as i32, stream as i32, 17), 12345) >> 8) as f32 / 16_777_216.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noise_is_deterministic_and_bounded() {
        for i in 0..1000 {
            let p = Vec3::new(i as f32 * 0.137, i as f32 * 0.071, i as f32 * 0.31);
            let value = perlin(p, 3, None);
            assert_eq!(value, perlin(p, 3, None));
            assert!(value.abs() < 1.5);
        }
    }

    #[test]
    fn lattice_random_is_repeatable_and_varies() {
        assert_eq!(lattice_random(3, 4, 5, 6), lattice_random(3, 4, 5, 6));
        assert_ne!(lattice_random(3, 4, 5, 6), lattice_random(3, 4, 5, 7));
        for i in 0..1000 {
            assert!((0.0..1.0).contains(&lattice_random(i, -i, 2, 1)));
        }
    }

    #[test]
    fn stretched_noise_tiles_and_has_its_own_feature_count_each_way() {
        for i in 0..50 {
            let t = i as f32 / 50.0;
            let n = |u, v| tiling_fbm_xy(u, v, 12, 3, 4, 9);
            assert!((n(0.0, t) - n(1.0, t)).abs() < 1.0e-4);
            assert!((n(t, 0.0) - n(t, 1.0)).abs() < 1.0e-4);
        }
        // Changes faster across than down: compare neighbours at the same small step.
        let (mut across, mut down) = (0.0, 0.0);
        for i in 0..400 {
            let (u, v) = ((i % 20) as f32 / 20.0, (i / 20) as f32 / 20.0);
            let n = |u, v| tiling_fbm_xy(u, v, 12, 3, 1, 4);
            across += (n(u + 0.01, v) - n(u, v)).abs();
            down += (n(u, v + 0.01) - n(u, v)).abs();
        }
        assert!(across > down * 2.0, "across {across}, down {down}");
    }

    #[test]
    fn tiling_fbm_matches_across_its_borders() {
        for i in 0..50 {
            let t = i as f32 / 50.0;
            assert!((tiling_fbm(0.0, t, 4, 5, 9) - tiling_fbm(1.0, t, 4, 5, 9)).abs() < 1.0e-4);
            assert!((tiling_fbm(t, 0.0, 4, 5, 9) - tiling_fbm(t, 1.0, 4, 5, 9)).abs() < 1.0e-4);
        }
    }
}
