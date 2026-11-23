import type { LocalFlowDictationPhase } from "../../types";
import { FeatureSmoother, SpectrumHistory, smoothingCoefficient } from "./audioFeatureSmoothing";
import { displacementForMotion, visualsForPhase, type PhaseVisuals } from "./overlayStateVisuals";
import {
  BAND_COUNT,
  COLUMN_COUNT,
  ambientRidge,
  bandsToColumns,
  conditionBands,
  historyAge,
  mirroredColumn,
  rampColor,
  resampleBands,
  ridgeEnvelope,
  smoothBands,
  softCeiling,
  sweepCrest,
  synthesizeBands,
} from "./spectrumMath";

export type TopologyQuality = "balanced" | "low";

export interface RendererTimingHooks {
  requestFrame: (callback: (timeMs: number) => void) => number;
  cancelFrame: (handle: number) => void;
  now: () => number;
}

export interface AudioFeatureTargets {
  level: number | null | undefined;
  pitch: number | null | undefined;
  brightness: number | null | undefined;
  /**
   * Spectrum magnitudes in 0..1, low to high frequency. When absent (or shorter than
   * MIN_REAL_BANDS) the renderer synthesises a plausible spectrum from level, pitch and
   * brightness.
   */
  bands?: ArrayLike<number> | null;
}

interface QualityProfile {
  rows: number;
  points: number;
  minFrameIntervalMs: number;
  glow: boolean;
}

const QUALITY_PROFILES: Record<TopologyQuality, QualityProfile> = {
  balanced: { rows: 9, points: 128, minFrameIntervalMs: 0, glow: true },
  low: { rows: 5, points: 48, minFrameIntervalMs: 32, glow: false },
};

/** Rows drawn when the user prefers reduced motion: enough for depth, little movement. */
const REDUCED_MOTION_ROWS = 3;
const MAX_ROWS = 9;
const MAX_POINTS = 128;

const MIN_REAL_BANDS = 8;
const HISTORY_FRAMES = 24;
const HISTORY_PUSH_INTERVAL_MS = 50;
/** History frames between neighbouring rows: nine rows span about 800 ms of sound. */
const HISTORY_FRAMES_PER_ROW = 2;
const PHASE_BLEND_TAU_MS = 140;

// Geometry, as fractions of the canvas height or width.
const REAR_BASELINE = 0.27;
const FRONT_BASELINE = 0.87;
const DEPTH_CURVE = 1.18;
const REAR_HEIGHT = 0.26;
const FRONT_HEIGHT = 0.64;
const BASE_INSET = 0.058;
const REAR_INSET_EXTRA = 0.052;
/** Rear ridges are drawn lower so the newest sound always reads as the tallest. */
const REAR_ATTENUATION = 0.45;
/** How much of what lies behind a ridge it hides; the remainder reads as depth fog. */
const OCCLUSION = 0.88;

/**
 * Colour by frequency while listening, as stops across the pill. The layout is mirrored, so
 * the centre is the lowest frequency and both ends are the highest: warm low tones in the
 * middle, through cyan, to violet highs at the edges.
 */
const SPECTRUM_STOPS: ReadonlyArray<readonly [number, string]> = [
  [0, "rgb(168, 132, 255)"],
  [0.2, "rgb(96, 170, 255)"],
  [0.38, "rgb(84, 222, 236)"],
  [0.5, "rgb(206, 255, 236)"],
  [0.62, "rgb(84, 222, 236)"],
  [0.8, "rgb(96, 170, 255)"],
  [1, "rgb(168, 132, 255)"],
];

const IDLE_PHASES: ReadonlySet<LocalFlowDictationPhase> = new Set([
  "idle",
  "inserted",
  "cancelled",
  "error",
]);

/**
 * Owns the animation loop and all mutable visualization state for the voice overlay.
 * Contains no dictation business logic: phases come in via `setPhase`, audio features via
 * `setFeatureTargets` or a polled source.
 *
 * Rendering model: a mirrored ridgeline spectrogram. The front ridge is the live spectrum
 * (low frequencies at the centre, highs toward both ends); each ridge behind it is the
 * spectrum a little earlier, so sound recedes into the distance. Ridges are drawn back to
 * front; each one first erases what it covers, then strokes its line, which gives true
 * hidden-line depth without painting a background over the frosted pill. All per-frame
 * work uses preallocated typed arrays.
 */
export class TopologyRenderer {
  private canvas: HTMLCanvasElement | null = null;
  private context: CanvasRenderingContext2D | null = null;
  private readonly timing: RendererTimingHooks;

  private frameHandle: number | null = null;
  private running = false;
  private disposed = false;
  private lastFrameAt: number | null = null;
  private timeSec = 0;
  private sweepCycle = 0;
  private historyAccumulatorMs = 0;

  private width = 0;
  private height = 0;

  private quality: QualityProfile = QUALITY_PROFILES.balanced;
  private reducedMotion = false;

  private phase: LocalFlowDictationPhase = "idle";
  private phaseVisuals: PhaseVisuals = visualsForPhase("idle", false);

  private readonly level = new FeatureSmoother(60, 260, 0.08);
  private readonly pitch = new FeatureSmoother(180, 240, 0.5);
  private readonly brightness = new FeatureSmoother(150, 260, 0.35);
  private targets: AudioFeatureTargets = { level: 0.08, pitch: 0.5, brightness: 0.35 };
  private featureSource: (() => AudioFeatureTargets | null) | null = null;

  // Spectrum state. `targetBands` is the latest conditioned input, `bands` follows it with
  // attack and release smoothing, `columns` is `bands` interpolated for drawing.
  private readonly targetBands = new Float32Array(BAND_COUNT);
  private readonly bands = new Float32Array(BAND_COUNT);
  private readonly columns = new Float32Array(COLUMN_COUNT);
  private readonly history = new SpectrumHistory(HISTORY_FRAMES, COLUMN_COUNT);

  // Geometry cache, rebuilt on resize or quality change.
  private readonly pointX = new Float32Array(MAX_ROWS * MAX_POINTS);
  private readonly pointColumn = new Float32Array(MAX_POINTS);
  private readonly pointEnvelope = new Float32Array(MAX_POINTS);
  private readonly pointProgress = new Float32Array(MAX_POINTS);
  private readonly pointY = new Float32Array(MAX_POINTS);
  private readonly rowBaseline = new Float32Array(MAX_ROWS);
  private readonly rowScale = new Float32Array(MAX_ROWS);
  private geometryRows = 0;
  private geometryPoints = 0;

  private readonly rgb = new Uint8ClampedArray(3);
  private underFill: CanvasGradient | null = null;
  private spectrumStroke: CanvasGradient | null = null;

  // Phase-derived parameters are themselves smoothed so state transitions
  // (success collapse, error tint, processing sweep) ease in rather than snap.
  private blend = {
    micDrive: 0,
    energyFloor: 0.1,
    sweep: 0,
    collapse: 0,
    historyGain: 0,
    accentStrength: 0,
    accentR: 235,
    accentG: 240,
    accentB: 245,
  };
  private readonly accent: [number, number, number] = [235, 240, 245];

  private detachEnvironment: (() => void) | null = null;
  private wasRunningBeforeHide = false;

  constructor(hooks?: Partial<RendererTimingHooks>) {
    this.timing = {
      requestFrame: hooks?.requestFrame ?? ((callback) => window.requestAnimationFrame(callback)),
      cancelFrame: hooks?.cancelFrame ?? ((handle) => window.cancelAnimationFrame(handle)),
      now: hooks?.now ?? (() => performance.now()),
    };
  }

  attach(canvas: HTMLCanvasElement): void {
    if (this.disposed) {
      return;
    }
    this.canvas = canvas;
    this.context = canvas.getContext("2d");
    this.observeEnvironment();
    this.resize();
  }

  start(): void {
    if (this.disposed || this.running) {
      return;
    }
    this.running = true;
    this.lastFrameAt = null;
    this.scheduleFrame();
  }

  stop(): void {
    this.running = false;
    if (this.frameHandle !== null) {
      this.timing.cancelFrame(this.frameHandle);
      this.frameHandle = null;
    }
  }

  dispose(): void {
    this.stop();
    this.disposed = true;
    this.detachEnvironment?.();
    this.detachEnvironment = null;
    this.canvas = null;
    this.context = null;
    this.underFill = null;
    this.spectrumStroke = null;
  }

  setPhase(phase: LocalFlowDictationPhase): void {
    if (phase === this.phase) {
      return;
    }
    // A fresh session must never inherit the previous session's picture. Listening can
    // also begin while an earlier dictation is still processing, so it always starts clean.
    if (phase === "listening" || (phase === "ready" && IDLE_PHASES.has(this.phase))) {
      this.resetDynamics();
    }
    this.phase = phase;
    this.phaseVisuals = visualsForPhase(phase, this.reducedMotion);
  }

  setFeatureTargets(targets: AudioFeatureTargets): void {
    this.targets = targets;
  }

  /** Optional pull-based feature source (a React ref), polled once per frame so
   * feature updates never require a React render. */
  setFeatureSource(source: (() => AudioFeatureTargets | null) | null): void {
    this.featureSource = source;
  }

  setQuality(quality: TopologyQuality): void {
    this.quality = QUALITY_PROFILES[quality] ?? QUALITY_PROFILES.balanced;
    this.rebuildGeometry();
  }

  setReducedMotion(reducedMotion: boolean): void {
    this.reducedMotion = reducedMotion;
    this.phaseVisuals = visualsForPhase(this.phase, reducedMotion);
    this.rebuildGeometry();
  }

  /** Clear smoothed levels, spectrum, peaks and history (new session, or shown after
   * hidden). */
  resetDynamics(): void {
    this.level.reset(0.08);
    this.pitch.reset(0.5);
    this.brightness.reset(0.35);
    // Drop the last pushed targets too: stale spectrum bands from the previous session
    // must not reappear before the first new audio frame arrives.
    this.targets = { level: 0.08, pitch: 0.5, brightness: 0.35 };
    this.targetBands.fill(0);
    this.bands.fill(0);
    this.columns.fill(0);
    this.history.fill(0);
    this.historyAccumulatorMs = 0;
    this.blend.sweep = 0;
    this.blend.collapse = 0;
  }

  get isRunning(): boolean {
    return this.running;
  }

  /** Smoothed microphone level; exposed for tests and diagnostics only. */
  get currentLevel(): number {
    return this.level.value;
  }

  /** Smoothed spectrum bands; exposed for tests and diagnostics only. */
  get currentBands(): Float32Array {
    return this.bands;
  }

  /** Advance and draw one frame; exposed for deterministic tests. */
  stepFrame(timeMs: number): void {
    const dtMs = this.lastFrameAt === null ? 16.7 : Math.min(100, timeMs - this.lastFrameAt);
    this.lastFrameAt = timeMs;
    if (dtMs <= 0) {
      return;
    }
    this.update(dtMs);
    this.draw();
  }

  private scheduleFrame(): void {
    if (!this.running || this.disposed) {
      return;
    }
    this.frameHandle = this.timing.requestFrame((timeMs) => {
      this.frameHandle = null;
      if (!this.running || this.disposed) {
        return;
      }
      if (this.quality.minFrameIntervalMs > 0 && this.lastFrameAt !== null) {
        const elapsed = timeMs - this.lastFrameAt;
        if (elapsed < this.quality.minFrameIntervalMs) {
          this.scheduleFrame();
          return;
        }
      }
      this.stepFrame(timeMs);
      this.scheduleFrame();
    });
  }

  private update(dtMs: number): void {
    const visuals = this.phaseVisuals;
    this.timeSec += (dtMs / 1000) * visuals.travelSpeed;

    const pulled = this.featureSource?.();
    if (pulled) {
      this.targets = pulled;
    }

    this.level.update(this.targets.level, dtMs);
    this.pitch.update(this.targets.pitch, dtMs);
    this.brightness.update(this.targets.brightness, dtMs);

    const k = smoothingCoefficient(dtMs, PHASE_BLEND_TAU_MS);
    const blend = this.blend;
    blend.micDrive += (visuals.micDrive - blend.micDrive) * k;
    blend.energyFloor += (visuals.energyFloor - blend.energyFloor) * k;
    blend.sweep += (visuals.sweep - blend.sweep) * k;
    blend.collapse += (visuals.collapse - blend.collapse) * k;
    blend.historyGain += (visuals.historyGain - blend.historyGain) * k;
    blend.accentStrength += (visuals.accentStrength - blend.accentStrength) * k;
    blend.accentR += (visuals.accent[0] - blend.accentR) * k;
    blend.accentG += (visuals.accent[1] - blend.accentG) * k;
    blend.accentB += (visuals.accent[2] - blend.accentB) * k;

    if (visuals.sweepSpeed > 0) {
      this.sweepCycle = (this.sweepCycle + (dtMs / 1000) * visuals.sweepSpeed) % 1;
    }

    this.updateSpectrum(dtMs, visuals);
  }

  private updateSpectrum(dtMs: number, visuals: PhaseVisuals): void {
    // While the microphone is not driving the picture the live spectrum eases to nothing;
    // what stays visible is the frozen history.
    if (visuals.micDrive > 0) {
      const source = this.targets.bands;
      if (source && source.length >= MIN_REAL_BANDS) {
        resampleBands(source, this.targetBands);
      } else {
        synthesizeBands(
          this.targetBands,
          this.level.value,
          this.pitch.value,
          this.brightness.value,
          this.timeSec,
        );
      }
      conditionBands(this.targetBands, this.level.value);
      if (visuals.micDrive < 1) {
        for (let index = 0; index < BAND_COUNT; index += 1) {
          this.targetBands[index] *= visuals.micDrive;
        }
      }
    } else {
      this.targetBands.fill(0);
    }

    smoothBands(this.bands, this.targetBands, dtMs);
    bandsToColumns(this.bands, this.columns);

    if (visuals.freeze) {
      // Stop recording but leave the accumulator alone: every ridge keeps reading exactly
      // the history it was showing, so nothing jumps when the freeze begins.
      return;
    }
    this.historyAccumulatorMs += dtMs;
    while (this.historyAccumulatorMs >= HISTORY_PUSH_INTERVAL_MS) {
      this.historyAccumulatorMs -= HISTORY_PUSH_INTERVAL_MS;
      this.history.push(this.columns);
    }
  }

  private activeRows(): number {
    return this.reducedMotion
      ? Math.min(REDUCED_MOTION_ROWS, this.quality.rows)
      : this.quality.rows;
  }

  private draw(): void {
    const context = this.context;
    if (!context || this.width <= 0 || this.height <= 0) {
      return;
    }
    if (this.geometryRows === 0) {
      this.rebuildGeometry();
    }

    context.clearRect(0, 0, this.width, this.height);

    const rows = this.geometryRows;
    const points = this.geometryPoints;
    const blend = this.blend;
    const displacement = displacementForMotion(this.reducedMotion) * (1 - blend.collapse);
    // Sweep travels slightly past both edges so it enters and exits smoothly.
    const sweepPosition = this.sweepCycle * 1.3 - 0.15;
    const sinceLastPush = this.historyAccumulatorMs / HISTORY_PUSH_INTERVAL_MS;
    const top = 3;
    const knee = this.height * 0.2;
    const bottom = this.height + 4;
    // While listening the ridges are coloured by frequency; in every other phase they take
    // the phase's accent. `micDrive` is already smoothed, so the two cross-fade.
    const spectrumMix = this.spectrumStroke ? Math.min(1, Math.max(0, blend.micDrive)) : 0;

    this.accent[0] = blend.accentR;
    this.accent[1] = blend.accentG;
    this.accent[2] = blend.accentB;
    context.lineJoin = "round";
    context.lineCap = "round";

    for (let rowIndex = 0; rowIndex < rows; rowIndex += 1) {
      const depth = rows === 1 ? 1 : rowIndex / (rows - 1);
      const rowsBack = rows - 1 - rowIndex;
      const isFront = rowsBack === 0;
      const age = historyAge(rowsBack, HISTORY_FRAMES_PER_ROW, sinceLastPush);
      const attenuation = 1 - REAR_ATTENUATION * (1 - depth);
      const baseline = this.rowBaseline[rowIndex];
      const scale = this.rowScale[rowIndex] * displacement;
      const ambientGain = blend.energyFloor * 0.3;
      const rowOffset = rowIndex * MAX_POINTS;

      let energy = 0;
      for (let point = 0; point < points; point += 1) {
        const column = this.pointColumn[point];
        const progress = this.pointProgress[point];
        // The front ridge reads the live smoothed spectrum so it responds at once; ridges
        // behind it read the recorded history at their age.
        const live = isFront ? this.sampleColumns(column) : 0;
        const recorded = this.history.sample(age, column);
        const sound = Math.max(live, recorded) * attenuation * blend.historyGain;
        const ambient = ambientGain * ambientRidge(progress, rowIndex, this.timeSec);
        const crest = blend.sweep > 0 ? sweepCrest(progress, sweepPosition, rowsBack) : 0;
        // The ambient ripple fades out smoothly under real sound, so the two never meet at
        // a visible kink.
        const lit =
          (sound + ambient * (1 - Math.min(1, sound * 3))) *
          (1 + blend.sweep * (0.75 * crest - 0.45));
        const value = (lit + blend.sweep * 0.22 * crest) * this.pointEnvelope[point];
        energy += value;
        this.pointY[point] = softCeiling(baseline - value * scale, top, knee);
      }
      energy /= points;

      // One closed path per ridge: off-canvas corners keep the closing edges invisible.
      context.beginPath();
      context.moveTo(-4, bottom);
      context.lineTo(-4, baseline);
      for (let point = 0; point < points; point += 1) {
        context.lineTo(this.pointX[rowOffset + point], this.pointY[point]);
      }
      context.lineTo(this.width + 4, baseline);
      context.lineTo(this.width + 4, bottom);

      // Erase what this ridge covers. The canvas has no background of its own, so erasing
      // shows the frosted pill through rather than painting a dark slab.
      context.globalCompositeOperation = "destination-out";
      context.fillStyle = OCCLUSION_STYLE;
      context.fill();
      context.globalCompositeOperation = "source-over";

      if (isFront && this.underFill) {
        context.fillStyle = this.underFill;
        context.fill();
      }

      rampColor(
        // Kept below the near-white end of the ramp so the phase tint stays visible.
        0.1 + 0.42 * depth + 0.28 * Math.min(1, energy * 2.2),
        this.accent,
        0.5 + 0.5 * blend.accentStrength,
        this.rgb,
      );
      const r = this.rgb[0];
      const g = this.rgb[1];
      const b = this.rgb[2];
      const alpha = 0.14 + 0.86 * Math.pow(depth, 1.6);

      const accentStyle = `rgb(${r}, ${g}, ${b})`;
      const glow =
        this.quality.glow && rowsBack < 2
          ? (0.5 + 0.5 * Math.min(1, energy * 3)) * (isFront ? 1 : 0.6)
          : 0;
      const lineWidth = 0.75 + 0.85 * depth;

      if (spectrumMix < 0.98) {
        this.strokeRidge(context, accentStyle, alpha * (1 - spectrumMix), glow, lineWidth);
      }
      if (spectrumMix > 0.02 && this.spectrumStroke) {
        // Louder ridges are more saturated; quiet distant ones stay faint.
        const vivid = alpha * spectrumMix * (0.7 + 0.3 * Math.min(1, energy * 3));
        this.strokeRidge(context, this.spectrumStroke, vivid, glow, lineWidth);
      }
      context.globalAlpha = 1;
    }
  }

  /**
   * Stroke the current path as one ridge: two wide faint passes for glow (cheaper than
   * shadowBlur), then the core line. Opacity goes through `globalAlpha` so the same
   * prebuilt gradient serves every row.
   */
  private strokeRidge(
    context: CanvasRenderingContext2D,
    style: string | CanvasGradient,
    alpha: number,
    glow: number,
    lineWidth: number,
  ): void {
    context.strokeStyle = style;
    if (glow > 0) {
      context.globalAlpha = Math.min(1, alpha * 0.1 * glow);
      context.lineWidth = 5;
      context.stroke();
      context.globalAlpha = Math.min(1, alpha * 0.2 * glow);
      context.lineWidth = 2.75;
      context.stroke();
    }
    context.globalAlpha = Math.min(1, alpha);
    context.lineWidth = lineWidth;
    context.stroke();
  }

  /** Linear read of the live column array at a fractional column index. */
  private sampleColumns(column: number): number {
    const lower = Math.floor(column);
    const upper = Math.min(COLUMN_COUNT - 1, lower + 1);
    const a = this.columns[lower];
    return a + (this.columns[upper] - a) * (column - lower);
  }

  private resize(): void {
    const canvas = this.canvas;
    const context = this.context;
    if (!canvas || !context) {
      return;
    }
    const width = canvas.clientWidth;
    const height = canvas.clientHeight;
    const ratio =
      typeof window !== "undefined" && Number.isFinite(window.devicePixelRatio)
        ? Math.max(1, Math.min(3, window.devicePixelRatio))
        : 1;

    this.width = width;
    this.height = height;
    canvas.width = Math.max(1, Math.round(width * ratio));
    canvas.height = Math.max(1, Math.round(height * ratio));
    context.setTransform(ratio, 0, 0, ratio, 0, 0);
    this.rebuildGeometry();
  }

  /** Precompute everything about the ridge layout that does not change per frame. */
  private rebuildGeometry(): void {
    const rows = Math.min(MAX_ROWS, this.activeRows());
    const points = Math.min(MAX_POINTS, this.quality.points);
    this.geometryRows = rows;
    this.geometryPoints = points;
    if (this.width <= 0 || this.height <= 0) {
      return;
    }

    for (let point = 0; point < points; point += 1) {
      const progress = points === 1 ? 0.5 : point / (points - 1);
      this.pointProgress[point] = progress;
      this.pointColumn[point] = mirroredColumn(progress, COLUMN_COUNT);
      this.pointEnvelope[point] = ridgeEnvelope(progress);
    }

    for (let rowIndex = 0; rowIndex < rows; rowIndex += 1) {
      const depth = rows === 1 ? 1 : rowIndex / (rows - 1);
      this.rowBaseline[rowIndex] =
        this.height *
        (REAR_BASELINE + Math.pow(depth, DEPTH_CURVE) * (FRONT_BASELINE - REAR_BASELINE));
      this.rowScale[rowIndex] = this.height * (REAR_HEIGHT + depth * (FRONT_HEIGHT - REAR_HEIGHT));
      // Rear ridges are narrower, which gives the trapezoid perspective.
      const inset = this.width * (BASE_INSET + (1 - depth) * REAR_INSET_EXTRA);
      const span = this.width - inset * 2;
      for (let point = 0; point < points; point += 1) {
        this.pointX[rowIndex * MAX_POINTS + point] = inset + this.pointProgress[point] * span;
      }
    }

    // Created here, never per frame. Test canvases may not implement gradients.
    const context = this.context;
    this.underFill = null;
    this.spectrumStroke = null;
    if (context && typeof context.createLinearGradient === "function") {
      const gradient = context.createLinearGradient(0, this.height * 0.3, 0, this.height);
      gradient.addColorStop(0, "rgba(255, 255, 255, 0.14)");
      gradient.addColorStop(1, "rgba(255, 255, 255, 0)");
      this.underFill = gradient;

      const inset = this.width * BASE_INSET;
      const spectrum = context.createLinearGradient(inset, 0, this.width - inset, 0);
      for (const [position, colour] of SPECTRUM_STOPS) {
        spectrum.addColorStop(position, colour);
      }
      this.spectrumStroke = spectrum;
    }
  }

  /** Wire up resize, DPI-change, and visibility listeners; all removed on dispose. */
  private observeEnvironment(): void {
    this.detachEnvironment?.();
    const cleanups: Array<() => void> = [];

    if (typeof window !== "undefined") {
      const onResize = () => this.resize();
      window.addEventListener("resize", onResize);
      cleanups.push(() => window.removeEventListener("resize", onResize));

      if (typeof window.matchMedia === "function") {
        // Re-arm a resolution query each time the DPR changes (monitor moves,
        // Windows scaling changes) so the backing store never goes blurry.
        let dprQuery: MediaQueryList | null = null;
        const onDprChange = () => {
          this.resize();
          armDprQuery();
        };
        const armDprQuery = () => {
          dprQuery?.removeEventListener?.("change", onDprChange);
          dprQuery = window.matchMedia(`(resolution: ${window.devicePixelRatio}dppx)`);
          dprQuery.addEventListener?.("change", onDprChange);
        };
        armDprQuery();
        cleanups.push(() => dprQuery?.removeEventListener?.("change", onDprChange));
      }
    }

    if (typeof ResizeObserver !== "undefined" && this.canvas) {
      const observer = new ResizeObserver(() => this.resize());
      observer.observe(this.canvas);
      cleanups.push(() => observer.disconnect());
    }

    if (typeof document !== "undefined") {
      // The overlay window is hidden (not destroyed) between dictations; stop
      // animating entirely while it is not visible.
      const onVisibility = () => {
        if (document.visibilityState === "hidden") {
          this.wasRunningBeforeHide = this.running;
          this.stop();
        } else if (this.wasRunningBeforeHide) {
          this.lastFrameAt = null;
          this.start();
        }
      };
      document.addEventListener("visibilitychange", onVisibility);
      cleanups.push(() => document.removeEventListener("visibilitychange", onVisibility));
    }

    this.detachEnvironment = () => {
      for (const cleanup of cleanups) {
        cleanup();
      }
    };
  }
}

const OCCLUSION_STYLE = `rgba(0, 0, 0, ${OCCLUSION})`;
