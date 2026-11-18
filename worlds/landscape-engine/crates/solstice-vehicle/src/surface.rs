//! What a tyre runs on.
//!
//! A circuit's surfaces are hard: asphalt, kerbs, concrete, with grass and gravel beside them.
//! Off the road the ground gives way. A tyre on clay, mud or sand sinks until the ground under
//! it carries its load, ploughs a rut as it rolls, and grips best while slipping far more than
//! it would on a road, as its tread shears the soil rather than rubber the stone. Rain changes
//! all of it: hard clay turns to grease, and mud softens. Each surface gives the numbers for
//! that ([`SurfaceGrip`]); the car does the rest (`vehicle.rs`).

/// A kind of ground, as a tyre feels it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Surface {
    /// The racing surface.
    #[default]
    Asphalt,
    /// Painted, ribbed concrete at a corner's edge.
    Kerb,
    /// Mown grass beside the track.
    Grass,
    /// A gravel trap, which is there to stop a car.
    Gravel,
    /// Bare concrete: paddock and run-off.
    Concrete,
    /// A bush track's hard-packed clay and dirt: firm in the dry, greasy in the wet.
    Dirt,
    /// Mud a wheel sinks a hand's depth into: a track's wet hollows and wheel ruts.
    Mud,
    /// A bog hole: deep, soft mud that stops a car driven by one axle.
    Bog,
    /// The floor of the bush off the track: leaf litter, bark and sticks over soft soil.
    Litter,
    /// Bare rock: slabs, ledges and steps. It grips well and gives nothing.
    Rock,
    /// A creek's bed where a track fords it: stones and gravel under moving water.
    Ford,
    /// Loose dry sand, as in a creek's bank or a washout.
    Sand,
}

/// How a surface changes what a tyre can do.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SurfaceGrip {
    /// Times the tyre's own friction on dry asphalt.
    pub grip: f32,
    /// Times that again when it is soaked: wet clay keeps half its grip, wet rock most of it.
    pub wet: f32,
    /// Rolling resistance, as a share of the load on the tyre: at a crawl, and at speed. Loose
    /// ground drags harder the faster a tyre ploughs through it; a car can crawl out of a
    /// gravel trap that stopped it from a hundred.
    pub rolling_slow: f32,
    pub rolling: f32,
    /// How rough it is: the height, in metres, of the bumps a wheel rides over.
    pub bump_height: f32,
    /// How far apart those bumps are, in metres.
    pub bump_length: f32,
    /// How many times further a tyre must slip here to grip its best than on a road. Soil is
    /// sheared, not gripped: the force builds over centimetres of slip, not millimetres.
    pub slip: f32,
    /// How much of the fall in grip past its best a tyre is spared here, from 0 to 1. On a
    /// road a spinning tyre slides on rubber and grips a quarter less. Soil is sheared by the
    /// tread, and goes on pushing back however fast the wheel turns (Janosi and Hanamoto), so
    /// a tyre spinning in mud pulls nearly as hard as one that is not: what it loses, it
    /// loses by digging itself in.
    pub hold: f32,
    /// How far a tyre carrying its share of a standing car sinks in, in metres.
    pub sink: f32,
    /// How much further a wheel spinning at twice the speed it rolls at digs itself in.
    pub dig: f32,
}

impl Surface {
    pub const ALL: [Self; 12] = [
        Self::Asphalt,
        Self::Kerb,
        Self::Grass,
        Self::Gravel,
        Self::Concrete,
        Self::Dirt,
        Self::Mud,
        Self::Bog,
        Self::Litter,
        Self::Rock,
        Self::Ford,
        Self::Sand,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Asphalt => "asphalt",
            Self::Kerb => "kerb",
            Self::Grass => "grass",
            Self::Gravel => "gravel",
            Self::Concrete => "concrete",
            Self::Dirt => "dirt",
            Self::Mud => "mud",
            Self::Bog => "bog",
            Self::Litter => "litter",
            Self::Rock => "rock",
            Self::Ford => "ford",
            Self::Sand => "sand",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|s| s.name() == name)
    }

    /// A number for this surface that fits a byte, for maps of the ground: its place in
    /// [`Surface::ALL`].
    pub fn index(self) -> u8 {
        Self::ALL
            .iter()
            .position(|&surface| surface == self)
            .unwrap_or(0) as u8
    }

    pub fn from_index(index: u8) -> Option<Self> {
        Self::ALL.get(index as usize).copied()
    }

    /// Whether a tyre sinks into it: soft ground, which sprays and holds a car back.
    pub fn is_soft(self) -> bool {
        self.grip().sink > 0.02
    }

    /// The road surfaces are figures for a slick tyre in the dry. Grass and gravel are from
    /// the usual ranges quoted for race tyres off the track: a third to a half of the grip, and
    /// a gravel trap, which a car sinks into, slowing it at about half a g by drag alone.
    ///
    /// The off-road ones follow the terramechanics of Bekker and Wong (*Theory of Ground
    /// Vehicles*): friction of a tyre on firm soil of 0.6 to 0.8, half that on wet clay and
    /// less in mud; motion resistance of a few hundredths of the load on a firm track, a tenth
    /// to a quarter in soft mud; and the slip for the best pull several times a road's.
    pub fn grip(self) -> SurfaceGrip {
        let hard = |grip: f32, wet: f32, rolling: f32| SurfaceGrip {
            grip,
            wet,
            rolling_slow: rolling,
            rolling,
            bump_height: 0.0,
            bump_length: 1.0,
            slip: 1.0,
            hold: 0.0,
            sink: 0.0,
            dig: 0.0,
        };
        match self {
            Self::Asphalt => hard(1.0, 0.78, 0.012),
            Self::Kerb => hard(0.9, 0.6, 0.015),
            Self::Concrete => hard(0.93, 0.8, 0.013),
            Self::Grass => SurfaceGrip {
                grip: 0.45,
                wet: 0.7,
                rolling_slow: 0.05,
                rolling: 0.07,
                bump_height: 0.018,
                bump_length: 1.3,
                ..hard(0.0, 0.0, 0.0)
            },
            Self::Gravel => SurfaceGrip {
                grip: 0.42,
                wet: 0.92,
                rolling_slow: 0.14,
                rolling: 0.55,
                bump_height: 0.025,
                bump_length: 0.9,
                ..hard(0.0, 0.0, 0.0)
            },
            Self::Dirt => SurfaceGrip {
                grip: 0.74,
                wet: 0.6,
                rolling_slow: 0.028,
                rolling: 0.05,
                bump_height: 0.022,
                bump_length: 1.1,
                slip: 1.7,
                hold: 0.7,
                sink: 0.004,
                dig: 0.012,
            },
            Self::Mud => SurfaceGrip {
                grip: 0.46,
                wet: 0.85,
                rolling_slow: 0.06,
                rolling: 0.2,
                bump_height: 0.035,
                bump_length: 0.8,
                slip: 2.6,
                hold: 0.9,
                sink: 0.07,
                dig: 0.05,
            },
            Self::Bog => SurfaceGrip {
                grip: 0.38,
                wet: 0.88,
                rolling_slow: 0.09,
                rolling: 0.32,
                bump_height: 0.03,
                bump_length: 0.9,
                slip: 3.0,
                hold: 0.9,
                sink: 0.14,
                dig: 0.05,
            },
            Self::Litter => SurfaceGrip {
                grip: 0.58,
                wet: 0.75,
                rolling_slow: 0.055,
                rolling: 0.1,
                bump_height: 0.04,
                bump_length: 0.75,
                slip: 1.9,
                hold: 0.75,
                sink: 0.015,
                dig: 0.03,
            },
            Self::Rock => SurfaceGrip {
                grip: 0.86,
                wet: 0.78,
                rolling_slow: 0.02,
                rolling: 0.025,
                bump_height: 0.05,
                bump_length: 0.55,
                slip: 1.15,
                hold: 0.25,
                sink: 0.0,
                dig: 0.0,
            },
            Self::Ford => SurfaceGrip {
                grip: 0.52,
                wet: 1.0,
                rolling_slow: 0.1,
                rolling: 0.6,
                bump_height: 0.045,
                bump_length: 0.5,
                slip: 1.6,
                hold: 0.7,
                sink: 0.03,
                dig: 0.03,
            },
            Self::Sand => SurfaceGrip {
                grip: 0.5,
                wet: 1.15,
                rolling_slow: 0.11,
                rolling: 0.3,
                bump_height: 0.02,
                bump_length: 1.4,
                slip: 2.4,
                hold: 0.9,
                sink: 0.055,
                dig: 0.09,
            },
        }
    }
}

/// How a surface takes a tyre's mark: what is left in the ground where one has been
/// ([`crate::rut`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Soil {
    /// How wet what a tyre churns up is, from 0 to 1, and how wet it stays once the churned
    /// ground has dried: a bog stays sodden, a clay track dries pale.
    pub wet: f32,
    pub stays: f32,
    /// How much bare soil a tyre leaves showing where it broke the ground: all of it in
    /// mud, most of it through leaf litter, none of it in sand, which is sand all the way
    /// down.
    pub churn: f32,
    /// The share of what a tyre presses out of its rut that is heaped up beside it. Mud
    /// flows aside; litter and dry soil pack down.
    pub berm: f32,
    /// How well it holds a tread's print: damp clay holds every lug, slop closes over it.
    pub print: f32,
}

impl Surface {
    /// How this ground takes a tyre's mark when it is this wet: 0 dry, 1 soaked.
    pub fn soil(self, wetness: f32) -> Soil {
        let w = wetness.clamp(0.0, 1.0);
        let soil = |wet: f32, stays: f32, churn: f32, berm: f32, print: f32| Soil {
            wet,
            stays,
            churn,
            berm,
            print,
        };
        match self {
            Self::Asphalt | Self::Kerb | Self::Concrete | Self::Rock => {
                soil(0.0, 0.0, 0.0, 0.0, 0.0)
            }
            Self::Grass => soil(0.2 + 0.5 * w, 0.1 + 0.3 * w, 0.5, 0.2, 0.5),
            Self::Gravel => soil(0.3 * w, 0.0, 0.0, 0.45, 0.2),
            Self::Dirt => soil(0.15 + 0.75 * w, 0.05 + 0.45 * w, 0.45 + 0.3 * w, 0.25, 0.85),
            Self::Mud => soil(0.9, 0.55 + 0.25 * w, 1.0, 0.5, 0.85),
            Self::Bog => soil(1.0, 0.8 + 0.2 * w, 1.0, 0.55, 0.45),
            Self::Litter => soil(0.3 + 0.5 * w, 0.12 + 0.4 * w, 0.8, 0.2, 0.6),
            Self::Ford => soil(1.0, 1.0, 0.25, 0.2, 0.2),
            Self::Sand => soil(0.05 + 0.6 * w, 0.3 * w, 0.15, 0.45, 0.7),
        }
    }
}

impl SurfaceGrip {
    /// Rolling resistance at a speed, in metres a second, as a share of the load.
    pub fn rolling_at(&self, speed: f32) -> f32 {
        /// The speed by which loose ground drags its hardest.
        const FULL: f32 = 14.0;
        self.rolling_slow + (self.rolling - self.rolling_slow) * (speed.abs() / FULL).min(1.0)
    }

    /// Its grip when it is this wet: 0 dry, 1 soaked.
    pub fn grip_at(&self, wetness: f32) -> f32 {
        self.grip * (1.0 + (self.wet - 1.0) * wetness.clamp(0.0, 1.0))
    }

    /// How far a tyre sinks in when the ground is this wet: rain softens what was already
    /// soft, by a third as much again, and firm ground a little.
    pub fn sink_at(&self, wetness: f32) -> f32 {
        let wetness = wetness.clamp(0.0, 1.0);
        self.sink * (1.0 + 0.3 * wetness)
            + if self.slip > 1.5 {
                0.012 * wetness
            } else {
                0.0
            }
    }
}

/// The height of a rough surface's bumps at a spot: smooth, repeatable noise between -1 and 1,
/// from where the wheel is on the ground and how far apart the bumps are.
pub fn bump(x: f32, z: f32, length: f32) -> f32 {
    let (u, v) = (x / length, z / length);
    let (iu, iv) = (u.floor(), v.floor());
    let (fu, fv) = (u - iu, v - iv);
    // Smoothstep, so that a wheel rolling over a cell's edge feels no kink.
    let (su, sv) = (fu * fu * (3.0 - 2.0 * fu), fv * fv * (3.0 - 2.0 * fv));
    let corner = |du: f32, dv: f32| lattice((iu + du) as i32, (iv + dv) as i32);
    let low = corner(0.0, 0.0) + (corner(1.0, 0.0) - corner(0.0, 0.0)) * su;
    let high = corner(0.0, 1.0) + (corner(1.0, 1.0) - corner(0.0, 1.0)) * su;
    low + (high - low) * sv
}

/// A value between -1 and 1 for a point of the lattice, the same every run and on every
/// platform: integer arithmetic only.
fn lattice(x: i32, z: i32) -> f32 {
    let mut h = (x as u32).wrapping_mul(0x9E37_79B1) ^ (z as u32).wrapping_mul(0x85EB_CA77);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2C1B_3C6D);
    h ^= h >> 12;
    (h & 0xFFFF) as f32 / 32_767.5 - 1.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_read_back() {
        for surface in Surface::ALL {
            assert_eq!(Surface::from_name(surface.name()), Some(surface));
            assert_eq!(Surface::from_index(surface.index()), Some(surface));
        }
        assert_eq!(Surface::from_name("ice"), None);
        assert_eq!(Surface::from_index(200), None);
    }

    #[test]
    fn loose_ground_drags_harder_at_speed() {
        let gravel = Surface::Gravel.grip();
        assert_eq!(gravel.rolling_at(0.0), gravel.rolling_slow);
        assert_eq!(gravel.rolling_at(30.0), gravel.rolling);
        assert_eq!(gravel.rolling_at(-30.0), gravel.rolling);
        assert!(
            gravel.rolling_at(7.0) > gravel.rolling_slow && gravel.rolling_at(7.0) < gravel.rolling
        );
        let road = Surface::Asphalt.grip();
        assert_eq!(road.rolling_at(0.0), road.rolling_at(80.0));
    }

    #[test]
    fn every_surface_is_one_a_tyre_can_be_on() {
        for surface in Surface::ALL {
            let grip = surface.grip();
            assert!((0.2..=1.0).contains(&grip.grip), "{surface:?}");
            assert!((0.4..=1.2).contains(&grip.wet), "{surface:?}");
            assert!(grip.rolling >= grip.rolling_slow && grip.rolling_slow > 0.0);
            assert!(grip.slip >= 1.0 && grip.sink >= 0.0 && grip.dig >= 0.0);
            assert!((0.0..=1.0).contains(&grip.hold));
            assert!(grip.bump_length > 0.0);
        }
    }

    #[test]
    fn rain_makes_clay_greasy_and_mud_deeper() {
        let dirt = Surface::Dirt.grip();
        assert_eq!(dirt.grip_at(0.0), dirt.grip);
        assert!(dirt.grip_at(1.0) < dirt.grip * 0.65);
        assert!(dirt.grip_at(0.5) < dirt.grip && dirt.grip_at(0.5) > dirt.grip_at(1.0));
        let bog = Surface::Bog.grip();
        assert!(bog.sink_at(1.0) > bog.sink * 1.3);
        // A road is as hard wet as dry.
        assert_eq!(Surface::Asphalt.grip().sink_at(1.0), 0.0);
        // The soft ones are the ones that hold a car back.
        assert!(Surface::Mud.is_soft() && Surface::Bog.is_soft() && Surface::Sand.is_soft());
        assert!(!Surface::Dirt.is_soft() && !Surface::Rock.is_soft());
    }

    #[test]
    fn soft_ground_keeps_a_mark_and_hard_ground_none() {
        for surface in Surface::ALL {
            for wetness in [0.0, 0.5, 1.0] {
                let soil = surface.soil(wetness);
                for share in [soil.wet, soil.stays, soil.churn, soil.berm, soil.print] {
                    assert!((0.0..=1.0).contains(&share), "{surface:?}");
                }
                assert!(
                    soil.stays <= soil.wet,
                    "{surface:?} dries wetter than it was"
                );
            }
        }
        assert_eq!(Surface::Rock.soil(1.0).churn, 0.0);
        assert!(Surface::Bog.soil(0.0).stays > Surface::Mud.soil(0.0).stays);
        // Rain makes a clay track's ruts wet; a bog's are wet whatever the weather.
        assert!(Surface::Dirt.soil(1.0).wet > Surface::Dirt.soil(0.0).wet + 0.5);
        assert_eq!(Surface::Bog.soil(0.0).wet, 1.0);
    }

    #[test]
    fn bumps_are_smooth_bounded_and_repeat() {
        let mut last = bump(0.0, 0.3, 1.3);
        for i in 1..2000 {
            let x = i as f32 * 0.01;
            let here = bump(x, 0.3, 1.3);
            assert!((-1.0..=1.0).contains(&here));
            assert!(
                (here - last).abs() < 0.05,
                "a kink at {x}: {last} to {here}"
            );
            last = here;
        }
        assert_eq!(bump(12.5, -7.25, 0.9), bump(12.5, -7.25, 0.9));
        // Not flat.
        let spread = (0..200)
            .map(|i| bump(i as f32 * 0.37, 1.0, 1.0).abs())
            .fold(0.0, f32::max);
        assert!(spread > 0.5);
    }
}
