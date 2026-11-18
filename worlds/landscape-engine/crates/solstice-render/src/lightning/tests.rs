//! The storm's lightning, on the CPU: when strikes come, how a flash goes, and its thunder.

use super::*;

fn storm(thunder: f32) -> Weather {
    Weather {
        thunder,
        ..Weather::by_name("storm").unwrap()
    }
}

/// Runs `seconds` of `weather` in steps of 1/60 s, gathering every strike's first frame and all
/// thunder.
fn run(lightning: &mut Lightning, weather: &Weather, seconds: f32) -> (Vec<Strike>, Vec<Thunder>) {
    let mut strikes = Vec::new();
    let mut thunder = Vec::new();
    for _ in 0..(seconds * 60.0) as u32 {
        let before = lightning.strike().map(|s| s.age);
        lightning.advance(weather, Vec3::ZERO, 1.0 / 60.0);
        if let Some(strike) = lightning.strike() {
            if before.is_none() || strike.age < before.unwrap_or(0.0) {
                strikes.push(strike.clone());
            }
        }
        thunder.extend(lightning.take_thunder());
    }
    (strikes, thunder)
}

#[test]
fn no_thunder_no_lightning() {
    let mut lightning = Lightning::default();
    let (strikes, thunder) = run(&mut lightning, &storm(0.0), 600.0);
    assert!(strikes.is_empty() && thunder.is_empty());
    assert!(lightning.flash(Vec3::ZERO).is_none());
    assert!(!lightning.holds_exposure());
}

#[test]
fn strikes_come_about_as_often_as_the_weather_says() {
    let mut lightning = Lightning::new(7);
    let (_, thunder) = run(&mut lightning, &storm(4.0), 1200.0);
    // Four a minute for twenty minutes, less the time each flash holds the sky.
    let count = thunder.len() as f32;
    assert!(
        (55.0..95.0).contains(&count),
        "{count} strikes in 20 minutes"
    );
}

#[test]
fn thunder_follows_the_flash_by_its_distance_at_the_speed_of_sound() {
    let mut lightning = Lightning::new(3);
    let (strikes, thunder) = run(&mut lightning, &storm(6.0), 300.0);
    assert!(!thunder.is_empty() && strikes.len() == thunder.len());
    for (strike, sound) in strikes.iter().zip(&thunder) {
        assert!((NEAREST..=FARTHEST).contains(&strike.distance));
        // Heard a frame after it was queued: within a frame of distance / 343 m/s.
        let expected = strike.distance / SPEED_OF_SOUND;
        assert!(
            (sound.delay - expected).abs() < 1.0 / 30.0,
            "{} {}",
            sound.delay,
            expected
        );
        assert_eq!(sound.to_ground, strike.kind == Kind::ToGround);
        assert!(sound.loudness > 0.0 && sound.loudness <= 1.0);
    }
    // Far thunder rolls longer and is quieter.
    let near = thunder
        .iter()
        .min_by(|a, b| a.distance.total_cmp(&b.distance))
        .unwrap();
    let far = thunder
        .iter()
        .max_by(|a, b| a.distance.total_cmp(&b.distance))
        .unwrap();
    assert!(far.rumble > near.rumble && far.loudness < near.loudness);
}

#[test]
fn a_flash_to_the_ground_is_a_few_strokes_tens_of_milliseconds_apart() {
    let mut lightning = Lightning::new(11);
    let (strikes, _) = run(&mut lightning, &storm(8.0), 600.0);
    let grounded: Vec<&Strike> = strikes
        .iter()
        .filter(|s| s.kind == Kind::ToGround)
        .collect();
    let in_cloud = strikes.len() - grounded.len();
    assert!(
        !grounded.is_empty() && in_cloud > grounded.len(),
        "most flashes stay in the cloud"
    );
    for strike in grounded {
        assert!((1..=5).contains(&strike.strokes.len()));
        for pair in strike.strokes.windows(2) {
            let gap = pair[1].0 - pair[0].0;
            assert!((0.03..=0.09).contains(&gap), "strokes {gap} s apart");
        }
        // Each stroke flashes, then fades within tens of milliseconds.
        let (start, _) = strike.strokes[0];
        let peak = strike.brightness_at(start + 0.004);
        assert!(peak > 0.9);
        let last = strike.strokes.last().unwrap().0;
        assert!(strike.brightness_at(last + 0.6) < 0.05);
        assert!(strike.brightness_at(LONGEST) < 0.01);
        // Its bolt runs from the cloud base to the ground.
        let top = strike.bolt.iter().map(|s| s.a.y).fold(f32::MIN, f32::max);
        let bottom = strike.bolt.iter().map(|s| s.b.y).fold(f32::MAX, f32::min);
        assert!(top >= 300.0 && bottom <= 0.0, "{top} {bottom}");
        assert!(strike.bolt.len() <= MAX_SEGMENTS);
    }
}

#[test]
fn a_nearer_flash_lights_the_scene_more() {
    let strike = |distance: f32| Strike {
        kind: Kind::ToGround,
        at: Vec3::new(distance, 1000.0, 0.0),
        distance,
        strokes: vec![(0.0, 1.0)],
        bolt: Vec::new(),
        age: 0.004,
    };
    let (near, far) = (strike(1000.0).illuminance(), strike(4000.0).illuminance());
    assert!((near / far - 16.0).abs() < 0.5, "{near} {far}");
    // A flash overhead is bright, but never brighter than the brightest allowed.
    assert!(strike(10.0).illuminance() <= BRIGHTEST);
}

#[test]
fn a_strike_can_be_asked_for() {
    let mut lightning = Lightning::default();
    lightning.strike_soon();
    lightning.advance(&storm(0.0), Vec3::ZERO, 1.0 / 60.0);
    assert!(lightning.strike().is_some() && lightning.holds_exposure());
    assert_eq!(lightning.take_thunder().len(), 1);
    // A stroke takes a few milliseconds to brighten.
    lightning.advance(&storm(0.0), Vec3::ZERO, 1.0 / 60.0);
    let flash = lightning.flash(Vec3::ZERO).expect("a flash");
    assert!(flash.illuminance > 0.0 && (flash.direction.length() - 1.0).abs() < 1.0e-4);
}

#[test]
fn weather_reads_and_writes_its_thunder() {
    let w = Weather::parse("thunder 2.5\n").unwrap();
    assert_eq!(w.thunder, 2.5);
    assert_eq!(Weather::parse(&w.to_text()).unwrap(), w);
    assert!(Weather::by_name("storm").unwrap().thunder > 0.0);
    assert_eq!(Weather::by_name("fair").unwrap().thunder, 0.0);
}

#[test]
fn a_strike_asked_for_falls_ahead() {
    for (seed, facing) in [
        (1, Vec3::X),
        (2, Vec3::NEG_Z),
        (3, Vec3::new(-0.6, -0.2, 0.77)),
    ] {
        let mut lightning = Lightning::new(seed);
        lightning.strike_soon();
        lightning.advance_facing(&storm(0.0), Vec3::ZERO, facing.normalize(), 1.0 / 60.0);
        let strike = lightning.strike().expect("a strike");
        assert_eq!(strike.kind, Kind::ToGround);
        let way = Vec2::new(strike.at.x, strike.at.z).normalize();
        let ahead = Vec2::new(facing.x, facing.z).normalize();
        assert!(
            way.angle_to(ahead).abs() <= 0.5 + 1.0e-4,
            "{way} vs {ahead}"
        );
    }
}

#[test]
fn a_far_faint_flash_holds_exposure_only_briefly() {
    let mut lightning = Lightning::new(5);
    lightning.strike_soon();
    lightning.advance(&storm(0.0), Vec3::ZERO, 1.0 / 60.0);
    assert!(lightning.holds_exposure());
    // Long after its last stroke, while it is still dying away, it no longer does.
    for _ in 0..80 {
        lightning.advance(&storm(0.0), Vec3::ZERO, 1.0 / 60.0);
    }
    assert!(lightning.strike().is_some() && !lightning.holds_exposure());
}
