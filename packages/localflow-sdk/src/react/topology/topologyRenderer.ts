import type { LocalFlowDictationPhase } from "../../types";
import {
  CircularHistory,
  FeatureSmoother,
  clampUnit,
  historyStepsForDepth,
  smoothingCoefficient,
} from "./audioFeatureSmoothing";
import {
  depthAlpha,
  dotRadius,
  projectPoint,
  surfaceHeight,
  type SmoothedFeatures,
  type SurfaceMotion,
} from "./topologyMath";
import { displacementForMotion, visualsForPhase, type PhaseVisuals } from "./overlayStateVisuals";

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
}

interface QualityProfile {
  rows: number;
  columnSpacing: number;
  minFrameIntervalMs: number;
}

const QUALITY_PROFILES: Record<TopologyQuality, QualityProfile> = {
  balanced: { rows: 11, columnSpacing: 8, minFrameIntervalMs: 0 },
  low: { rows: 7, columnSpacing: 12, minFrameIntervalMs: 32 },
};

const HISTORY_SIZE = 32;
const HISTORY_PUSH_INTERVAL_MS = 60;
const PHASE_BLEND_TAU_MS = 140;
const IDLE_PHASES: ReadonlySet<LocalFlowDictationPhase> = new Set([
  "idle",
  "inserted",
  "cancelled",
  "error",
]);

/**
 * Owns the animation loop and all mutable visualization state for the
 * topographic waveform. Contains no dictation business logic: phases come in
 * via `setPhase`, audio features via `setFeatureTargets` or a polled source.
 *
 * Rendering model: `rows` parallel depth rows of dots. Rear rows are higher,
 * narrower, dimmer and read older level history so speech peaks propagate
 * into the distance. One arc path + one fill per row per frame; no shadow
 * blur; the horizontal edge fade is a CSS mask on the canvas element.
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
  private readonly history = new CircularHistory(HISTORY_SIZE, 0.08);

  // Phase-derived parameters are themselves smoothed so state transitions
  // (success collapse, error tint, processing sweep) ease in rather than snap.
  private blend = {
    micDrive: 0,
    energyFloor: 0.1,
    sweep: 0,
    collapse: 0,
    accentStrength: 0,
    accentR: 235,
    accentG: 240,
    accentB: 245,
  };

  private detachEnvironment: (() => void) | null = null;

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
  }

  setPhase(phase: LocalFlowDictationPhase): void {
    if (phase === this.phase) {
      return;
    }
    // A fresh session must never inherit the previous session's terrain energy.
    if ((phase === "listening" || phase === "ready") && IDLE_PHASES.has(this.phase)) {
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
  }

  setReducedMotion(reducedMotion: boolean): void {
    this.reducedMotion = reducedMotion;
    this.phaseVisuals = visualsForPhase(this.phase, reducedMotion);
  }

  /** Clear smoothed levels and history (new session, or shown after hidden). */
  resetDynamics(): void {
    this.level.reset(0.08);
    this.pitch.reset(0.5);
    this.brightness.reset(0.35);
    this.history.fill(0.08);
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

    this.historyAccumulatorMs += dtMs;
    while (this.historyAccumulatorMs >= HISTORY_PUSH_INTERVAL_MS) {
      this.historyAccumulatorMs -= HISTORY_PUSH_INTERVAL_MS;
      this.history.push(this.level.value);
    }

    const k = smoothingCoefficient(dtMs, PHASE_BLEND_TAU_MS);
    const blend = this.blend;
    blend.micDrive += (visuals.micDrive - blend.micDrive) * k;
    blend.energyFloor += (visuals.energyFloor - blend.energyFloor) * k;
    blend.sweep += (visuals.sweep - blend.sweep) * k;
    blend.collapse += (visuals.collapse - blend.collapse) * k;
    blend.accentStrength += (visuals.accentStrength - blend.accentStrength) * k;
    blend.accentR += (visuals.accent[0] - blend.accentR) * k;
    blend.accentG += (visuals.accent[1] - blend.accentG) * k;
    blend.accentB += (visuals.accent[2] - blend.accentB) * k;

    if (visuals.sweepSpeed > 0) {
      this.sweepCycle = (this.sweepCycle + (dtMs / 1000) * visuals.sweepSpeed) % 1;
    }
  }

  private draw(): void {
    const context = this.context;
    if (!context || this.width <= 0 || this.height <= 0) {
      return;
    }

    context.clearRect(0, 0, this.width, this.height);

    const viewport = { width: this.width, height: this.height };
    const { rows, columnSpacing } = this.quality;
    const columns = Math.max(24, Math.round(this.width / columnSpacing));
    const displacement = displacementForMotion(this.reducedMotion);
    const blend = this.blend;
    const brightnessBoost = this.phaseVisuals.brightnessBoost;
    // Sweep travels slightly past both edges so it enters and exits smoothly.
    const sweepPosition = this.sweepCycle * 1.3 - 0.15;

    const motion: SurfaceMotion = {
      displacement,
      travel: 1,
      sweep: blend.sweep,
      sweepPosition,
      collapse: blend.collapse,
    };

    for (let rowIndex = 0; rowIndex < rows; rowIndex += 1) {
      const depth = rows === 1 ? 1 : rowIndex / (rows - 1);
      const rowLevel = this.history.at(historyStepsForDepth(depth, HISTORY_SIZE));
      const drive = clampUnit(blend.energyFloor + blend.micDrive * rowLevel, 0.1);
      const features: SmoothedFeatures = {
        level: drive,
        pitch: this.pitch.value,
        brightness: this.brightness.value,
      };
      const activity = drive + brightnessBoost * this.brightness.value;
      const alpha = depthAlpha(depth, activity);
      const mix = blend.accentStrength * (0.25 + 0.5 * depth);
      const r = Math.round(235 + (blend.accentR - 235) * mix);
      const g = Math.round(240 + (blend.accentG - 240) * mix);
      const b = Math.round(245 + (blend.accentB - 245) * mix);
      const baseRadius = dotRadius(depth, activity);
      const isFrontRow = rowIndex === rows - 1;

      context.beginPath();
      let firstX = 0;
      let firstY = 0;
      for (let columnIndex = 0; columnIndex < columns; columnIndex += 1) {
        const progress = columns === 1 ? 0.5 : columnIndex / (columns - 1);
        const height = surfaceHeight(progress, depth, this.timeSec, features, motion);
        const point = projectPoint(progress, depth, height, viewport);
        const radius = baseRadius * (0.75 + 0.25 * Math.min(1, Math.abs(height) * 1.4));
        context.moveTo(point.x + radius, point.y);
        context.arc(point.x, point.y, radius, 0, Math.PI * 2);
        if (columnIndex === 0) {
          firstX = point.x;
          firstY = point.y;
        }
      }
      context.fillStyle = `rgba(${r}, ${g}, ${b}, ${alpha.toFixed(3)})`;
      context.fill();

      if (isFrontRow) {
        // A faint connecting stroke anchors the foreground ridge (and becomes
        // the calm resting line as the terrain collapses on success/cancel).
        context.beginPath();
        context.moveTo(firstX, firstY);
        for (let columnIndex = 1; columnIndex < columns; columnIndex += 1) {
          const progress = columnIndex / (columns - 1);
          const height = surfaceHeight(progress, depth, this.timeSec, features, motion);
          const point = projectPoint(progress, depth, height, viewport);
          context.lineTo(point.x, point.y);
        }
        context.strokeStyle = `rgba(${r}, ${g}, ${b}, ${(0.1 + drive * 0.22).toFixed(3)})`;
        context.lineWidth = 1;
        context.stroke();
      }
    }
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

  private wasRunningBeforeHide = false;
}
