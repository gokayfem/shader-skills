//! Vehicle dynamics for gameplay: a car, as its tyres, springs, engine and the air move it.
//!
//! * [`spec`]: a car's numbers, and the `.vehicle` text file that holds them.
//! * [`tyre`]: the force a tyre makes for how much it slips.
//! * [`drivetrain`]: engine, clutch, gearbox, differential, brakes and wheels, solved together.
//! * [`vehicle`]: the car itself, stepped: suspension, tyres, drivetrain, aerodynamics, and the
//!   rigid body they move.
//! * [`driver`]: what a key or a stick is turned into at the wheel and pedals.
//! * [`surface`]: asphalt, kerb, grass and gravel, as a tyre feels them.
//! * [`rut`]: the ruts tyres press into soft ground, which stay.
//!
//! Nothing here touches the GPU, the physics engine or the world: a car is stepped against a
//! [`Ground`] it casts rays at, so it runs headless, and a run repeats exactly. `solstice-physics`
//! gives it the scene to drive on and deals with what its body hits.
//!
//! The models are the standard ones of vehicle dynamics: Pacejka's Magic Formula for the tyres,
//! with combined slip by the similarity method and a relaxation length; a spring, a damper, an
//! anti-roll bar and a bump stop at each wheel; a torque curve, a clutch, a sequential gearbox
//! and a clutch-pack differential; drag and downforce growing with the square of speed. See
//! Pacejka, *Tyre and Vehicle Dynamics*, and Milliken and Milliken, *Race Car Vehicle Dynamics*.

pub mod driver;
pub mod drivetrain;
pub mod rut;
pub mod spec;
pub mod surface;
pub mod tyre;
pub mod vehicle;

pub use driver::{Asked, Driver};
pub use rut::{Press, Ruts};
pub use spec::Transfer;
pub use spec::{SpecError, VehicleSpec};
pub use surface::Surface;
pub use tyre::Tyre;
pub use vehicle::{
    Body, Contact, Controls, DriveMode, Flat, Ground, Telemetry, Vehicle, Wheel, SUBSTEPS,
};
