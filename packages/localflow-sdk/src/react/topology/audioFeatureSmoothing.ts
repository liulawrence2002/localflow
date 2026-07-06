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
 * Fixed-size circular buffer of recent normalized levels. Rear terrain rows read
 * older entries so speech peaks propagate backward through the depth rows.
 * Never grows; writes overwrite the oldest entry.
 */
export class CircularHistory {
  private readonly buffer: Float32Array;
  private head = 0;

  constructor(
    readonly size: number,
    fill = 0,
  ) {
    this.buffer = new Float32Array(Math.max(1, Math.floor(size)));
    this.buffer.fill(clampUnit(fill));
  }

  push(value: number): void {
    this.head = (this.head + 1) % this.buffer.length;
    this.buffer[this.head] = clampUnit(value, this.buffer[this.head]);
  }

  /** Entry `stepsBack` writes ago; 0 is the newest. Clamped to the oldest entry. */
  at(stepsBack: number): number {
    const back = Math.max(0, Math.min(this.buffer.length - 1, Math.floor(stepsBack)));
    return this.buffer[(this.head - back + this.buffer.length) % this.buffer.length];
  }

  fill(value: number): void {
    this.buffer.fill(clampUnit(value));
  }
}

/** History offset for a depth row: front rows (depth→1) read the newest level,
 * rear rows (depth→0) read progressively older ones. */
export function historyStepsForDepth(depth: number, historySize: number): number {
  const clamped = clampUnit(depth, 1);
  return Math.round((1 - clamped) * Math.max(0, historySize - 1) * 0.6);
}
