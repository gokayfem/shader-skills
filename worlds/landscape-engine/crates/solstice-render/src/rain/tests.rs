//! Rain's physics and bookkeeping on the CPU, and its maps and surfaces on a real GPU. The GPU
//! tests pass trivially, with a note, if no GPU is available.

use super::*;

#[test]
fn no_rain_falls_at_rain_zero_and_a_downpour_at_one() {
    assert_eq!(rate_for(0.0), 0.0);
    assert!((rate_for(1.0) - HEAVIEST_RATE).abs() < 1.0e-4);
    let mut last = 0.0;
    for i in 1..=20 {
        let rate = rate_for(i as f32 / 20.0);
        assert!(rate > last);
        last = rate;
    }
    // Moderate rain, a few millimetres an hour, around the middle.
    assert!((3.0..15.0).contains(&rate_for(0.4)), "{}", rate_for(0.4));
}

#[test]
fn drops_follow_marshall_and_palmer() {
    // At 10 mm/h: slope 4.1 R^-0.21 = 2.52 per mm, and N(D > 1 mm) = N0/Λ e^(-Λ) ≈ 255 m^-3.
    let slope = mp_slope(10.0);
    assert!((slope - 2.52).abs() < 0.01, "{slope}");
    let above = drops_above(10.0, 1.0);
    assert!((above - 255.0).abs() < 5.0, "{above}");
    // All drops: N0/Λ.
    assert!((drops_above(10.0, 0.0) - 8000.0 / slope).abs() < 1.0);
    // Heavier rain has more drops of every size, and bigger ones on average.
    for d in [0.5, 1.0, 2.0, 3.0] {
        assert!(drops_above(40.0, d) > drops_above(10.0, d));
        assert!(drops_above(10.0, d) > drops_above(2.0, d));
    }
    assert!(mean_diameter_above(40.0, 1.0) > mean_diameter_above(2.0, 1.0));
    assert_eq!(drops_above(0.0, 1.0), 0.0);
    assert_eq!(visible_density(0.0), 0.0);
    assert!((visible_density(HEAVIEST_RATE) - 1.0).abs() < 1.0e-5);
    assert!(visible_density(5.0) < visible_density(20.0));
}

#[test]
fn drops_fall_at_the_speeds_gunn_and_kinzer_measured() {
    // Diameter in mm, terminal speed in m/s (Gunn and Kinzer 1949).
    for (d, v) in [
        (1.0, 4.03),
        (2.0, 6.49),
        (3.0, 8.06),
        (4.0, 8.83),
        (5.0, 9.09),
    ] {
        let speed = terminal_speed(d);
        assert!(
            (speed - v).abs() / v < 0.04,
            "{d} mm falls at {speed}, measured {v}"
        );
    }
}

fn weather(rain: f32) -> Weather {
    Weather {
        rain,
        ..Weather::by_name("overcast").unwrap()
    }
}

/// The rain state after `seconds` of `weather`, from `state`, in steps of a tenth of a second.
fn after(mut state: RainState, rain: f32, wind: f32, seconds: f32) -> RainState {
    for _ in 0..(seconds * 10.0) as u32 {
        state.advance(&weather(rain), wind, 0.1);
    }
    state
}

#[test]
fn surfaces_soak_in_a_downpour_and_puddles_fill_more_slowly() {
    let start = RainState::default();
    assert!(!start.active());
    let minute = after(start, 1.0, 0.0, 60.0);
    assert!(minute.active());
    assert!(
        minute.wetness > 0.7,
        "only {} wet after a minute",
        minute.wetness
    );
    assert!(minute.puddles < minute.wetness * 0.5, "{}", minute.puddles);
    let long = after(minute, 1.0, 0.0, 900.0);
    assert!(long.wetness > 0.99 && long.puddles > 0.8, "{long:?}");
    // Light rain soaks more slowly, and fills puddles less.
    let drizzle = after(start, 0.15, 0.0, 60.0);
    assert!(drizzle.wetness < minute.wetness);
    let drizzle_long = after(start, 0.15, 0.0, 1800.0);
    assert!(drizzle_long.puddles < long.puddles);
}

#[test]
fn what_the_rain_leaves_dries_after_it_and_faster_in_wind() {
    let soaked = after(RainState::default(), 1.0, 0.0, 900.0);
    let still = after(soaked, 0.0, 0.0, 300.0);
    let windy = after(soaked, 0.0, 10.0, 300.0);
    assert_eq!(still.rate, 0.0);
    assert!(still.wetness < soaked.wetness * 0.5);
    assert!(windy.wetness < still.wetness);
    // Puddles last longer than the wet on the stones.
    assert!(still.puddles / soaked.puddles > still.wetness / soaked.wetness);
    // And at last nothing is left to draw.
    let dry = after(soaked, 0.0, 0.0, 7200.0);
    assert!(!dry.active(), "{dry:?}");
}

#[test]
fn the_cover_map_lies_under_the_camera_from_well_above_it() {
    let camera = Vec3::new(300.0, 12.0, -150.0);
    let view = CoverView::new(camera);
    let half = MAP_TEXELS as f32 * MAP_TEXEL * 0.5;
    let centre = view.origin + Vec2::splat(half);
    assert!((centre - Vec2::new(camera.x, camera.z)).abs().max_element() <= MAP_SNAP);
    assert!(view.top >= camera.y + COVER_ABOVE - MAP_SNAP);
    // Depth runs from the top down; texel (0, 0) is the least x and z.
    let top = view
        .view_proj
        .project_point3(Vec3::new(camera.x, view.top, camera.z));
    assert!(top.z.abs() < 1.0e-4);
    let corner = view
        .view_proj
        .project_point3(Vec3::new(view.origin.x, 0.0, view.origin.y));
    // The least x is at the left, the least z at the top of the texture.
    assert!(
        (corner.x + 1.0).abs() < 1.0e-3 && (corner.y - 1.0).abs() < 1.0e-3,
        "{corner}"
    );
    // It stays put while the camera moves a little.
    assert_eq!(CoverView::new(camera + Vec3::new(3.0, 2.0, 1.0)), view);
}

#[test]
fn the_uniform_lays_out_as_its_shader_twin() {
    crate::shaders::assert_twin!(
        "rain/draw.wgsl",
        "RainParams",
        RainUniform,
        [map, camera, wind, rain, drops, extra, light]
    );
    crate::shaders::assert_twin!("rain/map.wgsl", "MapParams", MapUniform, [top, reach, _pad]);
}

#[test]
fn the_rains_clock_starts_afresh_with_each_rain() {
    let soaked = after(RainState::default(), 1.0, 0.0, 600.0);
    assert!(soaked.time > 590.0);
    let dried = after(soaked, 0.0, 0.0, 7200.0);
    assert!(!dried.active());
    let again = after(dried, 1.0, 0.0, 1.0);
    // A second of the new rain, not hours since the first began.
    assert!(again.time < 1.5, "{}", again.time);
}
