//! Between the player's hands and the car: what a key or a stick is turned into.
//!
//! A steering wheel turns through more than a turn, and a pedal through a foot's travel. A key
//! is up or down, and a thumbstick moves two centimetres. Handing either to the car as it comes
//! would make it undrivable, so every driving game puts something between them:
//!
//! * **Steering eases** towards what is asked rather than jumping there, and the faster the car
//!   goes the less lock a full push gives: just past the angle at which the front tyres grip
//!   best at that speed. A full push then asks for all the cornering there is, and no more.
//! * **The wheel follows a slide.** A real steering wheel is pulled round by the front tyres
//!   towards the way the car is going: let go of it in a slide and it steers into the slide by
//!   itself, and a driver's hands follow it without thinking. Keys and sticks have no such
//!   pull, so it is done here: past a few degrees of slide, the wheel turns towards where the
//!   car is going, by a little more than it has slid.
//! * **Pedals ease** too: a key pressed is a foot going down, in a fifth of a second.
//! * **The gearbox** can change gear by itself, and finds reverse when the car is held on the
//!   brake at a standstill.
//!
//! None of it alters what the car does: it only decides what the wheel and pedals are doing.

use crate::spec::{AIR_DENSITY, RPM_TO_RADIANS};
use crate::vehicle::{Body, Controls, Vehicle};

/// What the player is asking for, straight from the keys or the pad.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Asked {
    /// From 0 to 1.
    pub throttle: f32,
    pub brake: f32,
    /// From -1, left, to 1, right.
    pub steer: f32,
    pub handbrake: f32,
    /// Pressed since the step before.
    pub shift_up: bool,
    pub shift_down: bool,
}

/// How fast the steering follows a key, in locks a second: turning in, and coming back.
const STEER_IN: f32 = 3.0;
const STEER_BACK: f32 = 5.0;
/// And a stick, which the thumb has already eased.
const STEER_STICK: f32 = 9.0;
/// How far the car may point from the way it is going before the wheel follows the slide, in
/// radians: cornering hard, a car is a few degrees round already.
const SLIDE_FREE: f32 = 0.06;
/// How much of the slide beyond that the wheel takes up.
const SLIDE_FOLLOWED: f32 = 1.3;
/// How fast a pedal follows a key, in travels a second: going down, and coming up.
const PEDAL_DOWN: f32 = 5.5;
const PEDAL_UP: f32 = 11.0;
/// How long the automatic gearbox waits after a change before it makes another.
const SHIFT_REST: f32 = 0.35;

/// The driver's hands and feet.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Driver {
    /// Whether the gearbox changes gear by itself.
    pub automatic: bool,
    steer: f32,
    throttle: f32,
    brake: f32,
    /// Whether the last steering asked for was part of the way: a stick, not a key.
    stick: bool,
    /// Whether the automatic gearbox has the car going backwards.
    reversing: bool,
    since_shift: f32,
    /// How long the car has been held still on the brake.
    held_still: f32,
}

impl Default for Driver {
    fn default() -> Self {
        Self::new(true)
    }
}

impl Driver {
    pub fn new(automatic: bool) -> Self {
        Self {
            automatic,
            steer: 0.0,
            throttle: 0.0,
            brake: 0.0,
            stick: false,
            reversing: false,
            since_shift: 1.0,
            held_still: 0.0,
        }
    }

    /// The share of the steering lock a full push gives at this speed, in metres a second.
    ///
    /// A car turning a circle of radius `r` points its front wheels in by about `wheelbase / r`,
    /// and the tightest circle it can hold at a speed is `speed² / (grip × g)`. On top of that
    /// goes the slip angle the tyres grip best at, and a little more to feel the edge.
    pub fn lock_share(vehicle: &Vehicle, speed: f32, gravity: f32) -> f32 {
        let spec = vehicle.spec();
        let downforce = 0.5 * AIR_DENSITY * spec.aero.downforce * speed * speed;
        let cornering = spec.tyre.grip_lat * (gravity + downforce / spec.mass);
        let speed_squared = (speed * speed).max(1.0);
        let turn = (spec.wheelbase() * cornering / speed_squared).atan();
        ((turn + spec.tyre.peak_angle * 1.25) / spec.steering_lock).min(1.0)
    }

    /// What the wheel and pedals do this step, for what is asked.
    pub fn drive(
        &mut self,
        asked: &Asked,
        vehicle: &Vehicle,
        body: &Body,
        gravity: f32,
        seconds: f32,
    ) -> Controls {
        let speed = body.velocity.dot(body.forward());
        let ease = |now: f32, target: f32, up: f32, down: f32| {
            let rate = if target.abs() > now.abs() && target * now >= 0.0 {
                up
            } else {
                down
            };
            now + (target - now).clamp(-rate * seconds, rate * seconds)
        };

        // Steering.
        let wanted = asked.steer.clamp(-1.0, 1.0);
        if wanted != 0.0 {
            self.stick = wanted.abs() < 0.999;
        }
        // A heavy car's wheel is turned more slowly, and a heavy car's pedals pressed more
        // gently: its driver has more to move.
        let quick = vehicle.spec().steering_quick;
        self.steer = if self.stick {
            ease(self.steer, wanted, STEER_STICK * quick, STEER_STICK * quick)
        } else {
            ease(self.steer, wanted, STEER_IN * quick, STEER_BACK * quick)
        };
        let mut steer = self.steer * Self::lock_share(vehicle, speed.abs(), gravity);
        // Following a slide: the wheel turns towards the way the car is going.
        if speed > 6.0 {
            let sideways = body.velocity.dot(body.right());
            let slide = sideways.atan2(speed);
            let followed = (slide.abs() - SLIDE_FREE).max(0.0) * slide.signum() * SLIDE_FOLLOWED;
            steer = (steer + followed / vehicle.spec().steering_lock).clamp(-1.0, 1.0);
        }

        // Pedals. Going backwards under the automatic gearbox, the brake pedal drives.
        let (mut go, mut stop) = (asked.throttle.clamp(0.0, 1.0), asked.brake.clamp(0.0, 1.0));
        let (mut shift_up, mut shift_down) = (asked.shift_up, asked.shift_down);
        if self.automatic {
            (shift_up, shift_down) = self.change_gear(vehicle, speed, go, stop, seconds);
            if self.reversing {
                std::mem::swap(&mut go, &mut stop);
            }
            // Between gears on the way to first or reverse, a foot stays off: the engine
            // would only race in neutral, and the clutch then take up with a bang.
            if vehicle.gear() == 0 {
                go = 0.0;
            }
        }
        self.throttle = ease(self.throttle, go, PEDAL_DOWN * quick.min(1.0), PEDAL_UP);
        self.brake = ease(self.brake, stop, PEDAL_DOWN * 1.6, PEDAL_UP);

        Controls {
            throttle: self.throttle,
            brake: self.brake,
            steer,
            handbrake: asked.handbrake.clamp(0.0, 1.0),
            shift_up,
            shift_down,
        }
    }

    /// The automatic gearbox: whether to change up, or down, this step.
    fn change_gear(
        &mut self,
        vehicle: &Vehicle,
        speed: f32,
        go: f32,
        stop: f32,
        seconds: f32,
    ) -> (bool, bool) {
        self.since_shift += seconds;
        let still = speed.abs() < 0.6;
        self.held_still = if still && stop > 0.1 && go < 0.05 {
            self.held_still + seconds
        } else {
            0.0
        };
        // Held on the brake at a standstill, it finds reverse; asked to go again, first.
        if !self.reversing && self.held_still > 0.35 {
            self.reversing = true;
            self.held_still = 0.0;
        } else if self.reversing && go > 0.1 && speed > -0.6 {
            self.reversing = false;
        }
        if vehicle.shifting() || self.since_shift < SHIFT_REST {
            return (false, false);
        }
        let gear = vehicle.gear();
        let mut change = |up: bool| {
            self.since_shift = 0.0;
            (up, !up)
        };
        if self.reversing {
            return if gear > -1 {
                change(false)
            } else {
                (false, false)
            };
        }
        if gear < 1 {
            return change(true);
        }

        let spec = vehicle.spec();
        let gears = &spec.gearbox.gears;
        let (rpm, limit) = (vehicle.rpm(), spec.engine.limit);
        let top = gears.len() as i32;
        // Up, just before the limiter, unless the wheels are spinning the revs up. On loose
        // ground a tyre that is gripping slips more, so it is held to what the ground allows.
        let driven: &[usize] = match (spec.front.driven, spec.rear.driven) {
            (true, true) => &[0, 1, 2, 3],
            (true, false) => &[0, 1],
            _ => &[2, 3],
        };
        let gripping = driven.iter().all(|&index| {
            let wheel = &vehicle.wheels[index];
            wheel.slip_ratio.abs() < 0.2 * wheel.surface.grip().slip
        });
        // A gearbox that cruises changes up early on a light throttle, and later the further
        // the pedal is down.
        let cruise = spec.gearbox.cruise;
        let up_at = (limit - 330.0) * (cruise + (1.0 - cruise) * go.powf(1.5));
        // A gearbox that cruises goes by the road's speed, as a road car's does, not the
        // engine's: with a wheel spinning in mud, or the clutch slipping against a ledge, the
        // engine's speed says nothing of what gear the car should be in.
        let on_road = |gear: i32| speed.abs() / speed_in_gear(vehicle, gear as usize, 1.0);
        let rpm = if cruise < 1.0 { on_road(gear) } else { rpm };
        if gear < top && rpm > up_at && go > 0.05 && gripping {
            return change(true);
        }
        // Down, when the engine has fallen out of its power, sooner under braking or with the
        // throttle flat, and never to where it would over-rev.
        if gear > 1 {
            let lower = rpm * gears[gear as usize - 2] / gears[gear as usize - 1];
            let below = if cruise < 1.0 {
                // Well under where it would change straight back up, so that it never hunts.
                if stop > 0.1 {
                    up_at.max(limit * 0.5) * 0.7
                } else {
                    up_at * 0.7
                }
            } else if stop > 0.1 {
                limit * 0.76
            } else if go > 0.9 {
                limit * 0.70
            } else {
                limit * 0.47
            };
            if lower < below.max(spec.engine.idle * 2.5) && lower < limit - 900.0 {
                return change(false);
            }
        }
        (false, false)
    }
}

/// The speed, in metres a second, the car does at an engine speed in a gear (1 is first).
pub fn speed_in_gear(vehicle: &Vehicle, gear: usize, rpm: f32) -> f32 {
    let spec = vehicle.spec();
    let axle = if spec.front.driven {
        &spec.front
    } else {
        &spec.rear
    };
    let ratio = spec.gearbox.gears[gear - 1] * spec.gearbox.final_drive * vehicle.range();
    rpm * RPM_TO_RADIANS / ratio * axle.radius
}
