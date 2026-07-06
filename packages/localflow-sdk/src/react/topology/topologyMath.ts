/**
 * Deterministic surface + projection math for the topographic waveform.
 * Everything here is a pure function of its inputs (no randomness, no Date.now)
 * so identical inputs always produce identical terrain — flicker-free by design.
 *
 * Coordinate conventions:
 * - `progress` runs 0..1 left to right across the pill.
 * - `depth` runs 0 (rear-most row) to 1 (foreground row).
 * - Surface heights are unitless, roughly [-1, 1.6]; the projection scales them.
 */

export interface SmoothedFeatures {
  /** Voice drive already mixed with the phase's ambient floor, 0..1. */
  level: number;
  pitch: number;
  brightness: number;
}

export interface SurfaceMotion {
  /** Overall displacement multiplier (reduced-motion lowers this). */
  displacement: number;
  /** Time multiplier for traveling waves. */
  travel: number;
  /** Intensity of the non-microphone processing sweep, 0..1. */
  sweep: number;
  /** Sweep crest position, 0..1 across the pill. */
  sweepPosition: number;
  /** 0 = full terrain, 1 = flattened to a resting line (success/cancel). */
  collapse: number;
}

/** Smooth 0-at-edges envelope so the terrain fades into the pill's rounded ends. */
export function edgeEnvelope(progress: number): number {
  if (!(progress > 0) || !(progress < 1)) {
    return 0;
  }
  const base = Math.sin(Math.PI * progress);
  return Math.pow(base, 0.8);
}

/**
 * Terrain height at one sample point. Composition (per the design spec):
 * primary voice wave + brightness-driven harmonic + low-frequency swell
 * + small deterministic depth variation + center ridge that grows with speech
 * + optional traveling sweep crest used by processing/refining.
 */
export function surfaceHeight(
  progress: number,
  depth: number,
  timeSec: number,
  features: SmoothedFeatures,
  motion: SurfaceMotion,
): number {
  const envelope = edgeEnvelope(progress);
  if (envelope === 0) {
    return 0;
  }

  const t = timeSec * motion.travel;
  // Pitch sets spatial frequency (ridge spacing), never amplitude.
  const primaryOmega = (3.4 + features.pitch * 3.2) * Math.PI;
  const harmonicOmega = (7.5 + features.brightness * 4.5) * Math.PI;

  const primary = Math.sin(progress * primaryOmega + t * 1.15 + depth * 2.3) * 0.52;
  const harmonic =
    Math.sin(progress * harmonicOmega - t * 0.85 + depth * 4.7) *
    (0.14 + features.brightness * 0.22);
  const swell = Math.sin(progress * Math.PI * 1.7 + t * 0.32 + depth * 1.4) * 0.24;
  // Deterministic per-depth variation keeps rows related but not duplicates.
  const depthVariation = Math.sin(depth * 11.3 + progress * Math.PI * 5.1 - t * 0.22) * 0.09;

  const drive = 0.18 + features.level * 0.82;
  const ridgeFalloff = (progress - 0.5) * 3.1;
  const centerRidge =
    features.level * Math.exp(-(ridgeFalloff * ridgeFalloff)) * (0.5 + depth * 0.5) * 0.9;

  const sweepFalloff = (progress - motion.sweepPosition) * 4.2;
  const sweepCrest =
    motion.sweep > 0 ? motion.sweep * Math.exp(-(sweepFalloff * sweepFalloff)) * 0.85 : 0;

  const shape = (primary + harmonic + swell + depthVariation) * drive + centerRidge + sweepCrest;
  return shape * envelope * motion.displacement * (1 - motion.collapse);
}

export interface Viewport {
  width: number;
  height: number;
}

export interface ProjectedPoint {
  x: number;
  y: number;
}

const REAR_INSET_EXTRA = 0.052; // rear rows are narrower → trapezoid perspective
const BASE_INSET = 0.058;
const TOP_PAD = 0.16;
const ROW_SPAN = 0.66;
const DEPTH_CURVE = 1.18;

/** Baseline (zero-height) y position of a depth row, in logical pixels. */
export function rowBaseline(depth: number, viewport: Viewport): number {
  return viewport.height * (TOP_PAD + Math.pow(depth, DEPTH_CURVE) * ROW_SPAN);
}

/** Vertical scale for surface heights at a given depth: foreground moves more. */
export function heightScale(depth: number, viewport: Viewport): number {
  return viewport.height * (0.1 + depth * 0.17);
}

/**
 * Oblique projection of one terrain sample into pill-local logical pixels.
 * Rear rows sit higher, span a narrower x range, and move less vertically.
 */
export function projectPoint(
  progress: number,
  depth: number,
  height: number,
  viewport: Viewport,
): ProjectedPoint {
  const inset = viewport.width * (BASE_INSET + (1 - depth) * REAR_INSET_EXTRA);
  const x = inset + progress * (viewport.width - inset * 2);
  const rawY = rowBaseline(depth, viewport) - height * heightScale(depth, viewport);
  return { x, y: softClampY(rawY, viewport) };
}

/** Soft (tanh) clamp keeping every point inside the pill with a smooth approach,
 * so loud peaks compress instead of clipping against the border. */
export function softClampY(y: number, viewport: Viewport): number {
  const center = viewport.height * 0.5;
  const limit = viewport.height * 0.44;
  return center + Math.tanh((y - center) / limit) * limit;
}

/** Row opacity by depth: rear rows dim, foreground rows bright. */
export function depthAlpha(depth: number, activity: number): number {
  return Math.min(1, 0.1 + Math.pow(depth, 1.5) * 0.42 + activity * 0.22 * depth);
}

/** Dot radius by depth and activity, in logical pixels. */
export function dotRadius(depth: number, activity: number): number {
  return 0.7 + depth * 0.85 + activity * depth * 0.55;
}
