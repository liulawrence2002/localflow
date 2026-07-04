import type { DiagnosticMetric } from "./types";

export type PerformanceMetricKey =
  | "hotkey_to_recording_start"
  | "asr_partial_latency"
  | "release_to_final_transcript"
  | "llm_refinement_latency"
  | "text_insertion_latency"
  | "model_load_time"
  | "peak_memory";

export type PerformanceMarkKey =
  | "hotkey_pressed"
  | "recording_started"
  | "first_asr_partial"
  | "hotkey_released"
  | "final_transcript_ready"
  | "llm_refinement_started"
  | "llm_refinement_finished"
  | "insertion_started"
  | "insertion_finished"
  | "model_load_started"
  | "model_load_finished";

export interface PerformanceMeasurement {
  key: PerformanceMetricKey;
  value: number;
  unit: "ms" | "bytes";
}

export interface PerformanceSnapshot {
  measurements: PerformanceMeasurement[];
  diagnostics: DiagnosticMetric[];
}

export type PerformanceClock = () => number;

const metricLabels: Record<PerformanceMetricKey, string> = {
  hotkey_to_recording_start: "Hotkey to recording start",
  asr_partial_latency: "ASR partial latency",
  release_to_final_transcript: "Release to final transcript",
  llm_refinement_latency: "LLM refinement latency",
  text_insertion_latency: "Text insertion latency",
  model_load_time: "Model load time",
  peak_memory: "Peak memory",
};

const allMetricKeys = Object.keys(metricLabels) as PerformanceMetricKey[];

export class PerformanceRecorder {
  private readonly clock: PerformanceClock;
  private readonly marks = new Map<PerformanceMarkKey, number>();
  private readonly measurements = new Map<PerformanceMetricKey, PerformanceMeasurement>();

  constructor(clock: PerformanceClock = () => performanceNow()) {
    this.clock = clock;
  }

  mark(key: PerformanceMarkKey): void {
    this.marks.set(key, this.clock());
  }

  recordDuration(key: PerformanceMetricKey, durationMs: number): void {
    if (!Number.isFinite(durationMs) || durationMs < 0) {
      throw new Error("Performance duration must be a non-negative finite number.");
    }

    this.measurements.set(key, {
      key,
      value: durationMs,
      unit: "ms",
    });
  }

  recordPeakMemory(bytes: number): void {
    if (!Number.isFinite(bytes) || bytes < 0) {
      throw new Error("Peak memory must be a non-negative finite number.");
    }

    this.measurements.set("peak_memory", {
      key: "peak_memory",
      value: bytes,
      unit: "bytes",
    });
  }

  measureDuration(
    key: PerformanceMetricKey,
    startMark: PerformanceMarkKey,
    endMark: PerformanceMarkKey,
  ): void {
    const start = this.marks.get(startMark);
    const end = this.marks.get(endMark);

    if (start === undefined || end === undefined) {
      throw new Error(`Cannot measure ${metricLabels[key]} without both marks.`);
    }

    this.recordDuration(key, end - start);
  }

  deriveKnownDurations(): void {
    this.measureIfMarked("hotkey_to_recording_start", "hotkey_pressed", "recording_started");
    this.measureIfMarked("asr_partial_latency", "recording_started", "first_asr_partial");
    this.measureIfMarked(
      "release_to_final_transcript",
      "hotkey_released",
      "final_transcript_ready",
    );
    this.measureIfMarked(
      "llm_refinement_latency",
      "llm_refinement_started",
      "llm_refinement_finished",
    );
    this.measureIfMarked("text_insertion_latency", "insertion_started", "insertion_finished");
    this.measureIfMarked("model_load_time", "model_load_started", "model_load_finished");
  }

  snapshot(): PerformanceSnapshot {
    const measurements = allMetricKeys
      .map((key) => this.measurements.get(key))
      .filter((measurement): measurement is PerformanceMeasurement => Boolean(measurement));

    return {
      measurements,
      diagnostics: performanceDiagnostics(measurements),
    };
  }

  private measureIfMarked(
    key: PerformanceMetricKey,
    startMark: PerformanceMarkKey,
    endMark: PerformanceMarkKey,
  ): void {
    if (this.measurements.has(key)) {
      return;
    }

    const start = this.marks.get(startMark);
    const end = this.marks.get(endMark);
    if (start === undefined || end === undefined) {
      return;
    }

    this.recordDuration(key, end - start);
  }
}

export function performanceDiagnostics(measurements: PerformanceMeasurement[]): DiagnosticMetric[] {
  const byKey = new Map(measurements.map((measurement) => [measurement.key, measurement]));

  return allMetricKeys.map((key) => {
    const measurement = byKey.get(key);
    return {
      label: metricLabels[key],
      value: measurement ? formatMeasurement(measurement) : "Not measured",
      status: measurement ? "ok" : "warning",
    };
  });
}

export function formatMeasurement(measurement: PerformanceMeasurement): string {
  if (measurement.unit === "bytes") {
    return formatBytes(measurement.value);
  }

  return `${Math.round(measurement.value)} ms`;
}

function formatBytes(bytes: number): string {
  if (bytes < 1024) {
    return `${Math.round(bytes)} B`;
  }

  const mib = bytes / 1024 ** 2;
  if (mib < 1024) {
    return `${mib.toFixed(mib >= 10 ? 0 : 1)} MiB`;
  }

  const gib = mib / 1024;
  return `${gib.toFixed(gib >= 10 ? 0 : 1)} GiB`;
}

function performanceNow(): number {
  return typeof performance !== "undefined" ? performance.now() : Date.now();
}
