/**
 * Pure math for the ridgeline spectrogram overlay. No canvas, no DOM, no timers, no
 * randomness: identical inputs always give identical output, so the picture cannot flicker
 * and every function is unit-tested directly.
 *
 * Conventions:
 * - A "band" is one of BAND_COUNT spectrum magnitudes in [0, 1], low to high frequency.
 * - A "column" is one of COLUMN_COUNT interpolated points along half a ridge. Ridges are
 *   mirrored: column 0 (lowest frequency) sits at the centre of the pill and the highest
 *   column at both ends, because speech energy is concentrated in the low bands.
 * - `progress` runs 0..1 left to right across the pill.
 */

import { smoothingCoefficient } from "./audioFeatureSmoothing";

export const BAND_COUNT = 32;
export const COLUMN_COUNT = 48;

const BAND_MIN_HZ = 70;
const BAND_MAX_HZ = 8000;
const LOG_RANGE = Math.log(BAND_MAX_HZ / BAND_MIN_HZ);

/** Magnitudes at or below this are treated as nothing, which hides residual hiss. */
const BAND_FLOOR = 0.04;
/** Level range over which the picture fades in, so a silent room draws a calm line. */
const GATE_LOW = 0.05;
const GATE_HIGH = 0.12;

const ATTACK_MS = 35;
const RELEASE_LOW_MS = 180;
const RELEASE_HIGH_MS = 110;

function clamp01(value: number): number {
  if (!Number.isFinite(value)) {
    return 0;
  }
  return value < 0 ? 0 : value > 1 ? 1 : value;
}

function smoothstep(low: number, high: number, value: number): number {
  const t = clamp01((value - low) / (high - low));
  return t * t * (3 - 2 * t);
}

/** Nominal frequency of a band on a log scale; used only to place synthetic formants. */
export function bandFrequency(index: number): number {
  return BAND_MIN_HZ * Math.exp((LOG_RANGE * index) / (BAND_COUNT - 1));
}

/** Fractional band index for a frequency; the inverse of {@link bandFrequency}. */
export function bandIndexForFrequency(hz: number): number {
  return ((BAND_COUNT - 1) * Math.log(Math.max(hz, 1e-6) / BAND_MIN_HZ)) / LOG_RANGE;
}

/**
 * Copy `source` (any length, magnitudes in 0..1) into `out` (BAND_COUNT values) with linear
 * resampling. Values outside 0..1 are clamped and non-finite values become 0.
 */
export function resampleBands(source: ArrayLike<number>, out: Float32Array): void {
  const length = source.length;
  if (length === 0) {
    out.fill(0);
    return;
  }
  const last = length - 1;
  const outLast = out.length - 1;
  for (let index = 0; index < out.length; index += 1) {
    const position = outLast === 0 ? 0 : (index / outLast) * last;
    const lower = Math.floor(position);
    const upper = Math.min(last, lower + 1);
    const fraction = position - lower;
    const a = clamp01(source[lower]);
    const b = clamp01(source[upper]);
    out[index] = a + (b - a) * fraction;
  }
}

/**
 * Age in history frames at which a ridge `rowsBack` rows behind the front reads the spectrum
 * history. `sinceLastPush` is the fraction (0..1) of a push interval elapsed since the
 * newest frame was stored. Subtracting it keeps each ridge showing a fixed delay behind the
 * live sound: as time passes it reads slightly newer history, and when the next frame is
 * pushed (every stored frame becomes one frame older, the fraction returns to 0) the content
 * under the ridge is unchanged. Adding it instead would make the ridges jump on every push.
 */
export function historyAge(rowsBack: number, framesPerRow: number, sinceLastPush: number): number {
  return Math.max(0, rowsBack * framesPerRow - clamp01(sinceLastPush));
}

/**
 * Plausible bands for consumers that only provide level, pitch and brightness: four
 * formant-like bumps whose positions follow pitch and brightness, scaled by level. It is a
 * stand-in, not an analysis, but it moves the way a voice does.
 */
export function synthesizeBands(
  out: Float32Array,
  level: number,
  pitch: number,
  brightness: number,
  timeSec: number,
): void {
  const loudness = Math.pow(clamp01(level), 0.8);
  if (loudness === 0) {
    out.fill(0);
    return;
  }
  const p = clamp01(pitch);
  const b = clamp01(brightness);
  const t = Number.isFinite(timeSec) ? timeSec : 0;

  const centres = [
    bandIndexForFrequency(80 * Math.pow(4, p)),
    bandIndexForFrequency(350 + 450 * b),
    bandIndexForFrequency(1000 + 1400 * b),
    bandIndexForFrequency(2800 + 2500 * b),
  ];
  const sigmas = [1.6, 2.2, 2.0, 3.0];
  const amplitudes = [0.85, 1.0, 0.7, 0.25 + 0.55 * b];

  for (let index = 0; index < out.length; index += 1) {
    let sum = 0;
    for (let k = 0; k < 4; k += 1) {
      const centre = centres[k] + 0.8 * Math.sin(2.1 * t + 1.7 * k);
      const amplitude = amplitudes[k] * (0.85 + 0.15 * Math.sin(3.3 * t + 2.3 * k));
      const distance = (index - centre) / sigmas[k];
      sum += amplitude * Math.exp(-0.5 * distance * distance);
    }
    const rolloff = 1 - (0.35 * index) / (out.length - 1);
    out[index] = clamp01(loudness * rolloff * sum);
  }
}

/**
 * Prepare raw bands for display, in place: remove the floor, soften single-band spikes with
 * a 3-tap blur, and fade everything out as the level approaches silence.
 */
export function conditionBands(bands: Float32Array, level: number): void {
  const gate = smoothstep(GATE_LOW, GATE_HIGH, clamp01(level));
  let previous = 0;
  let current = Math.max(0, (clamp01(bands[0]) - BAND_FLOOR) / (1 - BAND_FLOOR));
  for (let index = 0; index < bands.length; index += 1) {
    const next =
      index + 1 < bands.length
        ? Math.max(0, (clamp01(bands[index + 1]) - BAND_FLOOR) / (1 - BAND_FLOOR))
        : 0;
    const edge = index === 0 || index === bands.length - 1;
    // The ends keep their own weight so the blur does not darken the outermost bands.
    const blurred = edge
      ? 0.75 * current + 0.25 * (index === 0 ? next : previous)
      : 0.25 * previous + 0.5 * current + 0.25 * next;
    bands[index] = clamp01(blurred * gate);
    previous = current;
    current = next;
  }
}

/**
 * Move `current` toward `target` with a fast attack and a slower release. High bands
 * release faster than low ones, as consonants die away faster than vowels. Frame-rate
 * independent: two short steps equal one long step.
 */
export function smoothBands(current: Float32Array, target: Float32Array, dtMs: number): void {
  const attack = smoothingCoefficient(dtMs, ATTACK_MS);
  const last = Math.max(1, current.length - 1);
  for (let index = 0; index < current.length; index += 1) {
    const goal = clamp01(target[index]);
    const value = current[index];
    if (goal > value) {
      current[index] = value + (goal - value) * attack;
    } else {
      const releaseMs = RELEASE_LOW_MS + ((RELEASE_HIGH_MS - RELEASE_LOW_MS) * index) / last;
      current[index] = value + (goal - value) * smoothingCoefficient(dtMs, releaseMs);
    }
  }
}

/** Uniform Catmull-Rom spline through p1..p2 with neighbours p0 and p3. */
export function catmullRom(p0: number, p1: number, p2: number, p3: number, t: number): number {
  const t2 = t * t;
  const t3 = t2 * t;
  return (
    0.5 *
    (2 * p1 +
      (-p0 + p2) * t +
      (2 * p0 - 5 * p1 + 4 * p2 - p3) * t2 +
      (-p0 + 3 * p1 - 3 * p2 + p3) * t3)
  );
}

/** Interpolate bands into smooth columns, clamped so the spline never overshoots [0, 1]. */
export function bandsToColumns(bands: Float32Array, out: Float32Array): void {
  const last = bands.length - 1;
  const outLast = Math.max(1, out.length - 1);
  for (let column = 0; column < out.length; column += 1) {
    const position = (column / outLast) * last;
    const index = Math.min(last - 1, Math.floor(position));
    const t = position - index;
    const p0 = bands[Math.max(0, index - 1)];
    const p1 = bands[index];
    const p2 = bands[index + 1];
    const p3 = bands[Math.min(last, index + 2)];
    out[column] = clamp01(catmullRom(p0, p1, p2, p3, t));
  }
}

/** Column index for a point at `progress`: 0 at the centre, COLUMN_COUNT - 1 at both ends. */
export function mirroredColumn(progress: number, columns: number): number {
  const distance = Math.min(1, Math.abs(2 * clamp01(progress) - 1));
  return distance * (columns - 1);
}

/** Height envelope that brings every ridge down to its baseline at the pill's ends. */
export function ridgeEnvelope(progress: number): number {
  if (!(progress > 0) || !(progress < 1)) {
    return 0;
  }
  const distance = Math.abs(2 * progress - 1);
  const squared = distance * distance;
  return 1 - squared * squared;
}

/**
 * Gentle non-microphone ripple in [0, 1] so idle and waiting states still breathe. `row`
 * offsets the phase so neighbouring ridges are related but not identical.
 */
export function ambientRidge(progress: number, row: number, timeSec: number): number {
  const slow = Math.sin(progress * 6.2 - timeSec * 1.3 + row * 0.7);
  const fine = Math.sin(progress * 13.1 + timeSec * 0.7 + row * 1.9);
  return clamp01(0.5 + 0.32 * slow + 0.18 * fine);
}

/**
 * Brightness of the travelling crest used while processing, in [0, 1]: a Gaussian centred
 * on `sweepPosition`, delayed slightly on rear rows so it reads as moving through depth.
 */
export function sweepCrest(progress: number, sweepPosition: number, row: number): number {
  const offset = (progress - sweepPosition - 0.03 * row) * 4.2;
  return Math.exp(-offset * offset);
}

/**
 * Keep a y coordinate from crossing `top`: unchanged at or below `knee` (further down the
 * canvas), then approaching `top` asymptotically. Unlike a symmetric clamp it never moves
 * a baseline.
 */
export function softCeiling(y: number, top: number, knee: number): number {
  if (!(y < knee)) {
    return y;
  }
  const span = knee - top;
  if (!(span > 0)) {
    return knee;
  }
  return top + span * Math.exp((y - knee) / span);
}

const HOT: readonly [number, number, number] = [240, 247, 252];

/**
 * Colour for a ridge at brightness `t` (0 dim and distant, 1 bright and energetic). The
 * ramp runs from a darkened accent through the accent to near-white; `strength` mixes it
 * with a neutral grey ramp so the phase tint stays restrained. Writes RGB into `out`.
 */
export function rampColor(
  t: number,
  accent: readonly [number, number, number],
  strength: number,
  out: Uint8ClampedArray,
): void {
  const position = clamp01(t);
  const mix = clamp01(strength);
  for (let channel = 0; channel < 3; channel += 1) {
    const tinted = rampChannel(position, accent[channel], HOT[channel]);
    const neutral = rampChannel(position, NEUTRAL_MID[channel], HOT[channel]);
    out[channel] = neutral + (tinted - neutral) * mix;
  }
}

const NEUTRAL_MID: readonly [number, number, number] = [150, 160, 172];

function rampChannel(position: number, mid: number, hot: number): number {
  const dark = mid * 0.6;
  const light = mid + (255 - mid) * 0.5;
  if (position < 0.35) {
    return dark + (mid - dark) * (position / 0.35);
  }
  if (position < 0.7) {
    return mid + (light - mid) * ((position - 0.35) / 0.35);
  }
  return light + (hot - light) * ((position - 0.7) / 0.3);
}
