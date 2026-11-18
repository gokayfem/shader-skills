//! The tyre: what force it makes for how much it slips.
//!
//! A tyre grips by slipping. Rolling faster or slower than the road passes under it (the slip
//! ratio) pushes the car along or holds it back; pointing a little away from where it is going
//! (the slip angle) pushes it sideways. Each force rises steeply with slip, peaks, and then falls
//! away as the tyre slides. That shape is Pacejka's Magic Formula (Pacejka, *Tyre and Vehicle
//! Dynamics*), used here in the form where the curve is fixed by where its peak is and how high.
//!
//! * **One budget for both.** Braking and cornering share the tyre's grip. The two slips are
//!   scaled by where each one peaks and combined into one, which sets how much force there is;
//!   the force then points against the slip. This is the similarity method (Pacejka, ch. 4;
//!   Milliken, *Race Car Vehicle Dynamics*, ch. 2), and it gives the friction ellipse.
//! * **Load sensitivity.** A tyre pressed twice as hard grips less than twice as well. That is
//!   why weight transfer costs grip, and why a car's balance follows its roll stiffness.

/// The Magic Formula scaled to peak at 1 where the slip is 1:
/// `sin(C atan(Bx - E (Bx - atan(Bx))))`.
///
/// `C` sets how far the force falls once the tyre slides: far out, it tends to `sin(C π/2)`.
/// `E` bends the curve around its peak: lower is sharper.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Curve {
    b: f32,
    c: f32,
    e: f32,
}

impl Curve {
    /// `c` must be above 1 and below 2, and `e` below 1, for the curve to have its peak.
    pub fn new(c: f32, e: f32) -> Self {
        assert!(
            c > 1.0 && c < 2.0 && e < 1.0,
            "no peak with C {c} and E {e}"
        );
        // The peak is where the sine's angle is a right angle: `u - E (u - atan u) = tan(π/2C)`
        // for `u = B`, as the slip there is 1. The left side grows with `u`, so halve the range.
        let target = f64::from(std::f32::consts::FRAC_PI_2 / c).tan();
        let e64 = f64::from(e);
        let (mut low, mut high) = (0.0_f64, 1.0e4_f64);
        for _ in 0..80 {
            let middle = 0.5 * (low + high);
            if middle - e64 * (middle - middle.atan()) < target {
                low = middle;
            } else {
                high = middle;
            }
        }
        Self {
            b: (0.5 * (low + high)) as f32,
            c,
            e,
        }
    }

    /// The force at this slip, as a share of the peak. Odd: a negative slip gives the negative.
    pub fn at(&self, slip: f32) -> f32 {
        let bx = self.b * slip;
        (self.c * (bx - self.e * (bx - bx.atan())).atan()).sin()
    }

    /// How steeply the force rises from nothing: per unit of slip, as a share of the peak.
    pub fn stiffness(&self) -> f32 {
        self.b * self.c
    }
}

/// One tyre's character.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tyre {
    /// Peak friction along the tyre, at the nominal load, on dry asphalt.
    pub grip_long: f32,
    /// Peak friction across it.
    pub grip_lat: f32,
    /// The slip ratio where the force along it peaks.
    pub peak_ratio: f32,
    /// The slip angle, in radians, where the force across it peaks.
    pub peak_angle: f32,
    /// The curves' shapes: `C` and `E` along, then across.
    pub shape_long: (f32, f32),
    pub shape_lat: (f32, f32),
    /// The load, in newtons, the grip figures are given at.
    pub nominal_load: f32,
    /// The share of its friction the tyre loses for each nominal load added.
    pub load_sensitivity: f32,
    /// How far the tyre rolls, in metres, before its side force catches up with its slip angle.
    pub relaxation: f32,
    /// What its tread makes of the ground: times its grip on firm ground (a road, rock, hard
    /// clay), and on soft ground that a tyre sinks into (mud, sand, a bog). A road tyre is
    /// more than 1 and less; a mud tyre, less than 1 and more.
    pub tread: (f32, f32),
}

impl Tyre {
    /// A racing slick of the size a GT car wears, warm, in the dry.
    ///
    /// * Peak friction of about 1.7 at the working load, lost at about 14% for each doubling.
    /// * The side force peaks near 7.5 degrees and falls away gently beyond it.
    /// * The force along peaks near 9% slip, and a locked or spinning wheel keeps about 60%.
    pub fn slick() -> Self {
        Self {
            grip_long: 1.72,
            grip_lat: 1.68,
            peak_ratio: 0.09,
            peak_angle: 7.5_f32.to_radians(),
            shape_long: (1.6, 0.4),
            shape_lat: (1.36, 0.2),
            nominal_load: 4000.0,
            load_sensitivity: 0.14,
            relaxation: 0.28,
            tread: (1.0, 1.0),
        }
    }

    /// What its tread makes of ground a tyre sinks this far into, in metres: times its grip.
    pub fn tread_on(&self, sink: f32) -> f32 {
        let soft = ((sink - 0.008) / 0.04).clamp(0.0, 1.0);
        self.tread.0 + (self.tread.1 - self.tread.0) * soft
    }

    /// The friction a tyre has under this load, as a multiple of the figure at its nominal one.
    pub fn load_factor(&self, load: f32) -> f32 {
        (1.0 - self.load_sensitivity * (load / self.nominal_load - 1.0)).clamp(0.55, 1.2)
    }

    /// The curves, worked out once.
    pub fn curves(&self) -> TyreCurves {
        TyreCurves {
            long: Curve::new(self.shape_long.0, self.shape_long.1),
            lat: Curve::new(self.shape_lat.0, self.shape_lat.1),
        }
    }

    /// The force the ground puts on the tyre, in newtons: along the way it points, and across.
    ///
    /// * `load`: how hard it is pressed down, in newtons.
    /// * `ratio`: the slip ratio, above zero when the tyre turns faster than it rolls.
    /// * `slide`: the tangent of the slip angle, above zero when the tyre is sliding to its
    ///   left over the ground, so that the force is to its right.
    /// * `grip`: the surface's, 1 on dry asphalt.
    pub fn force(
        &self,
        curves: &TyreCurves,
        load: f32,
        ratio: f32,
        slide: f32,
        grip: f32,
    ) -> (f32, f32) {
        if load <= 0.0 {
            return (0.0, 0.0);
        }
        let along = ratio / self.peak_ratio;
        let across = slide / self.peak_angle.tan();
        let slip = along.hypot(across);
        if slip < 1.0e-6 {
            return (0.0, 0.0);
        }
        let limit = load * self.load_factor(load) * grip;
        (
            limit * self.grip_long * curves.long.at(slip) * along / slip,
            limit * self.grip_lat * curves.lat.at(slip) * across / slip,
        )
    }
}

/// A tyre's two curves.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TyreCurves {
    pub long: Curve,
    pub lat: Curve,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_curve_peaks_at_one_where_the_slip_is_one() {
        for (c, e) in [(1.36, 0.2), (1.6, 0.4), (1.3, -1.0), (1.9, 0.9)] {
            let curve = Curve::new(c, e);
            assert!((curve.at(1.0) - 1.0).abs() < 1.0e-5, "C {c} E {e}");
            let mut slip = 0.0;
            while slip < 12.0 {
                assert!(curve.at(slip) <= 1.0 + 1.0e-6);
                slip += 0.01;
            }
            assert!(curve.at(0.5) < 1.0 && curve.at(2.0) < 1.0);
            assert!((curve.at(-0.7) + curve.at(0.7)).abs() < 1.0e-6, "odd");
        }
    }

    #[test]
    fn a_sliding_tyre_keeps_the_share_its_shape_says() {
        let curve = Curve::new(1.6, 0.4);
        let far = (1.6_f32 * std::f32::consts::FRAC_PI_2).sin();
        assert!((curve.at(1.0e4) - far).abs() < 0.01);
        // And it has fallen a good way there by the time the wheel is locked.
        let tyre = Tyre::slick();
        let locked = curve.at(1.0 / tyre.peak_ratio);
        assert!((0.55..0.75).contains(&locked), "{locked}");
    }

    #[test]
    fn the_slick_is_as_stiff_as_a_slick() {
        // A racing slick's cornering stiffness is about 0.6 to 0.9 of its load per degree.
        let tyre = Tyre::slick();
        let curves = tyre.curves();
        let one_degree = 1.0_f32.to_radians().tan();
        let (_, side) = tyre.force(&curves, 4000.0, 0.0, one_degree, 1.0);
        let per_degree = side / 4000.0;
        assert!((0.6..0.9).contains(&per_degree), "{per_degree}");
    }

    #[test]
    fn the_force_peaks_at_the_peak_slips() {
        let tyre = Tyre::slick();
        let curves = tyre.curves();
        let (along, across) = tyre.force(&curves, 4000.0, tyre.peak_ratio, 0.0, 1.0);
        assert!((along - 4000.0 * tyre.grip_long).abs() < 1.0 && across == 0.0);
        let (along, across) = tyre.force(&curves, 4000.0, 0.0, tyre.peak_angle.tan(), 1.0);
        assert!((across - 4000.0 * tyre.grip_lat).abs() < 1.0 && along == 0.0);
        // Against the slip, whichever way.
        let (along, across) = tyre.force(&curves, 4000.0, -0.05, -0.05, 1.0);
        assert!(along < 0.0 && across < 0.0);
    }

    #[test]
    fn braking_and_cornering_share_one_budget() {
        let tyre = Tyre::slick();
        let curves = tyre.curves();
        let slide = tyre.peak_angle.tan();
        let (_, alone) = tyre.force(&curves, 4000.0, 0.0, slide, 1.0);
        let (along, with) = tyre.force(&curves, 4000.0, -tyre.peak_ratio, slide, 1.0);
        assert!(with < alone * 0.8, "braking takes from cornering");
        // Never more than the peak in all.
        let total = (along / tyre.grip_long).hypot(with / tyre.grip_lat);
        assert!(total <= 4000.0 * 1.0001, "{total}");
    }

    #[test]
    fn a_heavier_load_grips_more_but_less_well() {
        let tyre = Tyre::slick();
        let curves = tyre.curves();
        let slide = tyre.peak_angle.tan();
        let (_, light) = tyre.force(&curves, 3000.0, 0.0, slide, 1.0);
        let (_, heavy) = tyre.force(&curves, 6000.0, 0.0, slide, 1.0);
        assert!(heavy > light, "more load, more force");
        assert!(heavy / 6000.0 < light / 3000.0, "but less for each newton");
        // Two tyres sharing a load evenly grip better than the same load moved onto one.
        let (_, even) = tyre.force(&curves, 4500.0, 0.0, slide, 1.0);
        assert!(2.0 * even > light + heavy);
        assert_eq!(tyre.force(&curves, 0.0, 0.1, 0.1, 1.0), (0.0, 0.0));
        assert_eq!(tyre.force(&curves, -5.0, 0.1, 0.1, 1.0), (0.0, 0.0));
    }

    #[test]
    fn grass_has_less_grip() {
        let tyre = Tyre::slick();
        let curves = tyre.curves();
        let (dry, _) = tyre.force(&curves, 4000.0, 0.09, 0.0, 1.0);
        let (grass, _) = tyre.force(&curves, 4000.0, 0.09, 0.0, 0.45);
        assert!((grass / dry - 0.45).abs() < 1.0e-4);
    }
}
