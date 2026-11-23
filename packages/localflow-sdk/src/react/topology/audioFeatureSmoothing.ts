/**
 * Frame-rate-independent smoothing and bounded history for overlay audio features.
 * Pure logic only — no canvas, no DOM, no timers — so it is deterministic under test.
 */

/** Longest frame gap we integrate over; larger gaps (missed frames, tab resume) are
 * treated as one long-but-bounded step so the terrain never snaps discontinuously. */
const MAX_STEP_MS = 100;

/** Clamp a normalized feature to [0, 1], substituting `fallback` for NaN/±Infinity/missing. */
export function clampUnit(value: number | null | undefined, fallback = 0): number {
  if (value == null || !Number.isFinite(value)) {
    return fallback;
  }
  return Math.max(0, Math.min(1, value));
}

/** Exponential smoothing coefficient for an elapsed `dtMs` and time constant `tauMs`.
 * Composable across frames: two consecutive steps equal one combined step exactly. */
export function smoothingCoefficient(dtMs: number, tauMs: number): number {
  if (!(dtMs > 0) || !(tauMs > 0)) {
    return dtMs > 0 ? 1 : 0;
  }
  return 1 - Math.exp(-dtMs / tauMs);
}

/**
 * One smoothed feature with separate attack (rising) and release (falling) time
 * constants, so speech feels immediate while peaks settle naturally.
 */
export class FeatureSmoother {
  value: number;

  constructor(
    private readonly attackMs: number,
    private readonly releaseMs: number,
    initial = 0,
  ) {
    this.value = clampUnit(initial);
  }

  update(target: number | null | undefined, dtMs: number): number {
    const goal = clampUnit(target, this.value);
    const tau = goal > this.value ? this.attackMs : this.releaseMs;
    // Bound the step so a long gap (missed events, tab resume) eases instead of snapping.
    this.value += (goal - this.value) * smoothingCoefficient(Math.min(dtMs, MAX_STEP_MS), tau);
    return this.value;
  }

  reset(value = 0): void {
    this.value = clampUnit(value);
  }
}

/**
 * Fixed-size ring of recent spectrum frames, each `columns` values wide. The ridgeline
 * spectrogram draws its rear rows from older frames, so sound visibly recedes into the
 * distance. Never grows and never allocates after construction.
 */
export class SpectrumHistory {
  private readonly buffer: Float32Array;
  private head = 0;

  constructor(
    readonly frames: number,
    readonly columns: number,
  ) {
    this.buffer = new Float32Array(Math.max(1, frames) * Math.max(1, columns));
  }

  /** Store a new frame, overwriting the oldest. */
  push(values: Float32Array): void {
    this.head = (this.head + 1) % this.frames;
    const offset = this.head * this.columns;
    for (let column = 0; column < this.columns; column += 1) {
      const value = values[column];
      this.buffer[offset + column] = Number.isFinite(value) ? Math.max(0, Math.min(1, value)) : 0;
    }
  }

  /**
   * Value at a fractional `column` and a fractional age in frames (0 is the newest),
   * blended linearly in both directions and clamped to the stored range. Interpolating
   * across columns matters: ridges are drawn with more points than there are columns, and
   * reading whole columns would draw them as staircases.
   */
  sample(ageFrames: number, column: number): number {
    const age = Math.max(0, Math.min(this.frames - 1, Number.isFinite(ageFrames) ? ageFrames : 0));
    const newer = Math.floor(age);
    const older = Math.min(this.frames - 1, newer + 1);
    const ageFraction = age - newer;

    const position = Math.max(0, Math.min(this.columns - 1, Number.isFinite(column) ? column : 0));
    const left = Math.floor(position);
    const right = Math.min(this.columns - 1, left + 1);
    const columnFraction = position - left;

    const newerOffset = ((this.head - newer + this.frames) % this.frames) * this.columns;
    const olderOffset = ((this.head - older + this.frames) % this.frames) * this.columns;
    const newerValue =
      this.buffer[newerOffset + left] +
      (this.buffer[newerOffset + right] - this.buffer[newerOffset + left]) * columnFraction;
    const olderValue =
      this.buffer[olderOffset + left] +
      (this.buffer[olderOffset + right] - this.buffer[olderOffset + left]) * columnFraction;
    return newerValue + (olderValue - newerValue) * ageFraction;
  }

  fill(value: number): void {
    this.buffer.fill(Math.max(0, Math.min(1, value)));
  }
}
