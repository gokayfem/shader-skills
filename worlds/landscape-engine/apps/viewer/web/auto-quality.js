// The page's automatic quality: it starts high and steps down while the frame rate stays below
// target. It never steps back up: oscillating between presets is worse than either one.
//
// Its last step, on a high-density display, is fewer pixels: Low drawn at one canvas pixel per
// CSS pixel. A Retina screen at a device pixel ratio of 2 has four times the pixels, and Low
// there still costs more than High at a ratio of 1 (an integrated GPU: 175 ms against 111).

/// The frame rate below which the page steps down.
export const TARGET_FPS = 28;
/// Below this, a frame rate is far too slow to need three reports in a row to be sure of it.
const VERY_SLOW_FPS = TARGET_FPS / 2;

/// Each step: a quality preset, and the most canvas pixels per CSS pixel it draws.
export function autoSteps(devicePixelRatio) {
  const steps = [
    { quality: "high", pixelRatio: 2 },
    { quality: "medium", pixelRatio: 2 },
    { quality: "low", pixelRatio: 2 },
  ];
  if (devicePixelRatio > 1) steps.push({ quality: "low", pixelRatio: 1 });
  return steps;
}

export class AutoQuality {
  constructor(devicePixelRatio) {
    this.steps = autoSteps(devicePixelRatio);
    this.level = 0;
    this.slowReports = 0;
    // The first reports, taken while shaders compile, say nothing about the scene.
    this.ignoreReports = 4;
  }

  get step() {
    return this.steps[this.level];
  }

  /// Takes the frame rate over the last report, and whether the scene was loading through it.
  /// Returns the step to take now, or null to stay.
  report(fps, loading) {
    if (this.level === this.steps.length - 1) return null;
    // While geometry pours in, the frame rate says how fast the scene loads, not how fast it
    // draws: a lower quality would outlast the loading it was chosen for.
    if (loading) {
      this.slowReports = 0;
      return null;
    }
    if (this.ignoreReports > 0) {
      this.ignoreReports -= 1;
      return null;
    }
    this.slowReports = fps < TARGET_FPS ? this.slowReports + 1 : 0;
    // Half the target or less is no passing hitch: two reports in a row settle it.
    const needed = fps < VERY_SLOW_FPS ? 2 : 3;
    if (this.slowReports < needed) return null;
    this.level += 1;
    this.slowReports = 0;
    // A report for the frames that made the new targets.
    this.ignoreReports = 1;
    return this.step;
  }

  /// A name for the step, beside "Automatic" in the quality list.
  get label() {
    const { quality, pixelRatio } = this.step;
    // Only a high-density display has a step at a ratio of 1.
    return pixelRatio === 1 ? `${quality}, fewer pixels` : quality;
  }
}
