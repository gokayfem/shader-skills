//! The fixed clock: turns frames of any length into whole steps of the same length.
//!
//! Gameplay and physics advance only in steps of [`STEP_SECONDS`], so a run gives the same
//! result at any frame rate. Each frame adds its time to an accumulator and takes out as many
//! whole steps as it holds. What is left over is how far the frame is between the last step and
//! the next, for drawing things in between.
//!
//! Time is counted in ticks of a sixty-millionth of a second, so that a step is exactly a
//! million ticks. A frame's length is rounded to the nearest tick, under 10 ns, so the steps
//! keep pace with real time to well under a part per million.

/// Steps a second.
pub const STEPS_PER_SECOND: u32 = 60;
/// The length of one step.
pub const STEP_SECONDS: f64 = 1.0 / STEPS_PER_SECOND as f64;
/// The most steps one frame runs. A frame that would need more is cut short, and the game runs
/// slower for it, rather than falling further behind with every frame.
pub const MAX_STEPS_PER_FRAME: u32 = 3;

/// Ticks in a step.
pub const STEP_TICKS: u64 = 1_000_000;
/// Ticks in a second.
pub const TICKS_PER_SECOND: u64 = STEP_TICKS * STEPS_PER_SECOND as u64;

/// Turns frame times into steps.
#[derive(Clone, Debug, Default)]
pub struct FixedClock {
    /// Time received but not yet stepped through, in ticks.
    pending: u64,
}

/// What one frame of a [`FixedClock`] holds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FrameSteps {
    /// Whole steps to run.
    pub steps: u32,
    /// How far the frame is from the last step towards the next, from 0 to just under 1.
    pub alpha: f32,
    /// Whether time was dropped because the frame needed more than [`MAX_STEPS_PER_FRAME`].
    pub slowed: bool,
}

impl FixedClock {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a frame of `seconds` and says how many steps it holds. Negative, infinite or
    /// not-a-number frame times count as nothing.
    pub fn advance(&mut self, seconds: f64) -> FrameSteps {
        let ticks = if seconds.is_finite() && seconds > 0.0 {
            // A frame of over an hour is a paused tab coming back, and is cut short anyway.
            (seconds.min(3600.0) * TICKS_PER_SECOND as f64).round() as u64
        } else {
            0
        };
        self.pending += ticks;
        let wanted = self.pending / STEP_TICKS;
        let steps = wanted.min(u64::from(MAX_STEPS_PER_FRAME));
        let slowed = wanted > steps;
        if slowed {
            // Keep only what is left over from the last step that runs: the rest is dropped.
            self.pending %= STEP_TICKS;
        } else {
            self.pending -= steps * STEP_TICKS;
        }
        FrameSteps {
            steps: steps as u32,
            alpha: self.pending as f32 / STEP_TICKS as f32,
            slowed,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_second_of_frames_is_sixty_steps_whatever_their_length() {
        for rate in [30, 60, 75, 90, 120, 144, 165, 240] {
            let mut clock = FixedClock::new();
            let frame = 1.0 / f64::from(rate);
            let steps: u32 = (0..rate).map(|_| clock.advance(frame).steps).sum();
            // The sixtieth falls right at the end of the second, so a frame rounded down by a
            // fraction of a tick leaves it for the next frame.
            assert!(
                (59..=60).contains(&steps),
                "{steps} at {rate} frames a second"
            );
            // A microsecond more completes it.
            assert_eq!(
                steps + clock.advance(1.0e-6).steps,
                60,
                "at {rate} frames a second"
            );
        }
    }

    #[test]
    fn an_hour_of_frames_keeps_pace_with_real_time() {
        // Steady rates whose frames are no whole number of ticks, and uneven frames.
        for rate in [144.0, 165.0, 59.94] {
            let mut clock = FixedClock::new();
            let frames = (3600.0 * rate) as u64;
            let steps: u64 = (0..frames)
                .map(|_| u64::from(clock.advance(1.0 / rate).steps))
                .sum();
            let real = frames as f64 / rate * f64::from(STEPS_PER_SECOND);
            assert!(
                (steps as f64 - real).abs() <= 1.0,
                "{steps} against {real} at {rate}"
            );
        }
        let mut clock = FixedClock::new();
        let mut steps = 0_u64;
        for i in 0..200_000 {
            let frame = if i % 2 == 0 { 0.0091 } else { 0.0243 };
            steps += u64::from(clock.advance(frame).steps);
        }
        let real = 100_000.0 * (0.0091 + 0.0243) * f64::from(STEPS_PER_SECOND);
        assert!((steps as f64 - real).abs() <= 1.0, "{steps} against {real}");
    }

    #[test]
    fn a_slow_frame_runs_at_most_the_limit_and_drops_the_rest() {
        let mut clock = FixedClock::new();
        let frame = clock.advance(0.5);
        assert_eq!(frame.steps, MAX_STEPS_PER_FRAME);
        assert!(frame.slowed);
        assert!(frame.alpha < 1.0);
        // The dropped time doesn't come back as a burst of steps afterwards.
        let next = clock.advance(1.0 / 60.0);
        assert!(next.steps <= 1 && !next.slowed);
    }

    #[test]
    fn alpha_is_how_far_into_the_next_step_the_frame_is() {
        let mut clock = FixedClock::new();
        let frame = clock.advance(STEP_SECONDS * 1.5);
        assert_eq!(frame.steps, 1);
        assert!((frame.alpha - 0.5).abs() < 1.0e-6, "{}", frame.alpha);
        let frame = clock.advance(STEP_SECONDS * 0.25);
        assert_eq!(frame.steps, 0);
        assert!((frame.alpha - 0.75).abs() < 1.0e-6, "{}", frame.alpha);
        let frame = clock.advance(STEP_SECONDS * 0.25);
        assert_eq!(frame.steps, 1);
        assert!(frame.alpha < 1.0e-6);
    }

    #[test]
    fn nonsense_frame_times_count_as_nothing() {
        let mut clock = FixedClock::new();
        for seconds in [-1.0, f64::NAN, f64::NEG_INFINITY, f64::INFINITY, 0.0] {
            assert_eq!(clock.advance(seconds).steps, 0);
        }
        // A huge but real one is cut short.
        assert_eq!(clock.advance(1.0e9).steps, MAX_STEPS_PER_FRAME);
    }
}
