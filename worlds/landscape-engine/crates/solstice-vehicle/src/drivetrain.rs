//! The turning parts: engine, clutch, gearbox, differential, brakes and wheels, solved together.
//!
//! Three things turn on a driven axle: the engine and the two wheels. They are joined by parts
//! that grip by friction: the clutch between the engine and the gearbox, the differential's
//! clutch packs between the two wheels, and a brake on each wheel. A friction joint holds its
//! two sides together until it is asked for more torque than it can carry, and then slips,
//! carrying just that much.
//!
//! Each joint is modelled as very stiff until it reaches its limit: its torque is its stiffness
//! times the difference in speed across it, and no more than the limit ([`Coupling`]). A
//! held joint is far too stiff to step forward in time the plain way, a millisecond at a time,
//! so the speeds at the *end* of the step are solved for instead (backward Euler): three
//! equations in three unknowns, which never overshoots however stiff the joint. A joint found
//! over its limit is then set slipping at that limit, and the speeds solved again.
//!
//! The tyres are part of it: each one's force on its wheel changes steeply with the wheel's
//! speed, so it is in the equations too, by its slope.

/// A friction joint: how much torque it can carry before it slips.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Coupling {
    pub limit: f32,
}

impl Coupling {
    pub fn new(limit: f32) -> Self {
        Self {
            limit: limit.max(0.0),
        }
    }
}

/// How stiff a held joint is, in newton metres for each radian a second across it. Stiff enough
/// that a held joint creeps by a thousandth of a turn a second at its limit.
const CLUTCH_STIFFNESS: f64 = 400.0;
const DIFFERENTIAL_STIFFNESS: f64 = 3000.0;
const BRAKE_STIFFNESS: f64 = 60_000.0;
/// The most passes the solve makes. Four joints settle in a few; the cap is for a joint on the
/// very edge of its limit, which either answer suits.
const PASSES: usize = 12;

/// One wheel, as the drivetrain sees it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Wheel {
    /// Radians a second.
    pub speed: f32,
    pub inertia: f32,
    pub radius: f32,
    /// The road's push on the tyre along the way it points, at the wheel's speed now.
    pub force: f32,
    /// How that push grows for each radian a second faster the wheel turns. Never below zero.
    pub stiffness: f32,
    /// The most torque its brake can make now.
    pub brake: f32,
}

/// The driven axle and what drives it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DrivenAxle {
    pub engine_speed: f32,
    pub engine_inertia: f32,
    /// What the engine makes now, at the flywheel: below zero on the overrun.
    pub engine_torque: f32,
    /// From the engine to the wheels, with the final drive: below zero in reverse, and zero in
    /// neutral.
    pub ratio: f32,
    pub efficiency: f32,
    pub clutch: Coupling,
    pub differential: Coupling,
    pub left: Wheel,
    pub right: Wheel,
}

/// The speeds at the end of the step.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Solved {
    pub engine: f32,
    pub left: f32,
    pub right: f32,
    /// The torque the clutch carried, from the engine to the gearbox.
    pub clutch_torque: f32,
}

/// A joint while solving: held, with its torque following the speeds, or slipping at a torque.
#[derive(Clone, Copy)]
enum Joint {
    Held,
    Slipping(f64),
}

/// A wheel that nothing drives: its speed after `dt`, under its brake and the road.
pub fn solve_free(wheel: &Wheel, dt: f32) -> f32 {
    let dt = f64::from(dt);
    let (speed, inertia, radius) = (
        f64::from(wheel.speed),
        f64::from(wheel.inertia),
        f64::from(wheel.radius),
    );
    let slope = radius * f64::from(wheel.stiffness);
    let limit = f64::from(wheel.brake);
    let momentum = inertia / dt * speed - radius * f64::from(wheel.force) + slope * speed;
    // Held by the brake first; if that asks more than the brake has, slipping against it.
    let held = momentum / (inertia / dt + slope + BRAKE_STIFFNESS);
    let asked = BRAKE_STIFFNESS * held;
    if limit <= 0.0 || asked.abs() > limit {
        let brake = if limit <= 0.0 {
            0.0
        } else {
            limit * asked.signum()
        };
        return ((momentum - brake) / (inertia / dt + slope)) as f32;
    }
    held as f32
}

/// The driven axle's speeds after `dt`.
pub fn solve(axle: &DrivenAxle, dt: f32) -> Solved {
    let dt = f64::from(dt);
    let ratio = f64::from(axle.ratio);
    let drive = ratio * f64::from(axle.efficiency) * 0.5;
    let wheels = [axle.left, axle.right];
    let mut limits = [
        f64::from(axle.clutch.limit),
        f64::from(axle.differential.limit),
        f64::from(wheels[0].brake),
        f64::from(wheels[1].brake),
    ];
    // In neutral nothing joins the engine to the wheels, whatever the clutch does.
    let mut joints = limits.map(|limit| {
        if limit > 0.0 {
            Joint::Held
        } else {
            Joint::Slipping(0.0)
        }
    });
    if axle.ratio == 0.0 {
        joints[0] = Joint::Slipping(0.0);
        limits[0] = 0.0;
    }

    let mut speeds = [0.0_f64; 3];
    let mut torques = [0.0_f64; 4];
    // Each pass sets slipping the one held joint asked for the most beyond its limit, and
    // holds again any slipping joint whose two sides the answer has passing each other the
    // other way. One at a time: a brake that turns out to slip changes which way the clutch
    // must, so deciding them together from one answer gets the clutch backwards.
    for _ in 0..PASSES {
        // Rows: the engine, the left wheel, the right wheel. Unknowns: their speeds after the
        // step. Each row is `inertia / dt * speed + what resists = inertia / dt * old + what
        // drives`.
        let mut a = [[0.0_f64; 3]; 3];
        let mut b = [0.0_f64; 3];
        let engine_inertia = f64::from(axle.engine_inertia);
        a[0][0] = engine_inertia / dt;
        b[0] = engine_inertia / dt * f64::from(axle.engine_speed) + f64::from(axle.engine_torque);
        for (side, wheel) in wheels.iter().enumerate() {
            let row = side + 1;
            let (inertia, radius) = (f64::from(wheel.inertia), f64::from(wheel.radius));
            let slope = radius * f64::from(wheel.stiffness);
            a[row][row] = inertia / dt + slope;
            b[row] = inertia / dt * f64::from(wheel.speed) - radius * f64::from(wheel.force)
                + slope * f64::from(wheel.speed);
        }
        // The clutch: its torque slows the engine, and through the gears turns each wheel.
        match joints[0] {
            Joint::Held => {
                // torque = stiffness * (engine - ratio * (left + right) / 2)
                let k = CLUTCH_STIFFNESS;
                let across = [1.0, -ratio * 0.5, -ratio * 0.5];
                for (column, weight) in across.iter().enumerate() {
                    a[0][column] += k * weight;
                    a[1][column] -= drive * k * weight;
                    a[2][column] -= drive * k * weight;
                }
            }
            Joint::Slipping(torque) => {
                b[0] -= torque;
                b[1] += drive * torque;
                b[2] += drive * torque;
            }
        }
        // The differential: its torque slows the faster wheel and speeds the slower.
        match joints[1] {
            Joint::Held => {
                let k = DIFFERENTIAL_STIFFNESS;
                a[1][1] += k;
                a[1][2] -= k;
                a[2][2] += k;
                a[2][1] -= k;
            }
            Joint::Slipping(torque) => {
                b[1] -= torque;
                b[2] += torque;
            }
        }
        for side in 0..2 {
            let row = side + 1;
            match joints[2 + side] {
                Joint::Held => a[row][row] += BRAKE_STIFFNESS,
                Joint::Slipping(torque) => b[row] -= torque,
            }
        }
        speeds = solve3(a, b);

        // How fast each joint's two sides pass each other, and what it would take to hold them.
        let across = [
            speeds[0] - ratio * 0.5 * (speeds[1] + speeds[2]),
            speeds[1] - speeds[2],
            speeds[1],
            speeds[2],
        ];
        let stiffness = [
            CLUTCH_STIFFNESS,
            DIFFERENTIAL_STIFFNESS,
            BRAKE_STIFFNESS,
            BRAKE_STIFFNESS,
        ];
        let mut worst: Option<(usize, f64)> = None;
        let mut caught = false;
        for index in 0..4 {
            match joints[index] {
                Joint::Held => {
                    torques[index] = stiffness[index] * across[index];
                    let over = torques[index].abs() / limits[index];
                    if over > 1.0 && worst.is_none_or(|(_, most)| over > most) {
                        worst = Some((index, over));
                    }
                }
                Joint::Slipping(torque) => {
                    torques[index] = torque;
                    // Friction only ever works against the slip.
                    if limits[index] > 0.0 && torque * across[index] < 0.0 {
                        joints[index] = Joint::Held;
                        caught = true;
                    }
                }
            }
        }
        if caught {
            continue;
        }
        match worst {
            Some((index, _)) => {
                joints[index] = Joint::Slipping(limits[index] * torques[index].signum());
            }
            None => break,
        }
    }
    Solved {
        engine: speeds[0] as f32,
        left: speeds[1] as f32,
        right: speeds[2] as f32,
        clutch_torque: torques[0] as f32,
    }
}

/// Solves three equations in three unknowns, by elimination with the largest pivot. The
/// equations here always have an answer: every row has its inertia on the diagonal.
// Rows and columns are numbers here, as they are in the algebra.
#[allow(clippy::needless_range_loop)]
fn solve3(mut a: [[f64; 3]; 3], mut b: [f64; 3]) -> [f64; 3] {
    for column in 0..3 {
        let pivot = (column..3)
            .max_by(|&x, &y| a[x][column].abs().total_cmp(&a[y][column].abs()))
            .unwrap_or(column);
        a.swap(column, pivot);
        b.swap(column, pivot);
        for row in column + 1..3 {
            let factor = a[row][column] / a[column][column];
            for k in column..3 {
                a[row][k] -= factor * a[column][k];
            }
            b[row] -= factor * b[column];
        }
    }
    let mut x = [0.0; 3];
    for row in (0..3).rev() {
        let known: f64 = (row + 1..3).map(|k| a[row][k] * x[k]).sum();
        x[row] = (b[row] - known) / a[row][row];
    }
    x
}

/// How stiff the lock between the two propshafts of a four-wheel drive is, held.
const CENTRE_STIFFNESS: f64 = 20_000.0;
/// The most passes the four-wheel solve makes: it has twice the joints.
const PASSES_ALL: usize = 24;

/// Both axles and what drives them: a four-wheel drive.
///
/// Five things turn: the engine and four wheels. The clutch joins the engine to the transfer
/// case, which shares the drive between the two propshafts. A lock between the propshafts
/// holds the two axles to one speed, as a part-time transfer case does once engaged; without
/// it, and with nothing sent forward, the front wheels roll free and the car is a rear-wheel
/// drive. Each axle has a differential, and each wheel a brake.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Driveline {
    pub engine_speed: f32,
    pub engine_inertia: f32,
    pub engine_torque: f32,
    /// From the engine to the wheels, with the final drive and the transfer case's range:
    /// below zero in reverse, and zero in neutral.
    pub ratio: f32,
    pub efficiency: f32,
    pub clutch: Coupling,
    /// The share of the drive the front axle is sent: none in two-wheel drive.
    pub front_share: f32,
    /// The lock between the front and rear propshafts.
    pub centre: Coupling,
    /// Each axle's differential.
    pub front: Coupling,
    pub rear: Coupling,
    /// Front left, front right, rear left, rear right.
    pub wheels: [Wheel; 4],
}

/// The speeds at the end of the step, for a four-wheel drive.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SolvedAll {
    pub engine: f32,
    pub wheels: [f32; 4],
    /// The torque the clutch carried, from the engine to the gearbox.
    pub clutch_torque: f32,
}

/// A four-wheel drive's speeds after `dt`: [`solve`], with both axles and what joins them.
pub fn solve_all(line: &Driveline, dt: f32) -> SolvedAll {
    let dt = f64::from(dt);
    let ratio = f64::from(line.ratio);
    let front_share = f64::from(line.front_share.clamp(0.0, 1.0));
    // How much of the propshafts' speed, and of the drive, each wheel is.
    let weight = [
        front_share * 0.5,
        front_share * 0.5,
        (1.0 - front_share) * 0.5,
        (1.0 - front_share) * 0.5,
    ];
    let drive = ratio * f64::from(line.efficiency);
    // The joints: the clutch, the lock between the axles, each differential, each brake.
    let mut limits = [
        f64::from(line.clutch.limit),
        f64::from(line.centre.limit),
        f64::from(line.front.limit),
        f64::from(line.rear.limit),
        f64::from(line.wheels[0].brake),
        f64::from(line.wheels[1].brake),
        f64::from(line.wheels[2].brake),
        f64::from(line.wheels[3].brake),
    ];
    let stiffness = [
        CLUTCH_STIFFNESS,
        CENTRE_STIFFNESS,
        DIFFERENTIAL_STIFFNESS,
        DIFFERENTIAL_STIFFNESS,
        BRAKE_STIFFNESS,
        BRAKE_STIFFNESS,
        BRAKE_STIFFNESS,
        BRAKE_STIFFNESS,
    ];
    let mut joints = limits.map(|limit| {
        if limit > 0.0 {
            Joint::Held
        } else {
            Joint::Slipping(0.0)
        }
    });
    if line.ratio == 0.0 {
        joints[0] = Joint::Slipping(0.0);
        limits[0] = 0.0;
    }
    // What each joint's two sides pass each other by, as weights on the five speeds: the
    // engine, then the wheels.
    let across: [[f64; 5]; 8] = [
        [
            1.0,
            -ratio * weight[0],
            -ratio * weight[1],
            -ratio * weight[2],
            -ratio * weight[3],
        ],
        [0.0, 0.5, 0.5, -0.5, -0.5],
        [0.0, 1.0, -1.0, 0.0, 0.0],
        [0.0, 0.0, 0.0, 1.0, -1.0],
        [0.0, 1.0, 0.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 0.0, 1.0],
    ];
    // And what a joint's torque does to each of the five: the clutch slows the engine and
    // turns the wheels through the gears; the centre lock slows the faster axle and speeds
    // the slower; a differential, its faster wheel; a brake, its wheel.
    let applied: [[f64; 5]; 8] = [
        [
            -1.0,
            drive * weight[0],
            drive * weight[1],
            drive * weight[2],
            drive * weight[3],
        ],
        [0.0, -0.5, -0.5, 0.5, 0.5],
        [0.0, -1.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 0.0, -1.0, 1.0],
        [0.0, -1.0, 0.0, 0.0, 0.0],
        [0.0, 0.0, -1.0, 0.0, 0.0],
        [0.0, 0.0, 0.0, -1.0, 0.0],
        [0.0, 0.0, 0.0, 0.0, -1.0],
    ];

    let mut speeds = [0.0_f64; 5];
    let mut torques = [0.0_f64; 8];
    for _ in 0..PASSES_ALL {
        let mut a = [[0.0_f64; 5]; 5];
        let mut b = [0.0_f64; 5];
        let engine_inertia = f64::from(line.engine_inertia);
        a[0][0] = engine_inertia / dt;
        b[0] = engine_inertia / dt * f64::from(line.engine_speed) + f64::from(line.engine_torque);
        for (index, wheel) in line.wheels.iter().enumerate() {
            let row = index + 1;
            let (inertia, radius) = (f64::from(wheel.inertia), f64::from(wheel.radius));
            let slope = radius * f64::from(wheel.stiffness);
            a[row][row] = inertia / dt + slope;
            b[row] = inertia / dt * f64::from(wheel.speed) - radius * f64::from(wheel.force)
                + slope * f64::from(wheel.speed);
        }
        for joint in 0..8 {
            match joints[joint] {
                // torque = stiffness * across, and it acts on each row as `applied` says: on
                // the left of the equations, with its sign turned.
                Joint::Held => {
                    for row in 0..5 {
                        if applied[joint][row] == 0.0 {
                            continue;
                        }
                        for column in 0..5 {
                            a[row][column] -=
                                applied[joint][row] * stiffness[joint] * across[joint][column];
                        }
                    }
                }
                Joint::Slipping(torque) => {
                    for row in 0..5 {
                        b[row] += applied[joint][row] * torque;
                    }
                }
            }
        }
        speeds = solve_n(a, b);

        let mut worst: Option<(usize, f64)> = None;
        let mut caught = false;
        for joint in 0..8 {
            let passing: f64 = (0..5).map(|k| across[joint][k] * speeds[k]).sum();
            match joints[joint] {
                Joint::Held => {
                    torques[joint] = stiffness[joint] * passing;
                    let over = torques[joint].abs() / limits[joint];
                    if over > 1.0 && worst.is_none_or(|(_, most)| over > most) {
                        worst = Some((joint, over));
                    }
                }
                Joint::Slipping(torque) => {
                    torques[joint] = torque;
                    if limits[joint] > 0.0 && torque * passing < 0.0 {
                        joints[joint] = Joint::Held;
                        caught = true;
                    }
                }
            }
        }
        if caught {
            continue;
        }
        match worst {
            Some((joint, _)) => {
                joints[joint] = Joint::Slipping(limits[joint] * torques[joint].signum());
            }
            None => break,
        }
    }
    SolvedAll {
        engine: speeds[0] as f32,
        wheels: [
            speeds[1] as f32,
            speeds[2] as f32,
            speeds[3] as f32,
            speeds[4] as f32,
        ],
        clutch_torque: torques[0] as f32,
    }
}

/// Solves five equations in five unknowns, as [`solve3`] does three.
#[allow(clippy::needless_range_loop)]
fn solve_n(mut a: [[f64; 5]; 5], mut b: [f64; 5]) -> [f64; 5] {
    const N: usize = 5;
    for column in 0..N {
        let pivot = (column..N)
            .max_by(|&x, &y| a[x][column].abs().total_cmp(&a[y][column].abs()))
            .unwrap_or(column);
        a.swap(column, pivot);
        b.swap(column, pivot);
        for row in column + 1..N {
            let factor = a[row][column] / a[column][column];
            for k in column..N {
                a[row][k] -= factor * a[column][k];
            }
            b[row] -= factor * b[column];
        }
    }
    let mut x = [0.0; N];
    for row in (0..N).rev() {
        let known: f64 = (row + 1..N).map(|k| a[row][k] * x[k]).sum();
        x[row] = (b[row] - known) / a[row][row];
    }
    x
}

#[cfg(test)]
mod tests {
    use super::*;

    const DT: f32 = 1.0 / 600.0;

    fn wheel(speed: f32) -> Wheel {
        Wheel {
            speed,
            inertia: 1.9,
            radius: 0.345,
            force: 0.0,
            stiffness: 0.0,
            brake: 0.0,
        }
    }

    fn axle(engine_speed: f32, wheel_speed: f32) -> DrivenAxle {
        DrivenAxle {
            engine_speed,
            engine_inertia: 0.13,
            engine_torque: 0.0,
            ratio: 3.15 * 3.7,
            efficiency: 1.0,
            clutch: Coupling::new(900.0),
            differential: Coupling::new(0.0),
            left: wheel(wheel_speed),
            right: wheel(wheel_speed),
        }
    }

    #[test]
    fn three_equations_are_solved() {
        let x = solve3(
            [[2.0, 1.0, -1.0], [-3.0, -1.0, 2.0], [-2.0, 1.0, 2.0]],
            [8.0, -11.0, -3.0],
        );
        for (found, expected) in x.iter().zip([2.0, 3.0, -1.0]) {
            assert!((found - expected).abs() < 1.0e-12);
        }
        // A zero where the first pivot would be.
        let x = solve3(
            [[0.0, 1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 4.0]],
            [5.0, 6.0, 8.0],
        );
        assert_eq!(x, [6.0, 5.0, 2.0]);
    }

    #[test]
    fn a_shut_clutch_turns_the_wheels_with_the_engine() {
        // In gear and matched, the engine's torque speeds everything up together, as one
        // inertia: the engine's, seen through the gears, and the wheels'.
        let ratio = 3.15 * 3.7;
        let mut drive = axle(100.0 * ratio, 100.0);
        drive.engine_torque = 400.0;
        let mut seconds = 0.0;
        for _ in 0..600 {
            let solved = solve(&drive, DT);
            drive.engine_speed = solved.engine;
            drive.left.speed = solved.left;
            drive.right.speed = solved.right;
            seconds += DT;
            let slip = solved.engine - ratio * solved.left;
            assert!(slip.abs() < 2.0, "the clutch held: {slip}");
        }
        let together = 2.0 * 1.9 + 0.13 * ratio * ratio;
        let expected = 100.0 + 400.0 * ratio / together * seconds;
        assert!(
            (drive.left.speed - expected).abs() < 0.02 * expected,
            "{} against {expected}",
            drive.left.speed
        );
        assert!((drive.left.speed - drive.right.speed).abs() < 1.0e-3);
    }

    #[test]
    fn a_clutch_asked_too_much_slips_at_its_limit() {
        // The engine far faster than the wheels: the clutch carries all it can, and no more.
        let mut drive = axle(600.0, 0.0);
        drive.clutch = Coupling::new(300.0);
        let solved = solve(&drive, DT);
        assert!((solved.clutch_torque - 300.0).abs() < 1.0e-3);
        let gained = solved.left * 1.9 / DT;
        assert!((gained - 300.0 * 3.15 * 3.7 * 0.5).abs() < 0.5, "{gained}");
        let lost = (600.0 - solved.engine) * 0.13 / DT;
        assert!((lost - 300.0).abs() < 0.5, "{lost}");
    }

    #[test]
    fn neutral_and_an_open_clutch_leave_the_engine_free() {
        for (ratio, clutch) in [(0.0, 900.0), (11.0, 0.0)] {
            let mut drive = axle(300.0, 50.0);
            drive.ratio = ratio;
            drive.clutch = Coupling::new(clutch);
            drive.engine_torque = 130.0;
            let solved = solve(&drive, DT);
            assert!((solved.engine - (300.0 + 130.0 / 0.13 * DT)).abs() < 1.0e-3);
            assert!((solved.left - 50.0).abs() < 1.0e-4);
            assert_eq!(solved.clutch_torque, 0.0);
        }
    }

    #[test]
    fn a_brake_stops_a_wheel_and_holds_it() {
        let mut free = wheel(80.0);
        free.brake = 2000.0;
        let mut steps = 0;
        while free.speed > 1.0e-3 {
            let before = free.speed;
            free.speed = solve_free(&free, DT);
            // Slowing at the brake's whole torque until it stops, never past it.
            assert!(free.speed >= -1.0e-3 && free.speed < before);
            steps += 1;
            assert!(steps < 1000);
        }
        let expected = 80.0 * 1.9 / 2000.0 / DT;
        assert!(
            (steps as f32 - expected).abs() < 3.0,
            "{steps} steps, not {expected}"
        );
        // Held: the road pushing with less than the brake has doesn't turn it.
        free.force = -3000.0;
        for _ in 0..100 {
            free.speed = solve_free(&free, DT);
        }
        assert!(free.speed.abs() < 0.05, "{}", free.speed);
        // And with more, it does.
        free.force = -9000.0;
        free.speed = solve_free(&free, DT);
        assert!(free.speed > 0.1);
    }

    #[test]
    fn the_differential_holds_its_wheels_together_until_it_must_slip() {
        // One wheel on ice, pushed back by the road on the other: a locked diff shares it.
        let mut drive = axle(0.0, 50.0);
        drive.ratio = 0.0;
        drive.differential = Coupling::new(500.0);
        drive.right.force = 600.0;
        let solved = solve(&drive, DT);
        assert!((solved.left - solved.right).abs() < 0.2, "held together");
        assert!(solved.left < 50.0 && solved.right < 50.0);
        // Too much for it: it slips, at its limit.
        drive.right.force = 6000.0;
        let solved = solve(&drive, DT);
        let on_left = (50.0 - solved.left) * 1.9 / DT;
        assert!((on_left - 500.0).abs() < 0.5, "{on_left}");
        assert!(solved.left > solved.right);
    }

    #[test]
    fn braking_hard_in_gear_the_clutch_slips_the_way_the_speeds_say() {
        // In reverse, going fast, the brakes on hard: more than the brakes can hold at once,
        // and more than the clutch can. The wheels must slow, and the engine with them.
        let ratio = -2.9 * 3.7;
        let mut drive = axle(-60.0 * ratio, -60.0);
        drive.ratio = ratio;
        drive.engine_torque = -90.0;
        drive.left.brake = 2300.0;
        drive.right.brake = 2300.0;
        for (label, clutch) in [("shut", 900.0), ("weak", 40.0)] {
            let mut drive = drive;
            drive.clutch = Coupling::new(clutch);
            for _ in 0..60 {
                let solved = solve(&drive, DT);
                assert!(
                    solved.left > drive.left.speed - 1.0e-4 && solved.left <= 1.0e-3,
                    "{label}: the wheel went from {} to {}",
                    drive.left.speed,
                    solved.left
                );
                assert!(solved.engine < drive.engine_speed + 1.0e-3, "{label}");
                drive.engine_speed = solved.engine.max(0.0);
                drive.left.speed = solved.left;
                drive.right.speed = solved.right;
            }
            assert!(
                drive.left.speed > -40.0,
                "{label}: braked to {}",
                drive.left.speed
            );
        }
    }

    #[test]
    fn a_stiff_tyre_never_makes_the_wheel_overshoot() {
        // A tyre a thousand times too stiff to step the plain way settles in one step.
        let mut free = wheel(10.0);
        free.force = 500.0;
        free.stiffness = 5.0e5;
        let next = solve_free(&free, DT);
        // The road pushed it back until the force was all but gone, and no further.
        let force_after = 500.0 + 5.0e5 * (next - 10.0);
        assert!(next < 10.0 && next > 9.99);
        assert!(force_after > 0.0 && force_after < 5.0, "{force_after}");
    }

    fn line(engine_speed: f32, wheel_speed: f32) -> Driveline {
        Driveline {
            engine_speed,
            engine_inertia: 0.13,
            engine_torque: 0.0,
            ratio: 3.15 * 3.7,
            efficiency: 1.0,
            clutch: Coupling::new(900.0),
            front_share: 0.5,
            centre: Coupling::new(30_000.0),
            front: Coupling::new(0.0),
            rear: Coupling::new(0.0),
            wheels: [wheel(wheel_speed); 4],
        }
    }

    #[test]
    fn with_nothing_sent_forward_a_four_wheel_drive_is_a_rear_wheel_drive() {
        // Two-wheel drive: the same answer as the one driven axle's own solve.
        let ratio = 3.15 * 3.7;
        let mut all = line(100.0 * ratio, 100.0);
        all.front_share = 0.0;
        all.centre = Coupling::new(0.0);
        all.engine_torque = 400.0;
        all.wheels[2].force = 900.0;
        all.wheels[3].brake = 300.0;
        let mut one = axle(100.0 * ratio, 100.0);
        one.engine_torque = 400.0;
        one.left.force = 900.0;
        one.right.brake = 300.0;
        let (both, rear) = (solve_all(&all, DT), solve(&one, DT));
        assert!((both.engine - rear.engine).abs() < 1.0e-3);
        assert!((both.wheels[2] - rear.left).abs() < 1.0e-4);
        assert!((both.wheels[3] - rear.right).abs() < 1.0e-4);
        assert!((both.clutch_torque - rear.clutch_torque).abs() < 0.05);
        // The front wheels roll on, untouched.
        assert!((both.wheels[0] - 100.0).abs() < 1.0e-5);
        assert!((both.wheels[1] - 100.0).abs() < 1.0e-5);
    }

    #[test]
    fn in_four_wheel_drive_all_four_wheels_speed_up_together() {
        let ratio = 3.15 * 3.7;
        let mut all = line(100.0 * ratio, 100.0);
        all.engine_torque = 400.0;
        let mut seconds = 0.0;
        for _ in 0..600 {
            let solved = solve_all(&all, DT);
            all.engine_speed = solved.engine;
            for (wheel, speed) in all.wheels.iter_mut().zip(solved.wheels) {
                wheel.speed = speed;
            }
            seconds += DT;
        }
        let together = 4.0 * 1.9 + 0.13 * ratio * ratio;
        let expected = 100.0 + 400.0 * ratio / together * seconds;
        for wheel in &all.wheels {
            assert!(
                (wheel.speed - expected).abs() < 0.02 * expected,
                "{} against {expected}",
                wheel.speed
            );
        }
    }

    #[test]
    fn the_locked_centre_keeps_an_axle_on_ice_from_spinning_away() {
        // The rear wheels have nothing under them; the front are held by the road. With the
        // propshafts locked together, the rear cannot run away from the front.
        let ratio = 3.15 * 3.7;
        let pull = |locked: bool| {
            let mut all = line(30.0 * ratio, 30.0);
            all.engine_torque = 500.0;
            all.centre = Coupling::new(if locked { 30_000.0 } else { 0.0 });
            for _ in 0..300 {
                for index in 0..2 {
                    // A tyre gripping a road that passes at 30: it pushes back the faster the
                    // wheel turns than that.
                    all.wheels[index].stiffness = 4.0e4;
                    all.wheels[index].force = 4.0e4 * (all.wheels[index].speed - 30.0);
                }
                let solved = solve_all(&all, DT);
                all.engine_speed = solved.engine;
                for (wheel, speed) in all.wheels.iter_mut().zip(solved.wheels) {
                    wheel.speed = speed;
                }
            }
            (all.wheels[0].speed, all.wheels[2].speed)
        };
        let (front, rear) = pull(true);
        assert!((rear - front).abs() < 0.5, "locked: {front} and {rear}");
        assert!(front < 31.5, "the road held them: {front}");
        // Open, the drive all goes to the axle that takes least to turn.
        let (front, rear) = pull(false);
        assert!(rear > front + 20.0, "open: {front} and {rear}");
    }

    #[test]
    fn a_locked_differential_turns_both_its_wheels_with_one_in_the_air() {
        let ratio = 3.15 * 3.7;
        let cross = |locker: f32| {
            let mut all = line(20.0 * ratio, 20.0);
            all.engine_torque = 450.0;
            all.front = Coupling::new(locker);
            all.rear = Coupling::new(locker);
            let mut pushed = 0.0;
            for _ in 0..300 {
                // Opposite corners on the ground, the other two in the air.
                for index in [0, 3] {
                    all.wheels[index].stiffness = 4.0e4;
                    all.wheels[index].force = 4.0e4 * (all.wheels[index].speed - 20.0);
                }
                let solved = solve_all(&all, DT);
                all.engine_speed = solved.engine;
                for (wheel, speed) in all.wheels.iter_mut().zip(solved.wheels) {
                    wheel.speed = speed;
                }
                pushed = 4.0e4 * (all.wheels[0].speed + all.wheels[3].speed - 40.0);
            }
            (pushed, all.wheels[1].speed - all.wheels[0].speed)
        };
        let (locked_push, locked_apart) = cross(30_000.0);
        let (open_push, open_apart) = cross(0.0);
        assert!(locked_apart.abs() < 0.5, "{locked_apart}");
        assert!(
            open_apart > 10.0,
            "the wheel in the air spins: {open_apart}"
        );
        assert!(
            locked_push > open_push * 3.0,
            "locked pushes {locked_push}, open {open_push}"
        );
    }

    #[test]
    fn five_equations_are_solved() {
        let mut a = [[0.0; 5]; 5];
        let mut b = [0.0; 5];
        let expected = [2.0, -1.0, 0.5, 4.0, -3.0];
        for row in 0..5 {
            for column in 0..5 {
                a[row][column] = if row == column {
                    6.0
                } else {
                    ((row * 5 + column) % 7) as f64 * 0.3 - 0.8
                };
                b[row] += a[row][column] * expected[column];
            }
        }
        for (found, expected) in solve_n(a, b).iter().zip(expected) {
            assert!((found - expected).abs() < 1.0e-10);
        }
    }
}
