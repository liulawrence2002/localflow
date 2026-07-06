import type { LocalFlowDictationPhase } from "../../types";

/**
 * Mapping from the application's real dictation phases to visual parameters.
 * This is intentionally the ONLY place phase semantics enter the renderer, and
 * it introduces no second state machine — it is a pure projection of the phase.
 */
export interface PhaseVisuals {
  /** How much live microphone level drives the terrain (0 = ignore mic). */
  micDrive: number;
  /** Ambient energy floor so silence still breathes instead of collapsing. */
  energyFloor: number;
  /** Traveling-wave speed multiplier. */
  travelSpeed: number;
  /** Non-microphone sweep intensity (processing/refining), 0..1. */
  sweep: number;
  /** Sweep cycles per second across the pill. */
  sweepSpeed: number;
  /** Restrained accent tint mixed into the near-monochrome dots. */
  accent: readonly [number, number, number];
  /** 0..1 how strongly the accent tint is mixed in. */
  accentStrength: number;
  /** 0..1 flatten toward a calm resting line (success/cancel). */
  collapse: number;
  /** Extra dot brightness for active speech. */
  brightnessBoost: number;
}

const NEUTRAL: readonly [number, number, number] = [235, 240, 245];
const ACCENT_BLUE: readonly [number, number, number] = [96, 165, 210];
const ACCENT_TEAL: readonly [number, number, number] = [88, 172, 150];
const ACCENT_GREEN: readonly [number, number, number] = [72, 176, 118];
const ACCENT_RED: readonly [number, number, number] = [214, 92, 74];

const AMBIENT: PhaseVisuals = {
  micDrive: 0,
  energyFloor: 0.1,
  travelSpeed: 0.55,
  sweep: 0,
  sweepSpeed: 0,
  accent: NEUTRAL,
  accentStrength: 0,
  collapse: 0,
  brightnessBoost: 0,
};

const BY_PHASE: Record<LocalFlowDictationPhase, PhaseVisuals> = {
  idle: AMBIENT,
  ready: {
    ...AMBIENT,
    energyFloor: 0.16,
    travelSpeed: 0.7,
    accent: ACCENT_TEAL,
    accentStrength: 0.3,
  },
  listening: {
    ...AMBIENT,
    micDrive: 1,
    energyFloor: 0.12,
    travelSpeed: 1,
    accent: ACCENT_BLUE,
    accentStrength: 0.18,
    brightnessBoost: 0.35,
  },
  processing: {
    ...AMBIENT,
    micDrive: 0,
    energyFloor: 0.2,
    travelSpeed: 0.75,
    sweep: 0.8,
    sweepSpeed: 0.42,
    accent: ACCENT_BLUE,
    accentStrength: 0.34,
  },
  refining: {
    ...AMBIENT,
    micDrive: 0,
    energyFloor: 0.18,
    travelSpeed: 0.5,
    sweep: 0.55,
    sweepSpeed: 0.22,
    accent: ACCENT_TEAL,
    accentStrength: 0.3,
  },
  inserted: {
    ...AMBIENT,
    energyFloor: 0.08,
    travelSpeed: 0.45,
    accent: ACCENT_GREEN,
    accentStrength: 0.42,
    collapse: 0.72,
  },
  cancelled: {
    ...AMBIENT,
    energyFloor: 0.04,
    travelSpeed: 0.35,
    accentStrength: 0,
    collapse: 1,
  },
  error: {
    ...AMBIENT,
    energyFloor: 0.14,
    travelSpeed: 0.6,
    accent: ACCENT_RED,
    accentStrength: 0.45,
    brightnessBoost: 0.1,
  },
};

/**
 * Visual parameters for a phase. Reduced motion keeps feedback (levels, tints)
 * but lowers displacement-driving speeds and disables the traveling sweep.
 */
export function visualsForPhase(
  phase: LocalFlowDictationPhase,
  reducedMotion: boolean,
): PhaseVisuals {
  const visuals = BY_PHASE[phase] ?? AMBIENT;
  if (!reducedMotion) {
    return visuals;
  }
  return {
    ...visuals,
    travelSpeed: visuals.travelSpeed * 0.3,
    sweep: 0,
    sweepSpeed: 0,
    micDrive: visuals.micDrive * 0.6,
  };
}

/** Displacement multiplier for the surface, honoring reduced motion. */
export function displacementForMotion(reducedMotion: boolean): number {
  return reducedMotion ? 0.45 : 1;
}
